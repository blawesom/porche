//! Route handlers + askama pages. The projection rule (E-28): every mutation
//! maps to one doc-09 method name — `CreateWorkload`, `StopWorkload`,
//! `DeleteWorkload` — and writes exactly one audit entry (E-26), success or
//! rejection. Errors render with the operator's own constraint reason (the
//! explainability rule, doc 05 §2).

use crate::audit::epoch_utc;
use crate::AppState;
use askama::Template;
use axum::extract::{Form, Path, State};
use axum::http::{header, Request, StatusCode};
use axum::middleware::Next;
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::Router;
use porche_core::{ops, DesiredStateStore, Markers, Record, VolumeSpec, WorkloadSpec, WorkloadState};
use std::sync::Arc;

pub fn router(state: Arc<AppState>) -> Router {
    // every route (and template link) rides the base path — the console can be
    // served at / (local) or under a path prefix behind a proxy (E-28 deploy)
    let b = state.base.clone();
    let root_base = state.base.clone();
    let root = move || async move { Redirect::to(&format!("{root_base}/w")) };
    let r = Router::new()
        .route(&format!("{b}/w"), get(workloads))
        .route(&format!("{b}/w/new"), get(create_form).post(create_submit))
        .route(&format!("{b}/w/{{id}}"), get(workload_detail))
        .route(&format!("{b}/w/{{id}}/console"), get(console_tail))
        .route(&format!("{b}/w/{{id}}/stop"), post(stop_workload))
        .route(&format!("{b}/w/{{id}}/wipe"), post(wipe_workload))
        .route(&format!("{b}/a"), get(fleet))
        .route(&format!("{b}/a/audit"), get(audit_page))
        .route(&format!("{b}/htmx.js"), get(htmx_js));
    let r = if b.is_empty() {
        r.route("/", get(root))
    } else {
        let root2 = root.clone();
        r.route(b.as_str(), get(root)).route(format!("{b}/").as_str(), get(root2))
    };
    r.layer(axum::middleware::from_fn_with_state(
        state.clone(),
        auth_mw,
    ))
    .with_state(state)
}

// --- auth (stage 1): optional bearer token; the planes bring real domains ---

const COOKIE: &str = "porche_token";

async fn auth_mw(State(app): State<Arc<AppState>>, req: Request<axum::body::Body>, next: Next) -> Response {
    let Some(want) = app.token.clone() else {
        return next.run(req).await;
    };
    let header_ok = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .is_some_and(|v| v == want);
    let cookie_ok = req
        .headers()
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|c| c.split(';').any(|p| p.trim() == format!("{COOKIE}={want}")));
    let query_token = req
        .uri()
        .query()
        .and_then(|q| q.split('&').find(|p| p.starts_with("token=")))
        .map(|p| p.strip_prefix("token=").unwrap_or_default());
    // one-time ?token= → set the cookie, redirect to the clean path (htmx
    // polls then carry the cookie on every fragment fetch)
    if query_token.is_some_and(|t| t == want) {
        let path = req.uri().path().to_string();
        return (
            StatusCode::SEE_OTHER,
            [
                (header::SET_COOKIE, format!("{COOKIE}={want}; Path=/; Max-Age=86400; HttpOnly; SameSite=Lax")),
                (header::LOCATION, path),
            ],
        )
            .into_response();
    }
    if !header_ok && !cookie_ok {
        return (StatusCode::UNAUTHORIZED, "porche-web: bad or missing token (Bearer header, porche_token cookie, or ?token=)\n")
            .into_response();
    }
    next.run(req).await
}

async fn htmx_js() -> Response {
    (
        [(header::CONTENT_TYPE, "application/javascript; charset=utf-8")],
        crate::HTMX_JS,
    )
        .into_response()
}

fn actor(app: &AppState) -> &'static str {
    // tokens are opaque in stage 1 — one actor; the planes bring identities
    if app.token.is_some() { "token-holder" } else { "operator" }
}

fn audit(app: &Arc<AppState>, action: &str, object: &str, result: &str) {
    app.audit
        .lock()
        .unwrap()
        .record(actor(app), action, object, result);
}

// --- view models ---------------------------------------------------------------

