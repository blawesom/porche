//! Route handlers + askama pages — the MS-1 object console (E-28 projection
//! rule, E-29 live re-entry). Every mutation maps to one doc-09 method name —
//! `CreateWorkload`, `StopWorkload`, `DeleteWorkload`, `DeleteVolume` — and
//! writes exactly one audit entry (E-26), success or rejection. The admin
//! surface is read-only (no mutations land there in MS-1). There is no dry
//! mode anywhere (E-29).

use crate::audit::epoch_utc;
use crate::state::*;
use crate::AppState;
use askama::Template;
use axum::extract::{Extension, Form, Path, State};
use axum::http::{header, HeaderMap, Request, StatusCode};
use axum::middleware::Next;
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::Router;
use porche_core::{ops, DesiredStateStore, Record, VolumeSpec, WorkloadSpec, WorkloadState};
use std::sync::Arc;

pub fn router(state: Arc<AppState>) -> Router {
    let root = move |State(app): State<Arc<AppState>>| async move {
        Redirect::to(&format!("{}/w", app.base_path))
    };
    let r = Router::new()
        .route("/", get(root))
        .route("/w", get(workloads))
        .route("/w/new", get(create_form).post(create_submit))
        .route("/w/volumes", get(volumes))
        .route("/w/volumes/{id}", get(volume_detail))
        .route("/w/volumes/{id}/delete", post(delete_volume))
        .route("/w/{id}", get(workload_detail))
        .route("/w/{id}/console", get(console_tail))
        .route("/a", get(fleet))
        .route("/a/host", get(host_detail))
        .route("/htmx.js", get(htmx_js))
        .route("/w/images", get(images));
    let r = r.layer(axum::middleware::from_fn_with_state(
        state.clone(),
        auth_mw,
    ));
    // E-30: behind a reverse proxy the whole tree nests under the prefix;
    // `nest` strips it before route matching (a middleware URI rewrite
    // cannot — axum matches on the original request path). Direct mode
    // (empty prefix) is an un-nested router.
    if state.base_path.is_empty() {
        r.with_state(state)
    } else {
        let bp = state.base_path.clone();
        Router::new().nest(bp.as_str(), r.with_state(state))
    }
}

// --- auth: two opaque-token domains (E-8 / E-29) --------------------------------

const COOKIE: &str = "porche_token";

/// The two strict AuthN domains (E-8). `/w` and children accept ONLY the user
/// token; `/a` and children accept ONLY the admin token; cross-domain access
/// is a 401 (UAT negative check `n1`). A surface with no configured token is
/// closed (401) — strict by default, never implicitly open.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Domain {
    User,
    Admin,
}

impl Domain {
    fn of_path(path: &str) -> Option<Domain> {
        if path == "/a" || path.starts_with("/a/") {
            Some(Domain::Admin)
        } else if path == "/w" || path.starts_with("/w/") {
            Some(Domain::User)
        } else {
            None
        }
    }
}

/// The auth seen on a request — one bit per domain token (an Authorization
/// header or cookie is read at most once, so one value can never satisfy both
/// letters; `?token=` sets the cookie under exactly the letter it matched).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Auth {
    user: bool,
    admin: bool,
}

fn bearer_matches(req: &Request<axum::body::Body>, want: &str) -> bool {
    req.headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .is_some_and(|v| v == want)
}

fn cookie_matches(req: &Request<axum::body::Body>, want: &str) -> bool {
    req.headers()
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|c| {
            c.split(';')
                .any(|p| p.trim() == format!("{COOKIE}={want}"))
        })
}

fn query_token(req: &Request<axum::body::Body>) -> Option<String> {
    req.uri().query().and_then(|q| {
        q.split('&')
            .find(|p| p.starts_with("token="))
            .map(|p| p.strip_prefix("token=").unwrap_or_default().to_string())
    })
}

fn request_auth(req: &Request<axum::body::Body>, user: Option<&str>, admin: Option<&str>) -> Auth {
    let matched = |want: Option<&str>| want.is_some_and(|w| bearer_matches(req, w) || cookie_matches(req, w));
    Auth {
        user: matched(user),
        admin: matched(admin),
    }
}

/// The authenticated tenant on a user-surface request (MS-2, E-31.2): the
/// token that matched the user domain maps to exactly one tenant scope.
/// `t0` is the legacy single-tenant scope.
#[derive(Clone, Debug)]
pub struct TenantCtx {
    pub tenant_id: String,
}

