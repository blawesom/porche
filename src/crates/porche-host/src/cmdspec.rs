//! A single external command, as argv — built and unit-tested as data, then executed
//! (or printed, in dry-run). No shell string interpolation anywhere: every argv is
//! exact, which is what makes the WS-C on-node sequence reviewable offline.

use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSpec {
    pub argv: Vec<String>,
}

impl CommandSpec {
    pub fn new(argv: &[&str]) -> Self {
        Self {
            argv: argv.iter().map(|s| s.to_string()).collect(),
        }
    }

    /// Display form for dry-run logs (quoting is visual only).
    pub fn display(&self) -> String {
        self.argv
            .iter()
            .map(|a| {
                if a.contains(' ') {
                    format!("{a:?}")
                } else {
                    a.clone()
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Execute; a non-zero exit is an error carrying stderr.
    pub fn run(&self) -> Result<(), String> {
        let output = Command::new(&self.argv[0])
            .args(&self.argv[1..])
            .output()
            .map_err(|e| format!("spawn {}: {e}", self.argv[0]))?;
        if output.status.success() {
            Ok(())
        } else {
            Err(format!(
                "{}: exit {:?}: {}",
                self.display(),
                output.status.code(),
                String::from_utf8_lossy(&output.stderr).trim()
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_quotes_only_spaces() {
        let c = CommandSpec::new(&["ip", "-n", "w-1", "addr", "add", "a b"]);
        assert_eq!(c.display(), "ip -n w-1 addr add \"a b\"");
    }

    #[test]
    fn run_reports_nonzero() {
        let c = CommandSpec::new(&["sh", "-c", "exit 3"]);
        assert!(c.run().unwrap_err().contains("exit"));
    }

    #[test]
    fn run_reports_spawn_failure() {
        let c = CommandSpec::new(&["definitely-not-a-binary-xyz", "--flag"]);
        assert!(c.run().is_err());
    }
}