struct WorkloadRow {
    id: String,
    state_class: String,
    state_line: String,
    vcpu: u32,
    ram_mib: u32,
    ip6: String,
    volume: String,
    created: String,
}

fn row_of(r: &Record) -> WorkloadRow {
    let (state_class, state_line) = match &r.state {
        WorkloadState::Creating => ("warn", "creating".to_string()),
        WorkloadState::Running => ("ok", "running".to_string()),
        WorkloadState::Stopping => ("warn", "stopping".to_string()),
        WorkloadState::Stopped => ("", "stopped".to_string()),
        WorkloadState::Failed { reason } => ("err", format!("failed — {reason}")),
    };
    WorkloadRow {
        id: r.spec.id.clone(),
        state_class: state_class.into(),
        state_line,
        vcpu: r.spec.vcpu,
        ram_mib: r.spec.ram_mib,
        ip6: r.spec.ip6.clone(),
        volume: r
            .spec
            .volume
            .as_ref()
            .map(|v| format!("{}M", v.size_mib))
            .unwrap_or_else(|| "—".into()),
        created: epoch_utc(r.created_at),
    }
}

// --- pages ---------------------------------------------------------------------

#[derive(Template)]
#[template(path = "workloads.html")]
struct WorkloadsPage {
    base: String,
    mode: &'static str,
    version: &'static str,
    records: Vec<WorkloadRow>,
}

async fn workloads(State(app): State<Arc<AppState>>) -> Html<String> {
    let records = app.store.list().unwrap_or_default();
    let rows = records.iter().map(row_of).collect();
    WorkloadsPage {
        base: app.base.clone(),
        mode: app.mode,
        version: env!("CARGO_PKG_VERSION"),
        records: rows,
    }
    .render()
    .unwrap_or_else(|e| panic!("template: {e}"))
    .into()
}

#[derive(Template)]
#[template(path = "workload_form.html")]
struct WorkloadFormPage {
    base: String,
    mode: &'static str,
    version: &'static str,
    error: Option<String>,
    form: FormVals,
    image_digests: Vec<(String, bool)>,
}

#[derive(Default, Clone)]
struct FormVals {
    id: String,
    image_digest: String,
    command: String,
    vcpu: String,
    ram_mib: String,
    volume_mib: String,
    ip6: String,
    gw6: String,
}

fn image_digests(app: &AppState, selected_for: &str) -> Vec<(String, bool)> {
    let mut out: Vec<String> = std::fs::read_dir(app.data_dir.join("images"))
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out.into_iter()
        .map(|d| {
            let selected = d == selected_for;
            (d, selected)
        })
        .collect()
}