async fn auth_mw(
    State(app): State<Arc<AppState>>,
    mut req: Request<axum::body::Body>,
    next: Next,
) -> Response {
    // E-30: the prefix is handled by `Router::nest` in `router()` (it strips
    // the prefix before route matching — a middleware URI rewrite cannot,
    // axum matches on the original request path). This layer keeps the two
    // strict token domains and re-joins redirect `Location`s under the
    // prefix for the browser.
    let bp = app.base_path.clone();
    let path = req.uri().path().to_string();
    let Some(domain) = Domain::of_path(&path) else {
        // static asset: no auth
        let res = next.run(req).await;
        return rejoin_prefix(res, &bp);
    };

    let user_want = app.user_token.as_deref();
    let admin_want = app.admin_token.as_deref();

    // MS-2: a user-domain request authenticates by ANY tenant token (or the
    // legacy user token = t0). The matched token fixes the tenant scope.
    let tenant_of_token = |tok: &str| -> Option<String> {
        app.tenants
            .iter()
            .find(|(_, t)| t == tok)
            .map(|(tid, _)| tid.clone())
    };

    // one-time `?token=` → set the cookie, 303 to the clean path (E-28 kept
    // mechanism). A query token of the WRONG domain matches nothing and falls
    // through to the strict 401 — a user token never opens `/a` and vice
    // versa (the split is by configured value; the UAT tokens are distinct).
    if let Some(qt) = query_token(&req) {
        let is_user = user_want == Some(qt.as_str())
            || (domain == Domain::User && tenant_of_token(&qt).is_some());
        let is_admin = admin_want == Some(qt.as_str());
        let ok = match domain {
            Domain::User => is_user,
            Domain::Admin => is_admin,
        };
        if ok {
            return set_cookie_and_redirect(&qt, &path, &bp);
        }
    }

    let auth = request_auth(&req, user_want, admin_want);
    let tenant_scope: Option<Option<String>> = match domain {
        Domain::User => {
            // resolve the tenant from whichever credential matched
            let cred = extract_credential(&req);
            let scope = cred.as_deref().and_then(|c| {
                if user_want == Some(c) {
                    Some("t0".to_string()) // legacy single-tenant token
                } else {
                    tenant_of_token(c)
                }
            });
            Some(scope)
        }
        Domain::Admin => None,
    };
    let ok = match domain {
        Domain::User => auth.user || tenant_scope.as_ref().is_some_and(|s| s.is_some()),
        Domain::Admin => auth.admin,
    };
    if ok {
        if let Some(Some(tid)) = tenant_scope {
            req.extensions_mut().insert(TenantCtx { tenant_id: tid });
        }
        let res = next.run(req).await;
        return rejoin_prefix(res, &bp);
    }

    // strict 401 on any unauthenticated/wrong-token surface (E-29 contract,
    // UAT `n1`): a 401 HTML page — the surface's login form — never a
    // workaround redirect and never the page behind a "login" landing. The
    // POST/other-method path gets the empty 401 body only.
    if req.method() == axum::http::Method::GET || req.method() == axum::http::Method::HEAD {
        render_auth401(domain, &bp)
    } else {
        (
            StatusCode::UNAUTHORIZED,
            "porche-web: 401 — valid token for this surface required\n",
        )
            .into_response()
    }
}

/// The presented credential (Bearer token, else the cookie value) — used to
/// resolve the tenant scope. Returns None when no credential is present.
fn extract_credential(req: &Request<axum::body::Body>) -> Option<String> {
    if let Some(v) = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
    {
        return Some(v.to_string());
    }
    req.headers()
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|c| {
            c.split(';').find_map(|p| {
                p.trim().strip_prefix("porche_token=").map(|v| v.to_string())
            })
        })
}

/// Set the cookie and 303 to the clean path. One cookie name, one domain's
/// value at a time: `?token=` only ever reaches here under the letter the
/// path configured it for. `base_path` (E-30) scopes the cookie and re-joins
/// the redirect target under the reverse-proxy prefix.
fn set_cookie_and_redirect(token: &str, path: &str, base_path: &str) -> Response {
    (
        StatusCode::SEE_OTHER,
        [
            (
                header::SET_COOKIE,
                format!("{COOKIE}={token}; Path={}/; Max-Age=86400; HttpOnly; SameSite=Lax", 
                    if base_path.is_empty() { "" } else { base_path }),
            ),
            (header::LOCATION, format!("{base_path}{path}")),
        ],
    )
        .into_response()
}

/// Re-join a response's `Location` header under the reverse-proxy prefix
/// (E-30): handlers emit clean paths (`/w/…`, `/a`); behind the proxy the
/// browser must land on `{base_path}/w/…`.
fn rejoin_prefix(mut res: Response, base_path: &str) -> Response {
    if base_path.is_empty() {
        return res;
    }
    if let Some(loc) = res
        .headers()
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
    {
        if loc.starts_with('/') && !loc.starts_with(&format!("{base_path}/")) {
            let new = format!("{base_path}{loc}");
            if let Ok(hv) = axum::http::HeaderValue::from_str(&new) {
                res.headers_mut().insert(header::LOCATION, hv);
            }
        }
    }
    res
}

#[derive(Template)]
#[template(path = "auth401.html")]
struct Auth401Page {
    surface: &'static str,
    surface_home: String,
}

/// The 401 HTML page (the surface's login form) — rendered straight from the
/// middleware on every unauthenticated request; the login page IS the 401
/// page, there is no separate login surface to redirect to.
fn render_auth401(domain: Domain, base_path: &str) -> Response {
    let (surface, home) = match domain {
        Domain::Admin => ("admin", "/a"),
        Domain::User => ("user", "/w"),
    };
    (
        StatusCode::UNAUTHORIZED,
        Html(
            Auth401Page {
                surface,
                surface_home: format!("{base_path}{home}"),
            }
            .render()
            .unwrap(),
        ),
    )
        .into_response()
}

async fn htmx_js() -> Response {
    (
        [(header::CONTENT_TYPE, "application/javascript; charset=utf-8")],
        crate::HTMX_JS,
    )
        .into_response()
}

/// Mutating calls carry `x-porche-actor` (set by the form); reads say
/// `user-token` (PoC-grade: tokens are opaque, one actor per surface token).
fn actor_of(headers: &HeaderMap) -> String {
    headers
        .get("x-porche-actor")
        .and_then(|v| v.to_str().ok())
        .filter(|v| !v.is_empty() && v.len() <= 64)
        .map(|v| v.to_string())
        .unwrap_or_else(|| "user-token".into())
}

fn audit(app: &Arc<AppState>, actor: &str, action: &str, object: &str, result: &str) {
    app.audit
        .lock()
        .unwrap()
        .record(actor, action, object, result);
}

// --- view models ---------------------------------------------------------------

struct WorkloadRow {
    id: String,
    state_css: String,
    state_label: String,
    vcpu: u32,
    ram_mib: u32,
    ip6: String,
    volume_label: String,
    volume_link: bool,
    created: String,
    can_stop: bool,
    can_delete: bool,
}
/// Image option row for the create form: dir name (the digest — the value),
/// a friendly short id for display, and manifest metadata lines when present.
struct ImageOpt {
    name: String,
    /// short display id, e.g. `sha256:962913…` (first 6 hash chars)
    short: String,
    manifest: String,
    selected: bool,
}

fn manifest_lines(dir: &std::path::Path) -> String {
    std::fs::read_to_string(dir.join("manifest.txt"))
        .map(|s| s.lines().collect::<Vec<_>>().join(" · "))
        .unwrap_or_default()
}

