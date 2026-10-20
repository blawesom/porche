//! Observable-state derivation for the MS-1 console (E-29 display rule).
//!
//! The display state `done (rc=N)` is derived from **observable signals** —
//! the guest's `porche.done rc=N` console marker AND the absence of the VM API
//! socket in `$DATA_DIR/run/{id}.sock` — never from the store enum alone
//! (`runs/20261020-M1/ms1/02-create/results.md`: a `running` store row can be
//! observably done the moment the job finished and powered off).
//!
//! Event lines (GetEvents) are derived from the store's `created_at`/`updated_at`
//! plus the console markers `porche.ready`, `porche.done rc=N` and
//! `reboot: Power down`. Marker wall-times are estimates anchored on the
//! kernel's monotonic `[secs]` prefixes and the console-log file mtime, so
//! they display with `≈` (plus the raw boot offset) — honest about their
//! provenance.

use crate::audit::epoch_utc;
use porche_core::WorkloadState;
use std::path::{Path, PathBuf};

/// The CLH API socket for a workload — its absence means the VM is gone.
pub fn vm_socket_present(data_dir: &Path, id: &str) -> bool {
    data_dir.join("run").join(format!("{id}.sock")).exists()
}

pub fn console_path(data_dir: &Path, id: &str) -> PathBuf {
    data_dir.join("consoles").join(format!("{id}.log"))
}

pub fn read_console(data_dir: &Path, id: &str) -> String {
    std::fs::read_to_string(console_path(data_dir, id)).unwrap_or_default()
}

/// What the console log proves. `done_rc` is `Some(n)` when `porche.done rc=n`
/// was seen with a readable number, `Some(None)` when the marker is there but
/// the number is not readable, `None` when no done marker exists at all.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConsoleSignals {
    pub done_rc: Option<Option<i32>>,
    pub ready: bool,
    pub power_down: bool,
}

/// Parse the rc out of the tail after a `porche.done` marker. The payload's
/// command output may share the marker line (`porche-helloporche.done rc=0` —
/// observed in `runs/20261020-M1/ms1/02-create/serial-console.log`), so the
/// marker is matched anywhere in the line.
fn done_rc_after(tail: &str) -> Option<i32> {
    let i = tail.find("rc=")?;
    let rest = &tail[i + 3..];
    let neg = rest.starts_with('-');
    let digits: String = rest
        .chars()
        .skip_while(|c| *c == '-')
        .take_while(|c| c.is_ascii_digit())
        .collect();
    if digits.is_empty() {
        return None;
    }
    let s = if neg {
        format!("-{digits}")
    } else {
        digits
    };
    s.parse().ok()
}

pub fn console_signals(text: &str) -> ConsoleSignals {
    let mut s = ConsoleSignals::default();
    for line in text.lines() {
        if let Some(i) = line.find("porche.done") {
            if s.done_rc.is_none() {
                s.done_rc = Some(done_rc_after(&line[i + "porche.done".len()..]));
            }
        }
        if line.contains("porche.ready") {
            s.ready = true;
        }
        if line.contains("reboot: Power down") {
            s.power_down = true;
        }
    }
    s
}

/// The display state of one workload object (labels are the console's words).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DisplayState {
    Pending,
    Running,
    Stopping,
    Stopped,
    /// Derived: the console carries `porche.done rc=N` (or the VM powered down
    /// without one) AND the VM socket is gone — see this module's header.
    Done(Option<i32>),
    Failed(String),
}

impl DisplayState {
    pub fn label(&self) -> String {
        match self {
            DisplayState::Pending => "pending".into(),
            DisplayState::Running => "running".into(),
            DisplayState::Stopping => "stopping".into(),
            DisplayState::Stopped => "stopped".into(),
            DisplayState::Done(Some(rc)) => format!("done (rc={rc})"),
            DisplayState::Done(None) => "done (rc=?)".into(),
            DisplayState::Failed(r) => format!("failed — {r}"),
        }
    }

    pub fn css(&self) -> &'static str {
        match self {
            DisplayState::Pending | DisplayState::Stopping => "warn",
            DisplayState::Running => "ok",
            DisplayState::Stopped => "",
            DisplayState::Done(Some(rc)) if *rc == 0 => "ok",
            DisplayState::Done(_) => "err",
            DisplayState::Failed(_) => "err",
        }
    }
}

/// MS-1 display-state derivation. The store enum is the *ordered transition*
/// only; `done (rc=N)` requires BOTH the console marker and a gone VM socket.
/// An operator stop (`stopped`) still displays as `stopped` — the done rule
/// never re-labels an ordered stop without a done marker. The `Failed` reason
/// is store-observable (it is written by `ops` rollback), so it displays as-is.
pub fn derive_state(
    store_state: &WorkloadState,
    signals: &ConsoleSignals,
    socket_present: bool,
) -> DisplayState {
    if let WorkloadState::Failed { reason } = store_state {
        return DisplayState::Failed(reason.clone());
    }
    if signals.done_rc.is_some() && !socket_present {
        return DisplayState::Done(signals.done_rc.unwrap());
    }
    match store_state {
        WorkloadState::Stopping => DisplayState::Stopping,
        WorkloadState::Stopped => DisplayState::Stopped,
        WorkloadState::Creating => DisplayState::Pending,
        WorkloadState::Running => {
            if signals.power_down && !socket_present {
                // VM exited on its own without printing a done marker
                DisplayState::Done(None)
            } else {
                DisplayState::Running
            }
        }
        WorkloadState::Failed { .. } => unreachable!("handled above"),
    }
}