async fn create_form(State(app): State<Arc<AppState>>) -> Html<String> {
    WorkloadFormPage {
        base: app.base.clone(),
        mode: app.mode,
        version: env!("CARGO_PKG_VERSION"),
        error: None,
        form: FormVals {
            vcpu: "1".into(),
            ram_mib: "128".into(),
            volume_mib: "256".into(),
            gw6: "fd42:1:1::ff".into(),
            ..Default::default()
        },
        image_digests: image_digests(&app, ""),
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
    Form(form): Form<CreateForm>,
) -> Response {
    let vals = FormVals {
        id: form.id.clone(),
        image_digest: form.image_digest.clone(),
        command: form.command.clone(),
        vcpu: form.vcpu.clone(),
        ram_mib: form.ram_mib.clone(),
        volume_mib: form.volume_mib.clone(),
        ip6: form.ip6.clone(),
        gw6: form.gw6.clone(),
    };

    let spec = build_spec(&form);
    let result = match spec {
        Ok(spec) => {
            let app2 = app.clone();
            tokio::task::spawn_blocking(move || {
                let mut ex = app2.exec.lock().unwrap();
                ops::create(&app2.store, &mut *ex, spec)
            })
            .await
            .unwrap_or_else(|e| Err(ops::OpsError::InvalidSpec(format!("join: {e}"))))
        }
        Err(e) => Err(ops::OpsError::InvalidSpec(e)),
    };

    match result {
        Ok(()) => {
            audit(&app, "CreateWorkload", &form.id, "ok");
            Redirect::to(&format!("{}/w/{}", app.base, form.id)).into_response()
        }
        Err(e) => {
            audit(&app, "CreateWorkload", &form.id, &format!("rejected: {e}"));
            Html(WorkloadFormPage {
                base: app.base.clone(),
                mode: app.mode,
                version: env!("CARGO_PKG_VERSION"),
                error: Some(e.to_string()),
                form: vals,
                image_digests: image_digests(&app, &form.image_digest),
            }
            .render()
            .unwrap())
            .into_response()
        }
    }
}

fn build_spec(form: &CreateForm) -> Result<WorkloadSpec, String> {
    let vcpu: u32 = form.vcpu.trim().parse().map_err(|_| format!("vcpu: not a number: {:?}", form.vcpu))?;
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

#[derive(Template)]
#[template(path = "workload_detail.html")]
struct WorkloadDetailPage {
    base: String,
    mode: &'static str,
    version: &'static str,
    row: WorkloadRow,
    command: String,
    image_digest: String,
    can_stop: bool,
    failed_reason: Option<String>,
}

async fn workload_detail(State(app): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    match app.store.get(&id) {
        Ok(Some(r)) => Html(WorkloadDetailPage {
            base: app.base.clone(),
            mode: app.mode,
            version: env!("CARGO_PKG_VERSION"),
            row: row_of(&r),
            command: r.spec.command,
            image_digest: r.spec.image_digest,
            can_stop: r.state == WorkloadState::Running,
            failed_reason: match &r.state {
                WorkloadState::Failed { reason } => Some(reason.clone()),
                _ => None,
            },
        }
        .render()
        .unwrap())
        .into_response(),
        _ => error_page(&app, &format!("workload not found: {id}"), &format!("{}/w", app.base)).into_response(),
    }
}

#[derive(Template)]
#[template(path = "console_tail.html")]
struct ConsoleTail {
    lines: Vec<String>,
}

async fn console_tail(State(app): State<Arc<AppState>>, Path(id): Path<String>) -> Html<String> {
    // the recorded serial log — GetConsole's recorded half in stage 1
    let path = app.data_dir.join("consoles").join(format!("{id}.log"));
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let mut lines: Vec<String> = text.lines().rev().take(40).map(String::from).collect();
    lines.reverse();
    ConsoleTail { lines }.render().unwrap().into()
}

async fn stop_workload(State(app): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
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
            audit(&app, "StopWorkload", &id, "ok");
            Redirect::to(&format!("{}/w/{}", app.base, id)).into_response()
        }
        Err(e) => {
            audit(&app, "StopWorkload", &id, &format!("rejected: {e}"));
            error_page(&app, &format!("stop {id}: {e}"), &format!("{}/w/{id}", app.base)).into_response()
        }
    }
}

#[derive(Template)]
#[template(path = "wipe_report.html")]
struct WipeReportPage {
    base: String,
    mode: &'static str,
    version: &'static str,
    id: String,
    mapper_absent: bool,
    keyfile_entry_removed: bool,
    keyslots_erased: bool,
    region_trimmed: bool,
    region_clean: bool,
}

async fn wipe_workload(State(app): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    // DeleteWorkload is an operation (doc 09 conventions): teardown with the
    // reported wipe — the report is the page.
    let id2 = id.clone();
    let app2 = app.clone();
    let result = tokio::task::spawn_blocking(move || {
        let mut ex = app2.exec.lock().unwrap();
        ops::wipe(&app2.store, &mut *ex, &id2, &Markers::new())
    })
    .await
    .unwrap_or_else(|e| Err(ops::OpsError::InvalidSpec(format!("join: {e}"))));

    match result {
        Ok(report) => {
            audit(&app, "DeleteWorkload", &id, "ok (teardown report rendered)");
            Html(WipeReportPage {
                base: app.base.clone(),
                mode: app.mode,
                version: env!("CARGO_PKG_VERSION"),
                id,
                mapper_absent: report.mapper_absent,
                keyfile_entry_removed: report.keyfile_entry_removed,
                keyslots_erased: report.keyslots_erased,
                region_trimmed: report.region_trimmed,
                region_clean: report.region_clean,
            }
            .render()
            .unwrap())
            .into_response()
        }
        Err(e) => {
            audit(&app, "DeleteWorkload", &id, &format!("rejected: {e}"));
            error_page(&app, &format!("wipe {id}: {e}"), &format!("{}/w/{id}", app.base)).into_response()
        }
    }
}