/// `sha256:<64 hex>` → `sha256:<first 10 hex>…`
fn short_digest(name: &str) -> String {
    if let Some(h) = name.strip_prefix("sha256:") {
        if h.len() > 12 {
            return format!("sha256:{}…", &h[..10]);
        }
    }
    name.to_string()
}

fn image_options(app: &AppState, selected_for: &str) -> Vec<ImageOpt> {
    let mut names: Vec<String> = std::fs::read_dir(app.data_dir.join("images"))
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
        .into_iter()
        .map(|name| {
            let manifest = manifest_lines(&app.data_dir.join("images").join(&name));
            let selected = name == selected_for;
            let short = short_digest(&name);
            ImageOpt {
                name,
                short,
                manifest,
                selected,
            }
        })
        .collect()
}

fn row_of(app: &AppState, r: &Record) -> WorkloadRow {
    let console = read_console(&app.data_dir, &r.spec.id);
    let signals = console_signals(&console);
    let socket = vm_socket_present(&app.data_dir, &r.spec.id);
    let d = derive_state(&r.state, &signals, socket);
    WorkloadRow {
        id: r.spec.id.clone(),
        state_css: d.css().into(),
        state_label: d.label(),
        vcpu: r.spec.vcpu,
        ram_mib: r.spec.ram_mib,
        ip6: r.spec.ip6.clone(),
        volume_label: r
            .spec
            .volume
            .as_ref()
            .map(|v| format!("{}M", v.size_mib))
            .unwrap_or_else(|| "—".into()),
        volume_link: r.spec.volume.is_some(),
        created: epoch_utc(r.created_at),
        can_stop: matches!(r.state, WorkloadState::Running),
        can_delete: !matches!(r.state, WorkloadState::Failed { .. }),
    }
}

// --- user surface: /w ----------------------------------------------------------

#[derive(Template)]
#[template(path = "workloads.html")]
struct WorkloadsPage {
    version: &'static str,
    base: String,
    records: Vec<WorkloadRow>,
}

/// Tenant context extractor for handlers — the user surface is ALWAYS
/// tenant-scoped (MS-2 view isolation). A missing context (route reached
/// without the middleware — a bug) fails closed as t0 rather than open.
fn tenant_ctx(req: &axum::http::Request<axum::body::Body>) -> TenantCtx {
    req.extensions()
        .get::<TenantCtx>()
        .cloned()
        .unwrap_or(TenantCtx {
            tenant_id: "t0".into(),
        })
}
async fn workloads(
    State(app): State<Arc<AppState>>,
    Extension(t): Extension<TenantCtx>,
) -> Html<String> {
let t = t;
    let records = app.store.list_for_tenant(&t.tenant_id).unwrap_or_default();
    let rows = records.iter().map(|r| row_of(&app, r)).collect();
    WorkloadsPage {
        version: env!("CARGO_PKG_VERSION"), base: app.base_path.clone(),
        records: rows,
    }
    .render()
    .unwrap_or_else(|e| panic!("template: {e}"))
    .into()
}

#[derive(Default, Clone)]
struct FormVals {
    id: String,
    command: String,
    vcpu: String,
    ram_mib: String,
    volume_mib: String,
    ip6: String,
    gw6: String,
}

#[derive(Template)]
#[template(path = "workload_form.html")]
struct WorkloadFormPage {
    version: &'static str,
    base: String,
    error: Option<String>,
    form: FormVals,
    images: Vec<ImageOpt>,
}

async fn create_form(State(app): State<Arc<AppState>>) -> Html<String> {
    WorkloadFormPage {
        version: env!("CARGO_PKG_VERSION"), base: app.base_path.clone(),
        error: None,
        form: FormVals {
            vcpu: "1".into(),
            ram_mib: "128".into(),
            volume_mib: "256".into(),
            gw6: "fd42:1:1::ff".into(),
            ..Default::default()
        },
        images: image_options(&app, ""),
    }
    .render()
    .unwrap()
    .into()
}

#[derive(serde::Deserialize, Debug)]
struct CreateForm {
    id: String,
    image_digest: String,
    command: String,
    vcpu: String,
    ram_mib: String,
    volume_mib: String,
    ip6: String,
    gw6: String,
}

