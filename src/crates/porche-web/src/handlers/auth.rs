//! The two strict AuthN domains (E-8 / E-29) + the MS-2 tenant scope — the
//! middleware layer both console surfaces sit behind. `/w` and children
//! accept ONLY the user/tenant token; `/a` and children ONLY the admin
//! token; cross-domain access is a 401 (UAT negative check `n1`). A surface
//! with no configured token is closed — strict by default, never open.

use crate::AppState;
use askama::Template;
use axum::extract::State;
use axum::http::{header, HeaderMap, Request, StatusCode};
use axum::middleware::Next;
use axum::response::{Html, IntoResponse, Response};
use std::sync::Arc;

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

pub(crate) async fn auth_mw(
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
            .find(|(_, e)| e.token == tok)
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
pub(crate) fn actor_of(headers: &HeaderMap) -> String {
    headers
        .get("x-porche-actor")
        .and_then(|v| v.to_str().ok())
        .filter(|v| !v.is_empty() && v.len() <= 64)
        .map(|v| v.to_string())
        .unwrap_or_else(|| "user-token".into())
}

pub(crate) fn audit(app: &Arc<AppState>, actor: &str, action: &str, object: &str, result: &str) {
    app.audit
        .lock()
        .unwrap()
        .record(actor, action, object, result);
}

// --- view models ---------------------------------------------------------------
