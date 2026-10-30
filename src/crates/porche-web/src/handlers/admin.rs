//! The admin surface (/a, read-only): fleet view + host detail. Stays on
//! direct store reads until the admin-plane API lands (P0/P1, doc 15) —
//! then it speaks the seam the user surface already does.

use crate::console_api::CallCtx;
use crate::state::{console_path, derive_events, read_console, EventLine};
use super::views::{hostname, image_options, row_of};
use crate::AppState;
use askama::Template;
use axum::extract::State;
use axum::response::Html;
use porche_core::DesiredStateStore;
use std::sync::Arc;

// --- admin surface: /a (read-only) ----------------------------------------------

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

pub(crate) async fn fleet(State(app): State<Arc<AppState>>) -> Html<String> {
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
    let mut images: Vec<String> = image_options(&app, &CallCtx::read("admin"), "")
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

pub(crate) async fn host_detail(State(app): State<Arc<AppState>>) -> Html<String> {
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