async fn create_submit(
    State(app): State<Arc<AppState>>,
    headers: HeaderMap,
    Extension(t): Extension<TenantCtx>,
    Form(form): Form<CreateForm>,
) -> Response {
    let actor = actor_of(&headers);
let t = t;

    let vals = FormVals {
        id: form.id.clone(),
        command: form.command.clone(),
        vcpu: form.vcpu.clone(),
        ram_mib: form.ram_mib.clone(),
        volume_mib: form.volume_mib.clone(),
        ip6: form.ip6.clone(),
        gw6: form.gw6.clone(),
    };

    // reject duplicate ids up front — the store's upsert would replace a row
    // (duplicate check is tenant-global: workload ids are host-global device
    // names — two tenants cannot both claim netns w-foo)
    let pre = match app.store.get(form.id.trim()) {
        Ok(Some(_)) => Some(format!("workload {:?} already exists", form.id.trim())),
        Ok(None) => None,
        Err(e) => Some(format!("store: {e}")),
    };

    let spec = pre.map(Err).unwrap_or_else(|| build_spec(&form, &t.tenant_id));
    match spec {
        Ok(spec) => {
            // validate BEFORE the synchronous pre-persist: an invalid spec must
            // re-render the form with the reason (like the duplicate-id check),
            // never leave a row stuck in `Creating` awaiting a task that will
            // fail at `ops::create`'s own validate() gate (observed via the UAT
            // smoke's over-long id, 2026-10-20).

            if let Err(e) = spec.validate() {
                audit(&app, &actor, "CreateWorkload", &form.id, &format!("rejected: {e}"));
                return Html(WorkloadFormPage {
                    version: env!("CARGO_PKG_VERSION"),
                    base: app.base_path.clone(),
                    error: Some(e),
                    form: vals,
                    images: image_options(&app, &form.image_digest),
                }
                .render()
                .unwrap())
                .into_response();
            }
            // MS-1 u03: the row must be observably `pending` (workspace state
            // Creating) before convergence runs — CreateWorkload returns
            // immediately; the real executor converges in the background and
            // the detail page's poll observes pending → running → done (rc=N).
            use porche_core::DesiredStateStore;
            if let Err(e) = app
                .store
                .upsert(&spec, &porche_core::WorkloadState::Creating)
                .map_err(|e| format!("store: {e}"))
            {
                audit(&app, &actor, "CreateWorkload", &form.id, &format!("rejected: {e}"));
                return Html(WorkloadFormPage {
                    version: env!("CARGO_PKG_VERSION"),
                    base: app.base_path.clone(),
                    error: Some(e),
                    form: vals,
                    images: image_options(&app, &form.image_digest),
                }
                .render()
                .unwrap())
                .into_response();
            }
            let app2 = app.clone();
            let id = spec.id.clone();
            let actor_bg = actor.clone();
            tokio::task::spawn_blocking(move || {
                let mut ex = app2.exec.lock().unwrap();
                let r = ops::create(&app2.store, &mut *ex, spec);
                let outcome = match r {
                    Ok(()) => "ok (converged)".to_string(),
                    Err(e) => format!("failed: {e}"),
                };
                app2
                    .audit
                    .lock()
                    .unwrap()
                    .record(&actor_bg, "CreateWorkload", &id, &outcome);
            });
            audit(&app, &actor, "CreateWorkload", &form.id, "ordered");
            Redirect::to(&format!("/w/{}", form.id)).into_response()
        }
        Err(e) => {
            audit(&app, &actor, "CreateWorkload", &form.id, &format!("rejected: {e}"));
            Html(WorkloadFormPage {
                version: env!("CARGO_PKG_VERSION"), base: app.base_path.clone(),
                error: Some(e.to_string()),
                form: vals,
                images: image_options(&app, &form.image_digest),
            }
            .render()
            .unwrap())
            .into_response()
        }
    }
}

fn build_spec(form: &CreateForm, tenant_id: &str) -> Result<WorkloadSpec, String> {
    let vcpu: u32 = form
        .vcpu
        .trim()
        .parse()
        .map_err(|_| format!("vcpu: not a number: {:?}", form.vcpu))?;
    let ram_mib: u32 = form
        .ram_mib
        .trim()
        .parse()
        .map_err(|_| format!("ram_mib: not a number: {:?}", form.ram_mib))?;
    let volume = if form.volume_mib.trim().is_empty() {
        None
    } else {
        Some(VolumeSpec {
            size_mib: form
                .volume_mib
                .trim()
                .parse()
                .map_err(|_| format!("volume_mib: not a number: {:?}", form.volume_mib))?,
        })
    };
    Ok(WorkloadSpec {
        tenant_id: tenant_id.to_string(),
        id: form.id.trim().to_string(),
        image_digest: form.image_digest.trim().to_string(),
        command: form.command.clone(),
        vcpu,
        ram_mib,
        volume,
        ip6: form.ip6.trim().to_string(),
        gw6: form.gw6.trim().to_string(),
    })
}

// --- workload detail: spec card + console + events + relations + topology -------

#[derive(Template)]
#[template(path = "workload_detail.html")]
struct WorkloadDetailPage {
    version: &'static str,
    base: String,
    row: WorkloadRow,
    command: String,
    image_digest: String,
    failed_reason: Option<String>,
    created: String,
    updated: String,
    events: Vec<EventLine>,
    /// host → workload → volume slot labels for the topology figure
    host_label: String,
    volume_slot: String,
    /// u03 "placement reason visible": why this workload sits where it does
    placement_reason: String,
}

async fn workload_detail(
    State(app): State<Arc<AppState>>,
    Path(id): Path<String>,
    Extension(t): Extension<TenantCtx>,
) -> Response {
let t = t;
    match app.store.get_for_tenant(&t.tenant_id, &id) {
        Ok(Some(r)) => {
            let console = read_console(&app.data_dir, &id);
            let log_mtime = std::fs::metadata(console_path(&app.data_dir, &id))
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64);
            let events = derive_events(r.created_at, r.updated_at, &console, log_mtime);
            let signals = console_signals(&console);
            let socket = vm_socket_present(&app.data_dir, &id);
            let d = derive_state(&r.state, &signals, socket);
            Html(WorkloadDetailPage {
                version: env!("CARGO_PKG_VERSION"), base: app.base_path.clone(),
                row: row_of(&app, &r),
                command: r.spec.command,
                image_digest: r.spec.image_digest,
                failed_reason: match &d {
                    DisplayState::Failed(reason) => Some(reason.clone()),
                    _ => None,
                },
                created: epoch_utc(r.created_at),
                updated: epoch_utc(r.updated_at),
                events,
                host_label: hostname(),
                volume_slot: r
                    .spec
                    .volume
                    .as_ref()
                    .map(|v| format!("{}M tier-E", v.size_mib))
                    .unwrap_or_else(|| "none".into()),
                // u03 "placement reason visible" — single-host PoC placement: this
                // host is the only Fleet member (F0 single-node scope) and the desired
                // run dir/namespace paths are per-host derived (doc 14 §2).
                placement_reason: "single-host PoC placement — this host is the only Fleet member (F0)".into(),
            }
            .render()
            .unwrap())
            .into_response()
        }
        _ => not_found_page(&app, &format!("workload not found: {id}"), "/w").into_response(),
    }
}

#[derive(Template)]
#[template(path = "console_tail.html")]
struct ConsoleTail {
    lines: Vec<String>,
}

async fn console_tail(
    State(app): State<Arc<AppState>>,
    Path(id): Path<String>,
    Extension(t): Extension<TenantCtx>,
) -> Html<String> {
    // GetConsole — the recorded serial log tail (MS-1: recorded half).
    // Tenant-scoped: a cross-tenant id returns an empty tail, never another
    // tenant's serial log.
let t = t;
    let text = if app.store.get_for_tenant(&t.tenant_id, &id).ok().flatten().is_some() {
        read_console(&app.data_dir, &id)
    } else {
        String::new()
    };
    let mut lines: Vec<String> = text.lines().rev().take(40).map(String::from).collect();
    lines.reverse();
    ConsoleTail { lines }.render().unwrap().into()
}

