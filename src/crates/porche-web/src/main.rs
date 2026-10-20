//! porche-web — the console-host (E-28; doc 12 §9, doc 09 §Console surfaces;
//! MS-1 live re-entry, E-29). Both UI surfaces (`/w` user console — tenant
//! plane; `/a` admin console — admin plane) over the **PoC adapter**: the
//! desired-state store + host-agent verbs in-process, with method names
//! shaped like the doc-09 map. The P1 adapter swaps the backend to the real
//! planes; the UIs do not change.
//!
//! MS-1 rules this binary enforces:
//! - **no dry surface, ever** (E-29) — the executor is `Runner::Shell` only.
//! - two opaque-token AuthN domains (E-8): `--user-token` for `/w`,
//!   `--admin-token` for `/a`; cross-domain and missing/invalid tokens are
//!   strict 401s.
//! - one binary role-selected at startup (I-13): `--surface user|admin` picks
//!   the listener's role and default port (user 18081, admin 18082).
//!
//! Mutations ("one binary role-selected" — I-12 single-writer model stage 1)
//! serialize through one executor lock; every mutation writes exactly one
//! audit-trail entry (E-26). The audit trail has no VIEW page at MS-1
//! (doc 09 MVP cuts) — the file is the record.

mod audit;
mod handlers;
mod state;

use audit::AuditLog;
use clap::{Parser, ValueEnum};
use porche_core::SqliteStore;
use porche_host::exec::{DirectExecutor, Runner};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub struct AppState {
    pub store: SqliteStore,
    /// host-agent ops take `&mut` — mutations hold this lock (stage 1, E-28)
    pub exec: Mutex<DirectExecutor>,
    pub data_dir: PathBuf,
    pub audit: Mutex<AuditLog>,
    /// `/w` domain token (env PORCHE_WEB_USER_TOKEN)
    pub user_token: Option<String>,
    /// `/a` domain token (env PORCHE_WEB_ADMIN_TOKEN)
    pub admin_token: Option<String>,
    /// CLH version pin — executed once at startup, never per render.
    pub clh: ClhPin,
    /// MS-2 PoC tenant map (E-31.3 — PoC-grade, static, no issuance/rotation):
    /// `(tenant_id, token)` pairs from --tenants. A tenant token opens only
    /// the user surface, scoped to its tenant. The legacy single user token
    /// (if configured) maps to `t0`, preserving pre-MS-2 behavior.
    pub tenants: Vec<(String, String)>,
    /// Reverse-proxy path prefix (E-30: `/porche/user`, `/porche/admin` behind
    /// dev.laplane.net's HAProxy; empty = direct loopback access). The
    /// middleware strips it before routing; redirects/cookie re-add it.
    pub base_path: String,
}

/// htmx, vendored (E-28 — no CDN) for live fragments only: console-log tail
/// and state polls. No SPA, no JS toolchain.
const HTMX_JS: &str = include_str!("../static/htmx.min.js");

/// One PoC tenant entry (E-31.3 marker: static token, no rotation — the P1
/// planes own real authN).
#[derive(serde::Deserialize)]
struct TenantEntry {
    token: String,
}

/// Cloud Hypervisor version line (`cloud-hypervisor --version`), cached —
/// the Fleet's CLH pin and the load rows must never re-exec per render.
pub struct ClhPin {
    pub version: String,
}

impl ClhPin {
    fn detect() -> Self {
        let bin = std::env::var("PORCHE_CLH_BIN")
            .unwrap_or_else(|_| "/usr/local/bin/cloud-hypervisor".into());
        let line = std::process::Command::new(&bin)
            .arg("--version")
            .output()
            .ok()
            .map(|o| {
                let text = String::from_utf8_lossy(&o.stdout).trim().to_string();
                if text.is_empty() {
                    String::from_utf8_lossy(&o.stderr).trim().to_string()
                } else {
                    text
                }
            })
            .unwrap_or_else(|| format!("unavailable ({} not found or failed)", bin));
        ClhPin { version: line }
    }
}

/// The console surface a listener is role-selected for (I-13). Both processes
/// speak both route trees so a cross-domain access is a strict 401 — never a
/// silent 404 on the "wrong" binary.
#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum Surface {
    User,
    Admin,
}

impl Surface {
    fn default_port(self) -> u16 {
        match self {
            Surface::User => 18081,
            Surface::Admin => 18082,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Surface::User => "user",
            Surface::Admin => "admin",
        }
    }
}

