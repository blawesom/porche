//! Route handlers — the router + the module layout of the console surfaces.
//! Every mutation maps to one doc-09 method name and writes exactly one
//! audit entry (E-26); there is no dry mode anywhere (E-29). The surfaces:
//! `auth` (the two strict token domains + tenant scope), `views` (row types
//! + fallback pages), `user` (the /w pages — the ConsoleApi seam), `admin`
//! (the /a pages, read-only).

mod admin;
mod auth;
mod user;
mod views;

use crate::AppState;
use axum::extract::State;
use axum::http::header;
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::Router;
use std::sync::Arc;

use admin::{fleet, host_detail};
use auth::auth_mw;
use user::{
    console_tail, create_form, create_submit, delete_volume, delete_workload, images,
    stop_workload, volume_detail, volumes, workload_detail, workloads,
};

#[cfg(test)]
use axum::http::{Request, StatusCode};
#[cfg(test)]
use porche_core::DesiredStateStore;

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
        .route("/w/{id}/stop", post(stop_workload))
        .route("/w/{id}/delete", post(delete_workload))
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

async fn htmx_js() -> Response {
    (
        [(header::CONTENT_TYPE, "application/javascript; charset=utf-8")],
        crate::HTMX_JS,
    )
        .into_response()
}

/// Mutating calls carry `x-porche-actor` (set by the form); reads say

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
        tenants: Vec<(String, crate::TenantEntry)>,
    ) -> Arc<AppState> {
        let dir = std::env::temp_dir().join(format!("porche-web-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = std::sync::Arc::new(porche_core::SqliteStore::open(dir.join("store.sqlite3")).unwrap());
        let exec = std::sync::Arc::new(std::sync::Mutex::new(DirectExecutor::new(
            Runner::Shell, &dir, dir.join("keys.json"),
        )));
        let audit = std::sync::Arc::new(std::sync::Mutex::new(AuditLog::open(dir.join("audit.jsonl"))));
        Arc::new(AppState {
            api: crate::adapter_poc::PocConsole::new(
                std::sync::Arc::clone(&store),
                std::sync::Arc::clone(&exec),
                dir.clone(),
                tenants.clone(),
                std::sync::Arc::clone(&audit),
            ),
            tenants,
            store,
            exec,
            data_dir: dir.clone(),
            audit,
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
                (
                    "t0".into(),
                    crate::TenantEntry {
                        token: "tok-t0".into(),
                        quota: crate::TenantQuota::default(),
                    },
                ),
                (
                    "t1".into(),
                    crate::TenantEntry {
                        token: "tok-t1".into(),
                        quota: crate::TenantQuota::default(),
                    },
                ),
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

    // MS-2.M3: quota enforcement — a tenant at its count ceiling cannot
    // create; the rejection re-renders the form with the reason and nothing
    // lands in the store.
    #[tokio::test]
    async fn quota_rejects_over_ceiling() {
        let app = test_app_full(
            "quota",
            None,
            Some("admin-secret".into()),
            String::new(),
            vec![(
                "t1".into(),
                crate::TenantEntry {
                    token: "tok-t1".into(),
                    quota: crate::TenantQuota {
                        vcpu: 4,
                        ram_mib: 512,
                        workloads: 1,
                    },
                },
            )],
        );
        // seed t1 with one workload (at the ceiling)
        let mut s = spec_for_test("q1");
        s.tenant_id = "t1".into();
        app.store
            .upsert(&s, &porche_core::WorkloadState::Running)
            .unwrap();
        let router = router(app.clone());

        let body = format!("id=q2&image_digest=sha256%3A{}&command=true&vcpu=1&ram_mib=128&volume_mib=&ip6=fd00%3A%3A2&gw6=fd00%3A%3Aff", "a".repeat(64));
        let res = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/w/new")
                    .header(header::AUTHORIZATION, "Bearer tok-t1")
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK, "form re-renders on quota reject");
        let page = body_string(res).await;
        assert!(page.contains("quota"), "rejection must name the quota");
        // and nothing landed in the store
        assert!(
            app.store.get("q2").unwrap().is_none(),
            "over-quota create must not persist a row"
        );
    }

    // route regression: every mutating route must be registered (a 2026-10-20
    // edit dropped /w/{id}/stop and /w/{id}/delete — axum 404'd empty, caught
    // by the smoke's delete step)
    #[tokio::test]
    async fn mutation_routes_are_registered() {
        let app = test_app("routes");
        let mut s = spec_for_test("rt1");
        s.tenant_id = "t0".into();
        app.store
            .upsert(&s, &porche_core::WorkloadState::Stopped)
            .unwrap();
        let router = router(app);
        for path in ["/w/rt1/stop", "/w/rt1/delete", "/w/rt1/console"] {
            let res = router
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri(path)
                        .header(header::AUTHORIZATION, "Bearer user-secret")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_ne!(
                res.status(),
                StatusCode::NOT_FOUND,
                "{path} must be a registered route"
            );
        }
    }
}