async fn stop_workload(
    State(app): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Extension(t): Extension<TenantCtx>,
) -> Response {
    let actor = actor_of(&headers);
    // tenant gate BEFORE the stop: a cross-tenant stop must be a 404
let t = t;
    if app.store.get_for_tenant(&t.tenant_id, &id).ok().flatten().is_none() {
        return not_found_page(&app, &format!("workload not found: {id}"), "/w").into_response();
    }
    let id2 = id.clone();
    let app2 = app.clone();
    let result = tokio::task::spawn_blocking(move || {
        let mut ex = app2.exec.lock().unwrap();
        ops::stop(&app2.store, &mut *ex, &id2)
    })
    .await
    .unwrap_or_else(|e| Err(ops::OpsError::InvalidSpec(format!("join: {e}"))));

    match result {
        Ok(()) => {
            audit(&app, &actor, "StopWorkload", &id, "ok");
            Redirect::to(&format!("/w/{id}")).into_response()
        }
        Err(e) => {
            audit(&app, &actor, "StopWorkload", &id, &format!("rejected: {e}"));
            error_page(&app, &format!("stop {id}: {e}"), &format!("/w/{id}")).into_response()
        }
    }
}

#[derive(Template)]
#[template(path = "wipe_report.html")]
struct WipeReportPage {
    version: &'static str,
    base: String,
    method: &'static str,
    id: String,
    report_json: String,
    mapper_absent: bool,
    keyfile_entry_removed: bool,
    keyslots_erased: bool,
    region_trimmed: bool,
    region_clean: bool,
    next_href: &'static str,
    next_label: &'static str,
}

fn wipe_report_page(
    base_path: &str,
    method: &'static str,
    id: &str,
    report: &porche_core::WipeReport,
    next_href: &'static str,
    next_label: &'static str,
) -> WipeReportPage {
    WipeReportPage {
        version: env!("CARGO_PKG_VERSION"), base: base_path.to_string(),
        method,
        id: id.to_string(),
        report_json: serde_json::to_string_pretty(report).unwrap_or_else(|_| "{}".into()),
        mapper_absent: report.mapper_absent,
        keyfile_entry_removed: report.keyfile_entry_removed,
        keyslots_erased: report.keyslots_erased,
        region_trimmed: report.region_trimmed,
        region_clean: report.region_clean,
        next_href,
        next_label,
    }
}

async fn delete_workload(
    State(app): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Extension(t): Extension<TenantCtx>,
) -> Response {
    // DeleteWorkload is an operation (doc 09 conventions): teardown with the
    // reported wipe — the report is the page.
    let actor = actor_of(&headers);
    // tenant gate BEFORE the wipe: a cross-tenant delete must be a 404, never
    // a wipe of another tenant's workload
let t = t;
    if app.store.get_for_tenant(&t.tenant_id, &id).ok().flatten().is_none() {
        return not_found_page(&app, &format!("workload not found: {id}"), "/w").into_response();
    }
    let id2 = id.clone();
    let app2 = app.clone();
    let result = tokio::task::spawn_blocking(move || {
        let mut ex = app2.exec.lock().unwrap();
        ops::wipe(&app2.store, &mut *ex, &id2, &porche_core::Markers::new())
    })
    .await
    .unwrap_or_else(|e| Err(ops::OpsError::InvalidSpec(format!("join: {e}"))));

    match result {
        Ok(report) => {
            audit(&app, &actor, "DeleteWorkload", &id, "ok (wipe report rendered)");
            Html(wipe_report_page(&app.base_path, "DeleteWorkload", &id, &report, "/w", "workloads").render().unwrap())
                .into_response()
        }
        Err(e) => {
            audit(&app, &actor, "DeleteWorkload", &id, &format!("rejected: {e}"));
            error_page(&app, &format!("delete {id}: {e}"), &format!("/w/{id}")).into_response()
        }
    }
}

// --- volumes (user surface; NO CreateVolume) ------------------------------------

struct VolumeRow {
    id: String,
    size_mib: u32,
    state_css: String,
    state_label: String,
    attached: Option<String>,
    attached_label: String,
}

fn volume_rows(app: &AppState, tenant_id: &str) -> Vec<VolumeRow> {
    let records = app.store.list_for_tenant(tenant_id).unwrap_or_default();
    let mut rows: Vec<VolumeRow> = records
        .iter()
        .filter(|r| r.spec.volume.is_some())
        .map(|r| {
            let console = read_console(&app.data_dir, &r.spec.id);
            let d = derive_state(
                &r.state,
                &console_signals(&console),
                vm_socket_present(&app.data_dir, &r.spec.id),
            );
            VolumeRow {
                id: r.spec.id.clone(),
                size_mib: r.spec.volume.as_ref().map(|v| v.size_mib).unwrap_or(0),
                state_css: d.css().into(),
                state_label: d.label(),
                attached: Some(r.spec.id.clone()),
                attached_label: r.spec.id.clone(),
            }
        })
        .collect();
    rows.sort_by(|a, b| a.id.cmp(&b.id));
    rows
}

#[derive(Template)]
#[template(path = "volumes.html")]
struct VolumesPage {
    version: &'static str,
    base: String,
    rows: Vec<VolumeRow>,
}

async fn volumes(
    State(app): State<Arc<AppState>>,
    Extension(t): Extension<TenantCtx>,
) -> Html<String> {
let t = t;
    VolumesPage {
        version: env!("CARGO_PKG_VERSION"), base: app.base_path.clone(),
        rows: volume_rows(&app, &t.tenant_id),
    }
    .render()
    .unwrap()
    .into()
}

#[derive(Template)]
#[template(path = "images.html")]
struct ImagesPage {
    version: &'static str,
    base: String,
    images: Vec<ImageOpt>,
}