#[derive(Template)]
#[template(path = "fleet.html")]
struct FleetPage {
    base: String,
    mode: &'static str,
    version: &'static str,
    hostname: String,
    osrelease: String,
    uptime: String,
    loadavg: String,
    mem_total_mb: u64,
    mem_avail_mb: u64,
    workloads: usize,
    images: Vec<String>,
}

fn read_trim(path: &str) -> String {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .trim()
        .to_string()
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

async fn fleet(State(app): State<Arc<AppState>>) -> Html<String> {
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
    let workloads = app.store.list().map(|l| l.len()).unwrap_or(0);
    FleetPage {
        base: app.base.clone(),
        mode: app.mode,
        version: env!("CARGO_PKG_VERSION"),
        hostname: read_trim("/proc/sys/kernel/hostname"),
        osrelease: read_trim("/proc/sys/kernel/osrelease"),
        uptime: humanize_uptime(
            read_trim("/proc/uptime")
                .split_whitespace()
                .next()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0.0),
        ),
        loadavg: read_trim("/proc/loadavg"),
        mem_total_mb: mem("MemTotal:"),
        mem_avail_mb: mem("MemAvailable:"),
        workloads,
        images: image_digests(&app, "")
            .into_iter()
            .map(|(d, _)| d)
            .collect(),
    }
    .render()
    .unwrap()
    .into()
}

#[derive(Template)]
#[template(path = "audit.html")]
struct AuditPage {
    base: String,
    mode: &'static str,
    version: &'static str,
    rows: Vec<AuditRow>,
}

struct AuditRow {
    ts: String,
    actor: String,
    action: String,
    object: String,
    result: String,
}

async fn audit_page(State(app): State<Arc<AppState>>) -> Html<String> {
    let rows = app
        .audit
        .lock()
        .unwrap()
        .entries()
        .into_iter()
        .map(|e| AuditRow {
            ts: epoch_utc(e.ts),
            actor: e.actor,
            action: e.action,
            object: e.object,
            result: e.result,
        })
        .collect();
    AuditPage {
        base: app.base.clone(),
        mode: app.mode,
        version: env!("CARGO_PKG_VERSION"),
        rows,
    }
    .render()
    .unwrap()
    .into()
}

#[derive(Template)]
#[template(path = "error.html")]
struct ErrorPage {
    base: String,
    mode: &'static str,
    version: &'static str,
    message: String,
    back: String,
}

fn error_page(app: &AppState, message: &str, back: &str) -> Html<String> {
    ErrorPage {
        base: app.base.clone(),
        mode: app.mode,
        version: env!("CARGO_PKG_VERSION"),
        message: message.into(),
        back: back.into(),
    }
    .render()
    .unwrap()
    .into()
}

