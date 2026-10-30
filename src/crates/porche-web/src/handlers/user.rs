//! The user surface (/w): every page speaks the ConsoleApi seam (doc 12
//! §9) — handlers parse forms, call the adapter, render askama pages. No
//! direct store/ops access here (the doc-15 P0 gate).

use super::auth::{actor_of, audit, TenantCtx};
use crate::audit::epoch_utc;
use crate::console_api::{ApiError, CallCtx, ConsoleApi};
use crate::state::{
    console_signals, derive_state, read_console, vm_socket_present, DisplayState, EventLine,
};
use super::views::{
    error_page, hostname, image_options, not_found_page, row_of, volume_rows, ImageOpt,
    VolumeRow, WorkloadRow,
};
use crate::AppState;
use askama::Template;
use axum::extract::{Extension, Form, Path, State};
use axum::http::HeaderMap;
use axum::response::{Html, IntoResponse, Redirect, Response};
use porche_core::{VolumeSpec, WorkloadSpec};
use std::sync::Arc;

// --- user surface: /w ----------------------------------------------------------

#[derive(Template)]
#[template(path = "workloads.html")]
struct WorkloadsPage {
    version: &'static str,
    base: String,
    records: Vec<WorkloadRow>,
}

pub(crate) async fn workloads(
    State(app): State<Arc<AppState>>,
    Extension(t): Extension<TenantCtx>,
) -> Html<String> {
    let ctx = CallCtx::read(&t.tenant_id);
    let records = app.api.list_workloads(&ctx).unwrap_or_default();
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

pub(crate) async fn create_form(
    State(app): State<Arc<AppState>>,
    Extension(t): Extension<TenantCtx>,
) -> Html<String> {
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
        images: image_options(&app, &CallCtx::read(&t.tenant_id), ""),
    }
    .render()
    .unwrap()
    .into()
}

#[derive(serde::Deserialize, Debug)]
pub(crate) struct CreateForm {
    id: String,
    image_digest: String,
    command: String,
    vcpu: String,
    ram_mib: String,
    volume_mib: String,
    ip6: String,
    gw6: String,
}