/// `/w/images` — the user-surface image listing (E-30 follow-up: the create
/// form's select shows a short id; this page carries the full digests).
async fn images(State(app): State<Arc<AppState>>) -> Html<String> {
    ImagesPage {
        version: env!("CARGO_PKG_VERSION"),
        base: app.base_path.clone(),
        images: image_options(&app, ""),
    }
    .render()
    .unwrap()
    .into()
}

#[derive(Template)]
#[template(path = "volume_detail.html")]
struct VolumeDetailPage {
    version: &'static str,
    base: String,
    row: VolumeRow,
    created: String,
    updated: String,
}

async fn volume_detail(
    State(app): State<Arc<AppState>>,
    Path(id): Path<String>,
    Extension(t): Extension<TenantCtx>,
) -> Response {
let t = t;
    let row = volume_rows(&app, &t.tenant_id).into_iter().find(|v| v.id == id);
    let rec = app.store.get_for_tenant(&t.tenant_id, &id).ok().flatten();
    match (row, rec) {
        (Some(row), Some(r)) => Html(VolumeDetailPage {
            version: env!("CARGO_PKG_VERSION"), base: app.base_path.clone(),
            row,
            created: epoch_utc(r.created_at),
            updated: epoch_utc(r.updated_at),
        }
        .render()
        .unwrap())
        .into_response(),
        _ => not_found_page(&app, &format!("volume not found: {id} (volumes live with their workload)"), "/w/volumes")
            .into_response(),
    }
}

async fn delete_volume(
    State(app): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Extension(t): Extension<TenantCtx>,
) -> Response {
    // DeleteVolume goes through the workload's wipe path (the volume file, its
    // mapper and its DEK die together — never the volume alone) and renders the
    // same wipe report. Volumes have no separate CRUD in MS-1 (doc 09 §1 bet:
    // the attach+wipe path).
    let actor = actor_of(&headers);
    // tenant gate BEFORE the wipe: a cross-tenant delete must be a 404, never
    // a wipe of another tenant's volume
let t = t;
    match app.store.get_for_tenant(&t.tenant_id, &id) {
        Ok(Some(_)) => {}
        _ => {
            return not_found_page(&app, &format!("volume not found: {id}"), "/w/volumes")
                .into_response()
        }
    }
    let id2 = id.clone();
    let app2 = app.clone();
    let result = tokio::task::spawn_blocking(move || {
        let mut ex = app2.exec.lock().unwrap();
        ops::wipe(&app2.store, &mut *ex, &id2, &porche_core::Markers::new())
    })
    .await
    .unwrap_or_else(|e| Err(ops::OpsError::InvalidSpec(format!("join: {e}"))));

    match result {
        Ok(report) => {
            audit(&app, &actor, "DeleteVolume", &id, "ok (wipe report rendered)");
            Html(
                wipe_report_page(&app.base_path, "DeleteVolume", &id, &report, "/w/volumes", "volumes")
                    .render()
                    .unwrap(),
            )
            .into_response()
        }
        Err(e) => {
            audit(&app, &actor, "DeleteVolume", &id, &format!("rejected: {e}"));
            error_page(&app, &format!("delete volume {id}: {e}"), "/w/volumes").into_response()
        }
    }
}

// --- admin surface: /a (read-only) ----------------------------------------------

fn hostname() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn read_trim(path: &str) -> String {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .trim()
        .to_string()
}

/// procfs load/mem snapshot (MS-1 host metrics).
struct LoadInfo {
    loadavg: String,
    mem_total_mb: u64,
    mem_avail_mb: u64,
    uptime: String,
    mem_used_pct: u64,
}

fn host_load() -> LoadInfo {
    let meminfo = read_trim("/proc/meminfo");
    let mem = |key: &str| -> u64 {
        meminfo
            .lines()
            .find(|l| l.starts_with(key))
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|v| v.parse().ok())
            .map(|kb: u64| kb / 1024)
            .unwrap_or(0)
    };
    let (total, avail) = (mem("MemTotal:"), mem("MemAvailable:"));
    let used_pct = if total > 0 {
        (total - avail) * 100 / total
    } else {
        0
    };
    LoadInfo {
        loadavg: read_trim("/proc/loadavg"),
        mem_total_mb: total,
        mem_avail_mb: avail,
        uptime: humanize_uptime(
            read_trim("/proc/uptime")
                .split_whitespace()
                .next()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0.0),
        ),
        mem_used_pct: used_pct,
    }
}

fn humanize_uptime(s: f64) -> String {
    let s = s as u64;
    let (d, r) = (s / 86400, s % 86400);
    let (h, r) = (r / 3600, r % 3600);
    let m = r / 60;
    if d > 0 {
        format!("{d}d {h}h {m}m")
    } else if h > 0 {
        format!("{h}h {m}m")
    } else {
        format!("{m}m")
    }
}

/// The CLH version pin executes once at startup (main.rs `ClhPin::detect`),
/// never per render — the Fleet pages read `app.clh.version`, the cached pin.

struct AllocRow {
    id: String,
    state_css: String,
    state_label: String,
    vcpu: u32,
    ram_mib: u32,
    ip6: String,
    volume: String,
    created: String,
}

/// The audit trail is written (one entry per mutation, E-26) but has no VIEW
/// page in MS-1 (doc 09 MVP cuts) — this reads the trail for the Fleet's wipe
/// aggregate only.
fn wipe_aggregate(app: &AppState) -> (usize, usize) {
    let entries = app.audit.lock().unwrap().entries();
    let mut total = 0usize;
    let mut failed = 0usize;
    for e in &entries {
        if matches!(e.action.as_str(), "DeleteWorkload" | "DeleteVolume") {
            total += 1;
            if !e.result.starts_with("ok") {
                failed += 1;
            }
        }
    }
    (total, failed)
}

