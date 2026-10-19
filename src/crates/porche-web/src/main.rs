//! porche-web — the console-host (E-28; doc 12 §9, doc 09 §Console surfaces):
//! both UI surfaces (`/w` user console — tenant plane; `/a` admin console —
//! admin plane) over the **PoC adapter**: the desired-state store + host-agent
//! verbs in-process, with method names shaped like the doc-09 map. The P1
//! adapter swaps the backend to the real planes; the UIs do not change.
//!
//! Stage 1: mutations serialize through one executor lock (the CLI's
//! single-writer model — I-12 reconciliation removes it in P1). AuthN domains
//! land with the planes; stage 1 carries an optional bearer token.

mod audit;
mod handlers;

use audit::AuditLog;
use clap::Parser;
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
    pub mode: &'static str,
    pub token: Option<String>,
}

/// htmx, vendored (E-28 — no CDN; pinned below) for live fragments only:
/// console-log tail and state polls. No SPA.
const HTMX_JS: &str = include_str!("../static/htmx.min.js");

#[derive(Parser)]
#[command(
    name = "porche-web",
    version,
    about = "PORCHE console-host (E-28) — /w user console, /a admin console"
)]
struct Cli {
    /// Desired-state table (SQLite — doc 12 §8, E-27).
    #[arg(long, default_value = "/var/lib/porche/desired-state.sqlite3")]
    store: PathBuf,
    /// Base dir for volume regions, sockets, console logs, images, audit log.
    #[arg(long, default_value = "/var/lib/porche")]
    data_dir: PathBuf,
    /// Mock KMS: one root-only JSON file (doc 07 §2).
    #[arg(long, default_value = porche_host::volume::DEFAULT_KEYS_FILE)]
    keys_file: PathBuf,
    /// Dry mode: the full UX cycle with nothing executed (offline demo —
    /// Runner::Dry: create converges, teardown reports all-pass).
    #[arg(long)]
    dry: bool,
    /// Bind address.
    #[arg(long, default_value = "127.0.0.1:8080")]
    bind: String,
    /// Optional bearer token — stage-1 stand-in; the planes bring the real
    /// authN domains (E-8/FR-40, E-28).
    #[arg(long, env = "PORCHE_WEB_TOKEN")]
    token: Option<String>,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    std::fs::create_dir_all(&cli.data_dir)
        .unwrap_or_else(|e| panic!("porche-web: data dir {}: {e}", cli.data_dir.display()));

    let store = SqliteStore::open(&cli.store)
        .unwrap_or_else(|e| panic!("porche-web: store {}: {e}", cli.store.display()));
    let runner = if cli.dry { Runner::Dry } else { Runner::Shell };
    let mode = if cli.dry { "dry" } else { "live" };
    let exec = DirectExecutor::new(runner, &cli.data_dir, &cli.keys_file);

    let state = Arc::new(AppState {
        store,
        exec: Mutex::new(exec),
        data_dir: cli.data_dir.clone(),
        audit: Mutex::new(AuditLog::open(cli.data_dir.join("console-audit.jsonl"))),
        mode,
        token: cli.token,
    });

    let listener = tokio::net::TcpListener::bind(&cli.bind)
        .await
        .unwrap_or_else(|e| panic!("porche-web: bind {}: {e}", cli.bind));
    eprintln!(
        "porche-web: http://{}/w ({mode} mode) — user console; /a admin console",
        listener.local_addr().unwrap()
    );
    axum::serve(listener, handlers::router(state))
        .await
        .expect("porche-web: server error");
}
