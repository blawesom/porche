//! `porche` — one binary, role-selected at startup (I-13, doc 12 §3). The PoC
//! implements the **host-agent** role only (doc 07 §2): four verbs over the local
//! desired-state table, shaped for reconciliation (I-12).

use clap::{Parser, Subcommand};
use porche_host::exec::{DirectExecutor, Runner};
use porche_host::volume;
use porche_core::{ops, DesiredStateStore, SqliteStore, WorkloadSpec};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "porche",
    version,
    about = "PORCHE — one binary, role-selected at startup (I-13); PoC role: host-agent (doc 07 §2)"
)]
struct Cli {
    #[command(subcommand)]
    role: Role,
}

#[derive(Subcommand)]
enum Role {
    /// Host-side agent: create/status/stop/wipe over the desired-state table.
    HostAgent {
        /// Desired-state table (SQLite — doc 12 §8, E-27).
        #[arg(long, default_value = "/var/lib/porche/desired-state.sqlite3")]
        store: PathBuf,
        /// Base dir for volume regions, sockets, pid files, console logs, images.
        #[arg(long, default_value = "/var/lib/porche")]
        data_dir: PathBuf,
        /// Mock KMS: one root-only JSON file (doc 07 §2).
        #[arg(long, default_value = volume::DEFAULT_KEYS_FILE)]
        keys_file: PathBuf,
        #[command(subcommand)]
        verb: Verb,
    },
}

#[derive(Subcommand)]
enum Verb {
    /// Converge a workload: netns → volume → VM → running.
    Create {
        /// Workload spec JSON (see src/examples/).
        #[arg(long)]
        spec: PathBuf,
    },
    /// Table view: one workload, or all.
    Status { id: Option<String> },
    /// Stop the VM (volume and netns kept).
    Stop { id: String },
    /// Kill → unmap → wipe (E-6) → netns removal; prints the §7.2 probe report.
    Wipe {
        id: String,
        /// Marker file (one hex string per line) — the §7.2 wipe probe input.
        #[arg(long)]
        markers: Option<PathBuf>,
    },
}

fn hex_decode(s: &str) -> Result<Vec<u8>, String> {
    if s.len() % 2 != 0 || s.is_empty() {
        return Err(format!("bad hex length: {s:?}"));
    }
    (0..s.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&s[i..i + 2], 16).map_err(|e| format!("bad hex {s:?}: {e}"))
        })
        .collect()
}

fn load_markers(path: Option<&PathBuf>) -> porche_core::Markers {
    match path {
        None => Vec::new(),
        Some(p) => {
            let text = std::fs::read_to_string(p)
                .unwrap_or_else(|e| die(format!("markers file {}: {e}", p.display())));
            text.lines()
                .filter(|l| !l.trim().is_empty())
                .map(|l| hex_decode(l.trim()).unwrap_or_else(|e| die(e)))
                .collect()
        }
    }
}

fn die(e: impl std::fmt::Display) -> ! {
    eprintln!("porche: {e}");
    std::process::exit(1)
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let Role::HostAgent {
        store: store_path,
        data_dir,
        keys_file,
        verb,
    } = cli.role;

    let store = SqliteStore::open(&store_path)
        .unwrap_or_else(|e| die(format!("store {}: {e}", store_path.display())));
    let runner = Runner::Shell;
    let mut ex = DirectExecutor::new(runner, &data_dir, &keys_file);

    match verb {
        Verb::Create { spec: path } => {
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| die(format!("spec {}: {e}", path.display())));
            let spec: WorkloadSpec = serde_json::from_str(&text)
                .unwrap_or_else(|e| die(format!("spec {}: {e}", path.display())));
            // Converge on the host for real (netns → volume → VM) and persist the
            // desired state — the verb cycle stays coherent across invocations.
            ops::create(&store, &mut ex, spec).unwrap_or_else(|e| die(e));
            println!("created");
            ExitCode::SUCCESS
        }

        Verb::Status { id } => {
            let records = match id {
                Some(id) => vec![store
                    .get(&id)
                    .unwrap_or_else(|e| die(e))
                    .unwrap_or_else(|| die(format!("workload not found: {id}")))],
                None => store.list().unwrap_or_else(|e| die(e)),
            };
            println!(
                "{:<10} {:<10} {:>5} {:>7}  {:<30} {}",
                "ID", "STATE", "VCPU", "RAM", "IP6", "VOLUME"
            );
            for r in records {
                let vol = r
                    .spec
                    .volume
                    .as_ref()
                    .map(|v| format!("{}M", v.size_mib))
                    .unwrap_or_else(|| "-".into());
                let state = match r.state {
                    porche_core::WorkloadState::Creating => "creating".to_string(),
                    porche_core::WorkloadState::Running => "running".to_string(),
                    porche_core::WorkloadState::Stopping => "stopping".to_string(),
                    porche_core::WorkloadState::Stopped => "stopped".to_string(),
                    porche_core::WorkloadState::Failed { reason } => format!("failed({reason})"),
                };
                println!(
                    "{:<10} {:<10} {:>5} {:>7}  {:<30} {}",
                    r.spec.id, state, r.spec.vcpu, format!("{}M", r.spec.ram_mib), r.spec.ip6, vol
                );
            }
            ExitCode::SUCCESS
        }

        Verb::Stop { id } => {
            ops::stop(&store, &mut ex, &id).unwrap_or_else(|e| die(e));
            println!("stopped");
            ExitCode::SUCCESS
        }

        Verb::Wipe { id, markers } => {
            let markers = load_markers(markers.as_ref());
            let report = ops::wipe(&store, &mut ex, &id, &markers).unwrap_or_else(|e| die(e));
            println!(
                "{}",
                serde_json::to_string_pretty(&report).unwrap_or_default()
            );
            if !report.all_pass() {
                eprintln!("porche: wipe probe FAILED for {id} — §7.2 wipe row");
                return ExitCode::FAILURE;
            }
            println!("wiped");
            ExitCode::SUCCESS
        }
    }
}