#[derive(Parser)]
#[command(
    name = "porche-web",
    version,
    about = "PORCHE console-host (MS-1, E-29) — /w user console, /a admin console; live only, no dry mode"
)]
struct Cli {
    /// Which surface this listener is role-selected for (default port: user
    /// 18081, admin 18082).
    #[arg(long, value_enum)]
    surface: Surface,
    /// Desired-state table (SQLite — doc 12 §8, E-27).
    #[arg(long, default_value = "/var/lib/porche/desired-state.sqlite3")]
    store: PathBuf,
    /// Base dir for volume regions, sockets, console logs, images, audit log.
    #[arg(long, default_value = "/var/lib/porche")]
    data_dir: PathBuf,
    /// Mock KMS: one root-only JSON file (doc 07 §2).
    #[arg(long, default_value = porche_host::volume::DEFAULT_KEYS_FILE)]
    keys_file: PathBuf,
    /// Bind address (loopback only — the console is SSH-forwarded, never
    /// published).
    #[arg(long, default_value = "127.0.0.1")]
    bind: String,
    /// Listen port (default: 18081 for --surface user, 18082 for --surface admin).
    #[arg(long)]
    port: Option<u16>,
    /// Reverse-proxy path prefix, e.g. /porche/user (E-30). Empty = direct.
    #[arg(long, env = "PORCHE_WEB_BASE_PATH", default_value = "")]
    base_path: String,
    /// `/w` user-surface token.
    #[arg(long, env = "PORCHE_WEB_USER_TOKEN")]
    user_token: Option<String>,
    /// `/a` admin-surface token.
    #[arg(long, env = "PORCHE_WEB_ADMIN_TOKEN")]
    admin_token: Option<String>,
    /// MS-2 PoC tenant map (E-31.3): JSON file `{"t0": {"token": "..."}, ...}`.
    /// Each entry opens only the user surface, scoped to its tenant. Static,
    /// no issuance/rotation — PoC-grade by design, replaced by the P1 planes.
    #[arg(long, env = "PORCHE_WEB_TENANTS")]
    tenants: Option<PathBuf>,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    // strict by default: each surface needs its token or nothing opens
    // (user surface: a tenants file also satisfies this — the tenant tokens
    // ARE the credentials; MS-2)
    let own_token = match cli.surface {
        Surface::User => {
            cli.user_token.is_some()
                || (cli.tenants.is_some()
                    && std::fs::read_to_string(cli.tenants.as_ref().unwrap())
                        .map(|t| !t.trim().is_empty())
                        .unwrap_or(false))
        }
        Surface::Admin => cli.admin_token.is_some(),
    };
    if !own_token {
        eprintln!(
            "porche-web: --surface {} needs its token (--user-token/--admin-token or \
             PORCHE_WEB_USER_TOKEN/PORCHE_WEB_ADMIN_TOKEN) — refusing to start open or blind",
            cli.surface.name()
        );
        std::process::exit(2);
    }

    // MS-2 PoC tenant map (E-31.3): static JSON, no issuance/rotation — the
    // marker is the honesty rule: this is PoC-grade auth, never claimed as
    // security. The legacy single user token (if any) maps to t0.
    let mut tenants: Vec<(String, String)> = Vec::new();
    if let Some(path) = &cli.tenants {
        let text = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("porche-web: tenants {}: {e}", path.display()));
        let map: std::collections::BTreeMap<String, TenantEntry> = serde_json::from_str(&text)
            .unwrap_or_else(|e| panic!("porche-web: tenants {}: {e}", path.display()));
        for (tid, e) in map {
            if tid.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
                && !tid.is_empty()
                && tid.len() <= 16
            {
                tenants.push((tid, e.token));
            } else {
                panic!("porche-web: invalid tenant id {tid:?} (must be [a-z0-9-], ≤16 chars)");
            }
        }
    }
    if let Some(ut) = &cli.user_token {
        // legacy single-tenant token → t0 scope (preserves pre-MS-2 behavior)
        tenants.push(("t0".into(), ut.clone()));
    }

    // E-30: normalize the reverse-proxy prefix once — no trailing slash, no
    // wildcard; empty means direct loopback access (the MS-1 default).
    let base_path = {
        let p = cli.base_path.trim().trim_end_matches('/').to_string();
        if p.contains('*') || (p.starts_with('/') && p.contains("//")) {
            eprintln!("porche-web: invalid --base-path {p:?}");
            std::process::exit(2);
        }
        p
    };

    std::fs::create_dir_all(&cli.data_dir)
        .unwrap_or_else(|e| panic!("porche-web: data dir {}: {e}", cli.data_dir.display()));

    let store = SqliteStore::open(&cli.store)
        .unwrap_or_else(|e| panic!("porche-web: store {}: {e}", cli.store.display()));
    // E-29: real execution only — `Runner::Dry` and `--dry` do not exist.
    let exec = DirectExecutor::new(Runner::Shell, &cli.data_dir, &cli.keys_file);

    let state = Arc::new(AppState {
        store,
        exec: Mutex::new(exec),
        data_dir: cli.data_dir.clone(),
        audit: Mutex::new(AuditLog::open(cli.data_dir.join("console-audit.jsonl"))),
        user_token: cli.user_token.clone(),
        admin_token: cli.admin_token.clone(),
        clh: ClhPin::detect(),
        tenants: tenants.clone(),
        base_path: base_path.clone(),
    });

    let port = cli.port.unwrap_or_else(|| cli.surface.default_port());
    let addr = format!("{}:{}", cli.bind, port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|e| panic!("porche-web: bind {addr}: {e}"));
    let prefix = if base_path.is_empty() {
        "/".to_string()
    } else {
        format!("{base_path}/")
    };
    eprintln!(
        "porche-web: http://{}/{}w (user) and {}/{}a (admin) — {} surface, live only (E-29)",
        listener.local_addr().unwrap(),
        prefix,
        listener.local_addr().unwrap(),
        prefix.clone(),
        cli.surface.name()
    );
    axum::serve(listener, handlers::router(state))
        .await
        .expect("porche-web: server error");
}