// --- tests: the UX cycle over the dry adapter (E-28 stage 1) --------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::AuditLog;
    use axum::body::Body;
    use http_body_util::BodyExt;
    use porche_core::WorkloadState;
    use porche_host::exec::{DirectExecutor, Runner};
    use tower::util::ServiceExt;

    fn test_app(tag: &str) -> Arc<AppState> {
        test_app_opt(tag, None)
    }

    fn test_app_opt(tag: &str, token: Option<String>) -> Arc<AppState> {
        test_app_full(tag, token, "")
    }

    fn test_app_full(tag: &str, token: Option<String>, base: &str) -> Arc<AppState> {
        let dir = std::env::temp_dir().join(format!("porche-web-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Arc::new(AppState {
            store: porche_core::SqliteStore::open(dir.join("store.sqlite3")).unwrap(),
            exec: std::sync::Mutex::new(DirectExecutor::new(
                Runner::Dry,
                &dir,
                dir.join("keys.json"),
            )),
            data_dir: dir.clone(),
            audit: std::sync::Mutex::new(AuditLog::open(dir.join("audit.jsonl"))),
            mode: "dry",
            token,
            base: base.to_string(),
        })
    }

    async fn body_string(res: Response) -> String {
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        String::from_utf8_lossy(&bytes).into_owned()
    }

    const VALID_FORM: &str = "id=w1&image_digest=sha256%3Aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa&command=true&vcpu=1&ram_mib=64&volume_mib=16&ip6=fd00%3A%3A1&gw6=fd00%3A%3Aff";

    #[tokio::test]
    async fn create_list_wipe_cycle_renders() {
        let app = test_app("cycle");
        let mut router = router(app.clone());

        // CreateWorkload → redirect to the detail page, row running
        let res = router
            .clone()
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/w/new")
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from(VALID_FORM))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            res.headers().get(header::LOCATION).unwrap(),
            "/w/w1"
        );
        assert_eq!(
            app.store.get("w1").unwrap().unwrap().state,
            WorkloadState::Running
        );

        // workloads list shows the row
        let res = router
            .clone()
            .clone()
            .oneshot(Request::builder().uri("/w").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let page = body_string(res).await;
        assert!(page.contains("w1"), "list missing w1");
        assert!(page.contains("running"), "list missing state");

        // detail renders the spec + console poll
        let res = router
            .clone()
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/w/w1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let page = body_string(res).await;
        assert!(page.contains("fd00::1"), "detail missing ip6");
        assert!(page.contains("hx-get"), "detail missing console poll");

        // DeleteWorkload → the wipe report page, row gone
        let res = router
            .clone()
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/w/w1/wipe")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let page = body_string(res).await;
        for probe in ["mapper_absent", "keyslots_erased", "region_clean"] {
            assert!(page.contains(probe), "wipe report missing {probe}");
        }
        assert!(app.store.get("w1").unwrap().is_none());

        // the audit stream holds both mutations (E-26)
        let res = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/a/audit")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let page = body_string(res).await;
        assert!(page.contains("CreateWorkload"), "audit missing create");
        assert!(page.contains("DeleteWorkload"), "audit missing delete");
    }

    #[tokio::test]
    async fn rejected_create_re_renders_with_the_reason() {
        let app = test_app("reject");
        let mut router = router(app.clone());

        let bad = "id=w9&image_digest=sha256%3Aaaaa&command=true&vcpu=1&ram_mib=64&volume_mib=&ip6=not-an-address&gw6=fd00%3A%3Aff";
        let res = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/w/new")
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from(bad))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let page = body_string(res).await;
        assert!(page.contains("not-an-address") || page.contains("invalid"), "no reason shown");
        assert!(app.store.get("w9").unwrap().is_none(), "rejected spec must not persist");

        // and the rejection itself is audited
        let page = {
            let res = router
            .clone()
                .clone()
                .oneshot(
                    Request::builder()
                        .uri("/a/audit")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            body_string(res).await
        };
        assert!(page.contains("rejected"), "audit missing rejection");
    }

    #[tokio::test]
    async fn token_gate_rejects_without_bearer() {
        let app = test_app_opt("auth", Some("secret".into()));
        let mut router = router(app);

        let res = router
            .clone()
            .oneshot(Request::builder().uri("/w").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

        let res = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/w")
                    .header(header::AUTHORIZATION, "Bearer secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn base_path_mounts_routes_and_prefixes_links() {
        let app = test_app_full("base", None, "/porche/user");
        let mut router = router(app);

        // the mount root redirects into the base
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
        assert_eq!(
            res.headers().get(header::LOCATION).unwrap(),
            "/porche/user/w"
        );

        // pages render with base-prefixed links (proxied deployments stay whole)
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
        let page = body_string(res).await;
        assert!(
            page.contains("href=\"/porche/user/w/new\""),
            "links not prefixed"
        );
        assert!(page.contains("src=\"/porche/user/htmx.js\""), "htmx not prefixed");

        // mutations redirect within the base
        let res = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/porche/user/w/new")
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from(VALID_FORM))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            res.headers().get(header::LOCATION).unwrap(),
            "/porche/user/w/w1"
        );
    }

    #[tokio::test]
    async fn query_token_sets_cookie_and_redirects() {
        let app = test_app_full("cookie", Some("secret".into()), "");
        let mut router = router(app);

        let res = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/w?token=secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::SEE_OTHER);
        assert_eq!(res.headers().get(header::LOCATION).unwrap(), "/w");
        let set_cookie = res
            .headers()
            .get(header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap();
        assert!(set_cookie.contains("porche_token=secret"), "{set_cookie}");

        // and the cookie satisfies the gate
        let res = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/w")
                    .header(header::COOKIE, "porche_token=secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }
}