/// MS-2.M3 quota gate (FR-5 skeleton): sums the tenant's live rows and
/// rejects a create that would exceed its vCPU/RAM/count ceiling. Returns
/// the rejection reason, or None when within quota. A tenant not in the
/// map (legacy t0 with no entry) has no quota — PoC default-open for the
/// operator's own scope only.
pub(crate) async fn create_submit(
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

    let spec = build_spec(&form, &t.tenant_id);
    match spec {
        Ok(spec) => {
            // validate BEFORE admission: an invalid spec must re-render the
            // form with the reason, never leave a row stuck in `Creating`
            // awaiting a task that will fail at `ops::create`'s own validate()
            // gate (observed via the UAT smoke's over-long id, 2026-10-20).
            if let Err(e) = spec.validate() {
                audit(&app, &actor, "CreateWorkload", &form.id, &format!("rejected: {e}"));
                return Html(WorkloadFormPage {
                    version: env!("CARGO_PKG_VERSION"),
                    base: app.base_path.clone(),
                    error: Some(e),
                    form: vals,
                    images: image_options(&app, &CallCtx::read(&t.tenant_id), &form.image_digest),
                }
                .render()
                .unwrap())
                .into_response();
            }
            // CreateWorkload through the API seam (doc 12 §9): the adapter
            // does the atomic admission (duplicate-id + tenant quota +
            // insert — one critical section, the doc-15 create-path race
            // closed) and spawns the convergence; the row is observably
            // `pending` (Creating) when the redirect lands (MS-1 u03).
            let ctx = CallCtx::mutate(&t.tenant_id, &actor);
            match app.api.create_workload(&ctx, spec) {
                Ok(()) => {}
                Err(ApiError::Rejected(e)) | Err(ApiError::Failed(e)) => {
                    audit(&app, &actor, "CreateWorkload", &form.id, &format!("rejected: {e}"));
                    return Html(WorkloadFormPage {
                        version: env!("CARGO_PKG_VERSION"),
                        base: app.base_path.clone(),
                        error: Some(e),
                        form: vals,
                        images: image_options(&app, &CallCtx::read(&t.tenant_id), &form.image_digest),
                    }
                    .render()
                    .unwrap())
                    .into_response();
                }
                Err(ApiError::NotFound(e)) => {
                    audit(&app, &actor, "CreateWorkload", &form.id, &format!("rejected: {e}"));
                    return not_found_page(&app, &e, "/w").into_response();
                }
            }
            // the convergence spawn + the settle audit entry live in the
            // adapter now — the page layer projects, the seam executes
            audit(&app, &actor, "CreateWorkload", &form.id, "ordered");
            Redirect::to(&format!("/w/{}", form.id)).into_response()
        }
        Err(e) => {
            audit(&app, &actor, "CreateWorkload", &form.id, &format!("rejected: {e}"));
            Html(WorkloadFormPage {
                version: env!("CARGO_PKG_VERSION"), base: app.base_path.clone(),
                error: Some(e.to_string()),
                form: vals,
                images: image_options(&app, &CallCtx::read(&t.tenant_id), &form.image_digest),
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
    /// owning tenant (MS-2 legibility)
    tenant_id: String,
}

pub(crate) async fn workload_detail(
    State(app): State<Arc<AppState>>,
    Path(id): Path<String>,
    Extension(t): Extension<TenantCtx>,
) -> Response {
    // Get + GetEvents through the seam; the display derivation (E-29:
    // console markers + socket absence) stays console-side
    let ctx = CallCtx::read(&t.tenant_id);
    match app.api.get_workload(&ctx, &id) {
        Ok(Some(r)) => {
            let events = app
                .api
                .get_events(&ctx, &id)
                .unwrap_or_default();
            let console = read_console(&app.data_dir, &id);
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
                tenant_id: r.spec.tenant_id.clone(),
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

pub(crate) async fn console_tail(
    State(app): State<Arc<AppState>>,
    Path(id): Path<String>,
    Extension(t): Extension<TenantCtx>,
) -> Html<String> {
    // GetConsole — the recorded serial log tail (MS-1: recorded half).
    // Cross-tenant ids get the empty tail (the adapter's rule — never
    // another tenant's serial log).
    let text = app
        .api
        .get_console(&CallCtx::read(&t.tenant_id), &id)
        .unwrap_or_default();
    let mut lines: Vec<String> = text.lines().rev().take(40).map(String::from).collect();
    lines.reverse();
    ConsoleTail { lines }.render().unwrap().into()
}

pub(crate) async fn stop_workload(
    State(app): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Extension(t): Extension<TenantCtx>,
) -> Response {
    let actor = actor_of(&headers);
    // StopWorkload through the seam; the tenant gate is the adapter's
    // (cross-tenant stop = NotFound, never a wipe of another tenant's row)
    let app2 = app.clone();
    let ctx = CallCtx::mutate(&t.tenant_id, &actor);
    let id2 = id.clone();
    let result = tokio::task::spawn_blocking(move || app2.api.stop_workload(&ctx, &id2))
        .await
        .unwrap_or_else(|e| Err(ApiError::Failed(format!("join: {e}"))));

    match result {
        Ok(()) => {
            audit(&app, &actor, "StopWorkload", &id, "ok");
            Redirect::to(&format!("/w/{id}")).into_response()
        }
        Err(ApiError::NotFound(e)) => {
            not_found_page(&app, &e, "/w").into_response()
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

pub(crate) async fn delete_workload(
    State(app): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Extension(t): Extension<TenantCtx>,
) -> Response {
    // DeleteWorkload is an operation (doc 09 conventions): teardown with the
    // reported wipe — the report is the page. The tenant gate is the
    // adapter's (cross-tenant delete = NotFound, never another tenant's wipe).
    let actor = actor_of(&headers);
    let app2 = app.clone();
    let ctx = CallCtx::mutate(&t.tenant_id, &actor);
    let id2 = id.clone();
    let result = tokio::task::spawn_blocking(move || app2.api.delete_workload(&ctx, &id2))
        .await
        .unwrap_or_else(|e| Err(ApiError::Failed(format!("join: {e}"))));

    match result {
        Ok(report) => {
            audit(&app, &actor, "DeleteWorkload", &id, "ok (wipe report rendered)");
            Html(wipe_report_page(&app.base_path, "DeleteWorkload", &id, &report, "/w", "workloads").render().unwrap())
                .into_response()
        }
        Err(ApiError::NotFound(e)) => {
            not_found_page(&app, &e, "/w").into_response()
        }
        Err(e) => {
            audit(&app, &actor, "DeleteWorkload", &id, &format!("rejected: {e}"));
            error_page(&app, &format!("delete {id}: {e}"), &format!("/w/{id}")).into_response()
        }
    }
}

#[derive(Template)]
#[template(path = "volumes.html")]
struct VolumesPage {
    version: &'static str,
    base: String,
    rows: Vec<VolumeRow>,
}

pub(crate) async fn volumes(
    State(app): State<Arc<AppState>>,
    Extension(t): Extension<TenantCtx>,
) -> Html<String> {
    VolumesPage {
        version: env!("CARGO_PKG_VERSION"), base: app.base_path.clone(),
        rows: volume_rows(&app, &CallCtx::read(&t.tenant_id)),
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
pub(crate) async fn images(
    State(app): State<Arc<AppState>>,
    Extension(t): Extension<TenantCtx>,
) -> Html<String> {
    ImagesPage {
        version: env!("CARGO_PKG_VERSION"),
        base: app.base_path.clone(),
        images: image_options(&app, &CallCtx::read(&t.tenant_id), ""),
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

pub(crate) async fn volume_detail(
    State(app): State<Arc<AppState>>,
    Path(id): Path<String>,
    Extension(t): Extension<TenantCtx>,
) -> Response {
    let ctx = CallCtx::read(&t.tenant_id);
    let row = volume_rows(&app, &ctx).into_iter().find(|v| v.id == id);
    let rec = app.api.get_workload(&ctx, &id).unwrap_or_default();
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

pub(crate) async fn delete_volume(
    State(app): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Extension(t): Extension<TenantCtx>,
) -> Response {
    // DeleteVolume goes through the workload's wipe path (the volume file, its
    // mapper and its DEK die together — never the volume alone) and renders the
    // same wipe report. Volumes have no separate CRUD in MS-1 (doc 09 §1 bet:
    // the attach+wipe path). The tenant gate is the adapter's (cross-tenant
    // delete = NotFound, never another tenant's wipe).
    let actor = actor_of(&headers);
    let app2 = app.clone();
    let ctx = CallCtx::mutate(&t.tenant_id, &actor);
    let id2 = id.clone();
    let result = tokio::task::spawn_blocking(move || app2.api.delete_volume(&ctx, &id2))
        .await
        .unwrap_or_else(|e| Err(ApiError::Failed(format!("join: {e}"))));

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
        Err(ApiError::NotFound(_)) => {
            not_found_page(&app, &format!("volume not found: {id}"), "/w/volumes")
                .into_response()
        }
        Err(e) => {
            audit(&app, &actor, "DeleteVolume", &id, &format!("rejected: {e}"));
            error_page(&app, &format!("delete volume {id}: {e}"), "/w/volumes").into_response()
        }
    }
}