#[derive(Template)]
#[template(path = "fleet.html")]
struct FleetPage {
    version: &'static str,
    base: String,
    hostname: String,
    osrelease: String,
    clh_version: String,
    load: LoadInfo,
    allocs: Vec<AllocRow>,
    wipes_total: usize,
    wipes_failed: usize,
    images: Vec<String>,
}

async fn fleet(State(app): State<Arc<AppState>>) -> Html<String> {
    let records = app.store.list().unwrap_or_default();
    let allocs: Vec<AllocRow> = records
        .iter()
        .map(|r| {
            let row = row_of(&app, r);
            AllocRow {
                id: row.id,
                state_css: row.state_css,
                state_label: row.state_label,
                vcpu: row.vcpu,
                ram_mib: row.ram_mib,
                ip6: row.ip6,
                volume: row.volume_label,
                created: row.created,
            }
        })
        .collect();
    let (wipes_total, wipes_failed) = wipe_aggregate(&app);
    let mut images: Vec<String> = image_options(&app, "")
        .into_iter()
        .map(|i| i.name)
        .collect();
    images.sort();
    FleetPage {
        version: env!("CARGO_PKG_VERSION"), base: app.base_path.clone(),
        hostname: hostname(),
        osrelease: read_trim("/proc/sys/kernel/osrelease"),
        clh_version: app.clh.version.clone(),
        load: host_load(),
        allocs,
        wipes_total,
        wipes_failed,
        images,
    }
    .render()
    .unwrap()
    .into()
}

// --- /a/host: host detail — metrics + per-workload event stream ------------------

struct HostWorkloadEvents {
    id: String,
    state_css: String,
    state_label: String,
    events: Vec<EventLine>,
}

#[derive(Template)]
#[template(path = "host_detail.html")]
struct HostDetailPage {
    version: &'static str,
    base: String,
    hostname: String,
    osrelease: String,
    clh_version: String,
    load: LoadInfo,
    workloads: Vec<HostWorkloadEvents>,
    event_count: usize,
}

async fn host_detail(State(app): State<Arc<AppState>>) -> Html<String> {
    let records = app.store.list().unwrap_or_default();
    let mut event_count = 0;
    let workloads: Vec<HostWorkloadEvents> = records
        .iter()
        .map(|r| {
            let console = read_console(&app.data_dir, &r.spec.id);
            let log_mtime = std::fs::metadata(console_path(&app.data_dir, &r.spec.id))
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64);
            let events = derive_events(r.created_at, r.updated_at, &console, log_mtime);
            event_count += events.len();
            let row = row_of(&app, r);
            HostWorkloadEvents {
                id: row.id,
                state_css: row.state_css,
                state_label: row.state_label,
                events,
            }
        })
        .collect();
    HostDetailPage {
        version: env!("CARGO_PKG_VERSION"), base: app.base_path.clone(),
        hostname: hostname(),
        osrelease: read_trim("/proc/sys/kernel/osrelease"),
        clh_version: app.clh.version.clone(),
        load: host_load(),
        workloads,
        event_count,
    }
    .render()
    .unwrap()
    .into()
}

// --- fallbacks ------------------------------------------------------------------

#[derive(Template)]
#[template(path = "error.html")]
struct ErrorPage {
    version: &'static str,
    base: String,
    message: String,
    back: String,
}

fn render_error(base_path: &str, message: &str, back: &str) -> Html<String> {
    ErrorPage {
        version: env!("CARGO_PKG_VERSION"), base: base_path.to_string(),
        message: message.into(),
        back: back.into(),
    }
    .render()
    .unwrap()
    .into()
}

fn error_page(app: &AppState, message: &str, back: &str) -> Html<String> {
    render_error(&app.base_path, message, back)
}

fn not_found_page(app: &AppState, message: &str, back: &str) -> (StatusCode, Html<String>) {
    (StatusCode::NOT_FOUND, render_error(&app.base_path, message, back))
}

