//! View models + shared page helpers (E-28 projection rule): the row types
//! the askama pages render, derived from store records + observable signals
//! (E-29 display rule), and the fallback pages (error / not-found) both
//! surfaces use.

use crate::audit::epoch_utc;
use crate::console_api::{CallCtx, ConsoleApi};
use crate::state::{console_signals, derive_state, read_console, vm_socket_present};
use crate::AppState;
use askama::Template;
use axum::http::StatusCode;
use axum::response::Html;
use porche_core::{Record, WorkloadState};

// --- view models ---------------------------------------------------------------

pub(crate) struct WorkloadRow {
    pub(crate) id: String,
    pub(crate) state_css: String,
    pub(crate) state_label: String,
    pub(crate) vcpu: u32,
    pub(crate) ram_mib: u32,
    pub(crate) ip6: String,
    pub(crate) volume_label: String,
    pub(crate) volume_link: bool,
    pub(crate) created: String,
    pub(crate) can_stop: bool,
    pub(crate) can_delete: bool,
}
/// Image option row for the create form: dir name (the digest — the value),
/// a friendly short id for display, and manifest metadata lines when present.
pub(crate) struct ImageOpt {
    pub(crate) name: String,
    /// short display id, e.g. `sha256:962913…` (first 6 hash chars)
    pub(crate) short: String,
    pub(crate) manifest: String,
    pub(crate) selected: bool,
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

/// The create form's image options — the `ListImages` read through the API
/// seam, mapped to the select's view model.
pub(crate) fn image_options(app: &AppState, ctx: &CallCtx, selected_for: &str) -> Vec<ImageOpt> {
    app.api
        .list_images(ctx)
        .unwrap_or_default()
        .into_iter()
        .map(|i| ImageOpt {
            short: short_digest(&i.digest),
            selected: i.digest == selected_for,
            name: i.digest,
            manifest: i.manifest,
        })
        .collect()
}

pub(crate) fn row_of(app: &AppState, r: &Record) -> WorkloadRow {
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

// --- volumes (user surface; NO CreateVolume) ------------------------------------

pub(crate) struct VolumeRow {
    pub(crate) id: String,
    pub(crate) size_mib: u32,
    pub(crate) state_css: String,
    pub(crate) state_label: String,
    pub(crate) attached: Option<String>,
    pub(crate) attached_label: String,
}

pub(crate) fn volume_rows(app: &AppState, ctx: &CallCtx) -> Vec<VolumeRow> {
    let records = app.api.list_volumes(ctx).unwrap_or_default();
    let mut rows: Vec<VolumeRow> = records
        .iter()
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

pub(crate) fn error_page(app: &AppState, message: &str, back: &str) -> Html<String> {
    render_error(&app.base_path, message, back)
}

pub(crate) fn not_found_page(app: &AppState, message: &str, back: &str) -> (StatusCode, Html<String>) {
    (StatusCode::NOT_FOUND, render_error(&app.base_path, message, back))
}
pub(crate) fn hostname() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .unwrap_or_default()
        .trim()
        .to_string()
}