/// One derived timeline row (GetEvents).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventLine {
    /// UTC stamp; `≈` marks a console-marker estimate, `time unknown` = no
    /// anchor available.
    pub time: String,
    pub label: String,
    pub source: &'static str,
    pub tone: &'static str,
}

/// One console marker seen in the log, with the kernel monotonic offset that
/// anchors it (the marker's own `[secs]` prefix if it has one, else the last
/// offset seen before it).
#[derive(Debug, Clone, PartialEq)]
pub struct ConsoleMark {
    pub label: String,
    pub offset: Option<f64>,
    pub tone: &'static str,
}

fn kernel_offset(line: &str) -> Option<f64> {
    let rest = line.strip_prefix('[')?;
    let end = rest.find(']')?;
    rest[..end].trim().parse().ok()
}

/// All marker occurrences in log order + the largest kernel offset in the log
/// (the anchor for the wall-time estimate).
pub fn console_marks(text: &str) -> (Vec<ConsoleMark>, Option<f64>) {
    let mut marks = Vec::new();
    let mut last_off: Option<f64> = None;
    let mut max_off: Option<f64> = None;
    for line in text.lines() {
        let own = kernel_offset(line);
        if let Some(o) = own {
            last_off = Some(o);
            max_off = Some(max_off.map_or(o, |m: f64| m.max(o)));
        }
        let label = if line.contains("porche.done") {
            let rc = console_signals(line).done_rc.flatten();
            Some(match rc {
                Some(rc) => format!("porche.done rc={rc} (job reported)"),
                None => "porche.done rc=? (job reported)".to_string(),
            })
        } else if line.contains("porche.ready") {
            Some("porche.ready (guest up)".to_string())
        } else if line.contains("reboot: Power down") {
            Some("reboot: Power down (VM exited)".to_string())
        } else {
            None
        };
        if let Some(label) = label {
            let tone = if label.contains("porche.done rc=0") {
                "ok"
            } else if label.contains("power down") {
                ""
            } else {
                "warn"
            };
            marks.push(ConsoleMark {
                label,
                offset: own.or(last_off),
                tone,
            });
        }
    }
    (marks, max_off)
}

/// GetEvents timeline: store transitions + console markers, in order.
///
/// `log_mtime` is the console-log file's mtime (epoch seconds) — the anchor for
/// the `≈` wall-time estimates of the marker rows.
pub fn derive_events(
    created_at: i64,
    updated_at: i64,
    console: &str,
    log_mtime: Option<i64>,
) -> Vec<EventLine> {
    let mut out = Vec::new();
    out.push(EventLine {
        time: epoch_utc(created_at),
        label: "created (CreateWorkload ordered)".into(),
        source: "store",
        tone: "",
    });

    let (marks, max_off) = console_marks(console);
    for m in &marks {
        let (time, label) = match (m.offset, max_off, log_mtime) {
            (Some(a), Some(mx), Some(mt)) => {
                let wall = mt - (mx - a).round() as i64;
                (
                    format!("≈ {}", epoch_utc(wall)),
                    format!("{} (boot +{:.2}s)", m.label, a),
                )
            }
            (Some(a), _, _) => (
                "time unknown".to_string(),
                format!("{} (boot +{:.2}s)", m.label, a),
            ),
            _ => ("time unknown".to_string(), m.label.clone()),
        };
        out.push(EventLine {
            time,
            label,
            source: "console",
            tone: m.tone,
        });
    }

    if updated_at > created_at {
        out.push(EventLine {
            time: epoch_utc(updated_at),
            label: "updated (last store transition)".into(),
            source: "store",
            tone: "",
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // Focused MS-1 test: `done (rc=N)` derives from the console marker + VM
    // socket absence — never from the store enum alone.
    #[test]
    fn done_rc_derives_from_console_log_and_socket_absence() {
        // real marker shapes from runs/20261020-M1/ms1/02-create/serial-console.log
        let log = "boot noise\nporche.ready\nporche-helloporche.done rc=0\n[    0.938874] reboot: Power down\n";
        let sig = console_signals(log);
        assert_eq!(sig.done_rc, Some(Some(0)), "rc must parse out of a shared line");
        assert!(sig.ready && sig.power_down);

        let running = WorkloadState::Running;
        // socket gone → done (rc=0), even though the store still says running
        assert_eq!(
            derive_state(&running, &sig, false),
            DisplayState::Done(Some(0))
        );
        // socket still present → not done yet
        assert_eq!(derive_state(&running, &sig, true), DisplayState::Running);
        // the store enum alone (no console marker) never yields done
        let quiet = console_signals("");
        assert_eq!(derive_state(&running, &quiet, false), DisplayState::Running);
        // an ordered stop stays `stopped` without a done marker
        let stopped = WorkloadState::Stopped;
        assert_eq!(derive_state(&stopped, &quiet, false), DisplayState::Stopped);
    }
}