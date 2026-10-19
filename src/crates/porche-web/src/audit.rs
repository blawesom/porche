//! The console audit trail (the E-26 convention as a stream): every mutation
//! through the console-host = exactly one entry, success or rejection. Stage 1
//! keeps it beside the desired-state store; the planes (P1) write the same
//! entries via SEC-23.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(serde::Deserialize)]
pub struct AuditEntry {
    pub ts: i64,
    pub actor: String,
    pub action: String,
    pub object: String,
    pub result: String,
}

pub struct AuditLog {
    path: PathBuf,
}

/// What the audit page serves per read (bounded; the JSONL file is the record).
const READ_BACK: usize = 200;

impl AuditLog {
    pub fn open(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn record(&mut self, actor: &str, action: &str, object: &str, result: &str) {
        let e = AuditEntry {
            ts: now_epoch(),
            actor: actor.into(),
            action: action.into(),
            object: object.into(),
            result: result.into(),
        };
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&self.path) {
            let _ = writeln!(
                f,
                "{}",
                serde_json::json!({
                    "ts": e.ts, "actor": e.actor, "action": e.action,
                    "object": e.object, "result": e.result
                })
            );
        }
    }

    /// Newest first, read back from the JSONL — every writer (both console
    /// surfaces, later the CLI) lands here, not just this process's ring.
    pub fn entries(&self) -> Vec<AuditEntry> {
        let text = std::fs::read_to_string(&self.path).unwrap_or_default();
        let mut out: Vec<AuditEntry> = text
            .lines()
            .filter(|l| !l.trim().is_empty())
            .filter_map(|l| serde_json::from_str::<AuditEntry>(l).ok())
            .collect();
        if out.len() > READ_BACK {
            let skip = out.len() - READ_BACK;
            out.drain(..skip);
        }
        out.reverse();
        out
    }
}

pub fn now_epoch() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Epoch seconds → "YYYY-MM-DD hh:mm:ss UTC" (civil-from-days — no chrono dep).
pub fn epoch_utc(ts: i64) -> String {
    let days = ts.div_euclid(86_400);
    let secs = ts.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02} UTC",
        secs / 3600,
        (secs % 3600) / 60,
        secs % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_utc_formats_known_values() {
        // 2026-10-19 12:00:00 UTC = 1792411200
        assert_eq!(epoch_utc(1_792_411_200), "2026-10-19 12:00:00 UTC");
        assert_eq!(epoch_utc(0), "1970-01-01 00:00:00 UTC");
    }

    #[test]
    fn record_appends_and_reverses() {
        let dir = std::env::temp_dir().join(format!("porche-audit-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut log = AuditLog::open(dir.join("audit.jsonl"));
        log.record("operator", "CreateWorkload", "w1", "ok");
        log.record("operator", "DeleteWorkload", "w1", "ok");
        let entries = log.entries();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].action, "DeleteWorkload"); // newest first
        let text = std::fs::read_to_string(dir.join("audit.jsonl")).unwrap();
        assert!(text.contains("CreateWorkload"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