// --- focused MS-1 tests ---------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::AuditLog;
    use axum::body::Body;
    use http_body_util::BodyExt;
    use porche_host::exec::{DirectExecutor, Runner};
    use tower::util::ServiceExt;

    fn test_app(tag: &str) -> Arc<AppState> {
        test_app_tokens(tag, Some("user-secret".into()), Some("admin-secret".into()))
    }

    fn test_app_tokens(
        tag: &str,
        user_token: Option<String>,
        admin_token: Option<String>,
    ) -> Arc<AppState> {
        test_app_base(tag, user_token, admin_token, String::new())
    }

    #[allow(clippy::too_many_arguments)]
    fn test_app_base(
        tag: &str,
        user_token: Option<String>,
        admin_token: Option<String>,
        base_path: String,
    ) -> Arc<AppState> {
        test_app_full(tag, user_token, admin_token, base_path, Vec::new())
    }

    fn test_app_full(
        tag: &str,
        user_token: Option<String>,
        admin_token: Option<String>,
        base_path: String,
        tenants: Vec<(String, String)>,
    ) -> Arc<AppState> {
        let dir = std::env::temp_dir().join(format!("porche-web-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Arc::new(AppState {
            tenants,
            store: porche_core::SqliteStore::open(dir.join("store.sqlite3")).unwrap(),
            exec: std::sync::Mutex::new(DirectExecutor::new(Runner::Shell, &dir, dir.join("keys.json"))),
            data_dir: dir.clone(),
            audit: std::sync::Mutex::new(AuditLog::open(dir.join("audit.jsonl"))),
            user_token,
            admin_token,
            clh: crate::ClhPin { version: "test".into() },
            base_path,
        })
    }

    fn spec_for_test(id: &str) -> porche_core::WorkloadSpec {
        porche_core::WorkloadSpec {
            tenant_id: "t0".into(),
            id: id.into(),
            image_digest: format!("sha256:{}", "b".repeat(64)),
            command: "true".into(),
            vcpu: 1,
            ram_mib: 64,
            volume: Some(porche_core::VolumeSpec { size_mib: 8 }),
            ip6: "fd00::1".into(),
            gw6: "fd00::ff".into(),
        }
    }

    async fn body_string(res: Response) -> String {
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        String::from_utf8_lossy(&bytes).into_owned()
    }

    // Focused MS-1 test: the token-domain split — the user token never opens /a,
    // the admin token never opens /w, and every unauthenticated request is a
    // direct 401 HTML page (the surface's login form), never a workaround
    // redirect (E-29, UAT `n1`).
    #[tokio::test]
    async fn token_domain_split_is_strict() {
        let app = test_app("split");
        let router = router(app);

        // user token on /a → strict 401 page (admin domain)
        let res = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/a")
                    .header(header::AUTHORIZATION, "Bearer user-secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED, "user token must never open /a");
        let page = body_string(res).await;
        assert!(page.contains("401 — authentication required"), "no 401 page body");
        assert!(page.contains("admin"), "401 page must name the admin surface");

        // admin token on /w → strict 401 page (user domain)
        let res = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/w")
                    .header(header::AUTHORIZATION, "Bearer admin-secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED, "admin token must never open /w");

        // missing token on a protected page → 401 page (not a 200 landed page)
        let res = router
            .clone()
            .oneshot(Request::builder().uri("/w/w1").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

        // POST with the wrong domain's token → empty 401, no redirect workaround
        let res = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/w/new")
                    .header(header::AUTHORIZATION, "Bearer admin-secret")
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from("id=x&command=true"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

        // each token opens its own domain
        let res = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/w")
                    .header(header::AUTHORIZATION, "Bearer user-secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let res = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/a")
                    .header(header::AUTHORIZATION, "Bearer admin-secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }

    // Focused MS-1 test: the create form renders (with the live image listing).
    #[tokio::test]
    async fn create_form_renders() {
        let app = test_app("form");
        std::fs::create_dir_all(app.data_dir.join("images")).unwrap();
        let img = app
            .data_dir
            .join("images")
            .join("sha256:bbbb");
        std::fs::create_dir_all(&img).unwrap();
        std::fs::write(img.join("manifest.txt"), "kernel 6.18.57 sha256:cccc\n").unwrap();

        let router = router(app);
        let res = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/w/new")
                    .header(header::AUTHORIZATION, "Bearer user-secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let page = body_string(res).await;
        assert!(page.contains("image_digest"), "form missing digest select");
        assert!(page.contains("sha256:bbbb"), "form missing live image dir");
        assert!(page.contains("kernel 6.18.57"), "manifest metadata not shown");
        assert!(page.contains("volume"), "form missing volume field");
    }

    // E-30: behind a reverse-proxy prefix, every route, redirect and link
    // lives under the prefix; the token domains stay strict through it.
    #[tokio::test]
    async fn base_prefix_routes_and_auth() {
        let app = test_app_base(
            "prefix",
            Some("user-secret".into()),
            Some("admin-secret".into()),
            "/porche/user".into(),
        );
        let router = router(app);

        // unauthenticated page under the prefix → 401 page whose form action
        // and links are prefix-joined
        let res = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/porche/user/w")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        let page = body_string(res).await;
        assert!(page.contains("action=\"/porche/user/w\""), "401 form posts under the prefix");

        // authenticated page under the prefix renders with prefix-joined links
        let res = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/porche/user/w")
                    .header(header::AUTHORIZATION, "Bearer user-secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let page = body_string(res).await;
        assert!(page.contains("href=\"/porche/user/w/new\""), "create link prefix-joined");

        // `?token=` under the prefix redirects back under the prefix
        let res = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/porche/user/w?token=user-secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            res.headers().get(header::LOCATION).unwrap(),
            "/porche/user/w",
            "token redirect stays under the prefix"
        );

        // bare prefix redirects to the surface home under the prefix
        let res = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/porche/user")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        assert_eq!(res.headers().get(header::LOCATION).unwrap(), "/porche/user/w");

        // the admin token still never opens the user surface, prefix or not
        let res = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/porche/user/w")
                    .header(header::AUTHORIZATION, "Bearer admin-secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED, "domain split holds behind the prefix");
    }

    // MS-2: two tenants — a tenant token lists only its workloads; a
    // cross-tenant detail is a 404 (never a leak); the admin surface still
    // sees all rows; a tenant token never opens /a.
    #[tokio::test]
    async fn tenant_view_isolation() {
        let app = test_app_full(
            "tenants",
            None, // no legacy user token: tenants are the only user credentials
            Some("admin-secret".into()),
            String::new(),
            vec![
                ("t0".into(), "tok-t0".into()),
                ("t1".into(), "tok-t1".into()),
            ],
        );
        // two workloads, one per tenant
        for (tid, wid) in [("t0", "wa"), ("t1", "wb")] {
            let mut s = spec_for_test(wid);
            s.tenant_id = tid.into();
            app.store
                .upsert(&s, &porche_core::WorkloadState::Running)
                .unwrap();
        }
        let router = router(app.clone());

        // t0 lists only wa
        let res = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/w")
                    .header(header::AUTHORIZATION, "Bearer tok-t0")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let page = body_string(res).await;
        assert!(page.contains("wa"), "t0 must see its own workload");
        assert!(!page.contains("wb"), "t0 must NOT see t1's workload");

        // t0 asking t1's detail → 404 (not a 403 leak)
        let res = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/w/wb")
                    .header(header::AUTHORIZATION, "Bearer tok-t0")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND, "cross-tenant detail must 404");

        // t1's console tail of wa → empty (no cross-tenant serial leak)
        let res = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/w/wa/console")
                    .header(header::AUTHORIZATION, "Bearer tok-t1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let page = body_string(res).await;
        assert!(!page.contains("porche.ready"), "cross-tenant console must be empty");

        // a tenant token never opens /a
        let res = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/a")
                    .header(header::AUTHORIZATION, "Bearer tok-t0")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED, "tenant token must never open /a");

        // admin still sees every row (allocations span tenants)
        let res = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/a")
                    .header(header::AUTHORIZATION, "Bearer admin-secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let page = body_string(res).await;
        assert!(page.contains("wa") && page.contains("wb"), "admin sees all tenants");
    }
}