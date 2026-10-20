//! The direct executor: runs the real host-side operations (netns, LUKS2, CLH).
//! MS-1 (E-29): `Runner::Dry` and `--dry-run` do not return — the executor is
//! real-execution only (Runner::Shell); the proof is the exit runs, not prints.
//! The CLH process is spawned jailed inside the workload netns (doc 14 §2).

use crate::cmdspec::CommandSpec;
use crate::{netns, volume};
use porche_clh::config::build_cmdline;
use porche_clh::{ClhClient, VmConfig};
use porche_core::spec::WorkloadSpec;
use porche_core::{ExecuteError, Executor, Markers, WipeReport};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Real-executor split (kept named per doc 07 §8.1): only `Shell` exists as of
/// MS-1 — there is no dry surface, ever (E-29 milestone rule).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Runner {
    Shell,
}

pub struct DirectExecutor {
    pub runner: Runner,
    pub data_dir: PathBuf,
    pub keys_file: PathBuf,
}

impl DirectExecutor {
    pub fn new(
        runner: Runner,
        data_dir: impl Into<PathBuf>,
        keys_file: impl Into<PathBuf>,
    ) -> Self {
        Self {
            runner,
            data_dir: data_dir.into(),
            keys_file: keys_file.into(),
        }
    }

    fn run(&self, c: &CommandSpec) -> Result<(), String> {
        let Runner::Shell = self.runner;
        c.run()
    }

    fn ensure_bridge(&self) -> Result<(), String> {
        let show = CommandSpec::new(&["ip", "link", "show", netns::BRIDGE]);
        if show.run().is_err() {
            for c in [
                CommandSpec::new(&["ip", "link", "add", netns::BRIDGE, "type", "bridge"]),
                CommandSpec::new(&["ip", "link", "set", netns::BRIDGE, "up"]),
            ] {
                self.run(&c)?;
            }
        }
        Ok(())
    }

    fn keyfile_path(&self, id: &str) -> PathBuf {
        self.data_dir.join("run").join(format!("{id}.key"))
    }

    fn vm_socket(&self, id: &str) -> PathBuf {
        self.data_dir.join("run").join(format!("{id}.sock"))
    }

    fn vm_pid_file(&self, id: &str) -> PathBuf {
        self.data_dir.join("run").join(format!("{id}.pid"))
    }

    fn console_path(&self, id: &str) -> PathBuf {
        self.data_dir.join("consoles").join(format!("{id}.log"))
    }

    fn image_dir(&self, digest: &str) -> PathBuf {
        self.data_dir.join("images").join(digest)
    }

    fn kill_pid_file(&self, id: &str) {
        let pid_file = self.vm_pid_file(id);
        if let Ok(pid) = std::fs::read_to_string(&pid_file) {
            if let Ok(pid) = pid.trim().parse::<i32>() {
                // SIGKILL: deterministic teardown outranks graceful shutdown here —
                // vm_shutdown was already attempted in stop_vm.
                let _ = Command::new("kill").args(["-9", &pid.to_string()]).output();
            }
        }
        let _ = std::fs::remove_file(&pid_file);
    }
}

/// The CLH binary — absolute path: `ip netns exec` resolves via execvp under the
/// spawning user's PATH, and root's secure_path does not include /usr/local/bin
/// (observed on Rocky 9 — the child dies instantly, the socket never appears).
fn clh_binary() -> String {
    std::env::var("PORCHE_CLH_BIN").unwrap_or_else(|_| "/usr/local/bin/cloud-hypervisor".into())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn wait_for_socket(client: &ClhClient, timeout: Duration) -> Result<(), ExecuteError> {
    let start = Instant::now();
    loop {
        if client.vmm_ping().is_ok() {
            return Ok(());
        }
        if start.elapsed() > timeout {
            return Err(ExecuteError::Vm(
                "CLH api socket did not come up".into(),
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn vol_err(e: String) -> ExecuteError {
    ExecuteError::Volume(e)
}

impl Executor for DirectExecutor {
    fn setup_netns(&mut self, spec: &WorkloadSpec) -> Result<(), ExecuteError> {
        self.ensure_bridge()
            .map_err(|e| ExecuteError::Netns(format!("bridge: {e}")))?;
        // idempotent pre-clean: a prior wipe or failed create can leave the root-side
        // veth (netns deletion detaches, not deletes it) — treat absence as the
        // precondition so `ip link add` never hits "File exists" on re-create.
        let _ = CommandSpec::new(&["ip", "netns", "del", &netns::ns_name(&spec.id)]).run();
        let _ = CommandSpec::new(&["ip", "link", "del", &netns::veth_root(&spec.id)]).run();
        for c in netns::commands(spec) {
            self.run(&c)
                .map_err(|e| ExecuteError::Netns(format!("{}: {e}", c.display())))?;
        }
        Ok(())
    }

    fn create_volume(&mut self, spec: &WorkloadSpec) -> Result<(), ExecuteError> {
        if spec.volume.is_none() {
            return Ok(()); // volume-less workload
        }
        let keyfile = self.keyfile_path(&spec.id);
        let commands = volume::create_commands(spec, &self.data_dir, &keyfile);

        // DEK into the mock KMS, keyfile (0600), format + open, keyfile gone
        let dek = volume::generate_dek();
        let mut keys = volume::load_keys(&self.keys_file).map_err(vol_err)?;
        keys.insert(spec.id.clone(), hex(&dek));
        volume::save_keys(&self.keys_file, &keys).map_err(vol_err)?;

        if let Some(parent) = keyfile.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| vol_err(format!("mkdir: {e}")))?;
        }
        // The DEK never appears in argv (cryptsetup reads it from the 0600 file).
        std::fs::write(&keyfile, &dek).map_err(|e| vol_err(format!("keyfile: {e}")))?;
        let result = (|| -> Result<(), ExecuteError> {
            for c in &commands {
                self.run(c).map_err(vol_err)?;
            }
            Ok(())
        })();

        // keyfile hygiene either way: zero it, then remove it
        let _ = std::fs::write(&keyfile, vec![0u8; dek.len()]);
        let _ = std::fs::remove_file(&keyfile);
        result
    }

    fn start_vm(&mut self, spec: &WorkloadSpec) -> Result<(), ExecuteError> {
        let ns = netns::ns_name(&spec.id);
        let sock = self.vm_socket(&spec.id);
        let console = self.console_path(&spec.id);
        let image = self.image_dir(&spec.image_digest);

        let cfg = VmConfig {
            vcpu: spec.vcpu,
            ram_mib: spec.ram_mib,
            kernel_path: image.join("kernel").display().to_string(),
            initramfs_path: image.join("initramfs").display().to_string(),
            cmdline: build_cmdline(spec),
            disks: match &spec.volume {
                Some(_) => vec![format!("/dev/mapper/{}", volume::mapper_name(&spec.id))],
                None => vec![],
            },
            tap: netns::tap_name(&spec.id),
            mac: spec.assigned_mac(),
            offloads_off: true, // SEC-7: default for untrusted workloads
            console_log: console.display().to_string(),
        };

        for p in [&image.join("kernel"), &image.join("initramfs")] {
            if !p.exists() {
                return Err(ExecuteError::Vm(format!(
                    "platform image artifact missing: {} (digest {})",
                    p.display(),
                    spec.image_digest
                )));
            }
        }
        for dir in [sock.parent(), console.parent()] {
            if let Some(d) = dir {
                std::fs::create_dir_all(d)
                    .map_err(|e| ExecuteError::Vm(format!("mkdir: {e}")))?;
            }
        }

        // The VMM runs jailed inside the workload netns (doc 14 §2: process boundary
        // and network boundary coincide). Absolute binary path — see clh_binary().
        let child = Command::new("ip")
            .args(["netns", "exec", &ns])
            .arg(clh_binary())
            .arg("--api-socket")
            .arg(&sock)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| ExecuteError::Vm(format!("spawn CLH: {e}")))?;
        std::fs::write(self.vm_pid_file(&spec.id), child.id().to_string())
            .map_err(|e| ExecuteError::Vm(format!("pid file: {e}")))?;

        let client = ClhClient::new(&sock);
        wait_for_socket(&client, Duration::from_secs(5))?;
        client
            .vm_create(&cfg.to_clh_json())
            .map_err(|e| ExecuteError::Vm(format!("vm.create: {e}")))?;
        client
            .vm_boot()
            .map_err(|e| ExecuteError::Vm(format!("vm.boot: {e}")))?;
        Ok(())
    }

    fn stop_vm(&mut self, id: &str) -> Result<(), ExecuteError> {
        let sock = self.vm_socket(id);
        if sock.exists() {
            let client = ClhClient::new(&sock);
            // clean shutdown, then remove the VM from the VMM, then the VMM exits
            let _ = client.vm_shutdown();
            let _ = client.vm_delete();
            let _ = client.vmm_shutdown();
        }
        // belt and braces: the pid must be gone before wipe touches the volume
        self.kill_pid_file(id);
        let _ = std::fs::remove_file(&sock);
        Ok(())
    }

    fn teardown(&mut self, id: &str, markers: &Markers) -> Result<WipeReport, ExecuteError> {
        // 1. kill VM → unmap → wipe (FR-3 order). Best-effort stop first.
        self.stop_vm(id)?;

        let mut report = WipeReport {
            mapper_absent: true,
            keyfile_entry_removed: true,
            keyslots_erased: true,
            region_trimmed: true,
            region_clean: true,
        };
        let region = volume::region_path(&self.data_dir, id);

        // 2. volume crypto-erase — only if a region exists (volume-less workloads and
        //    pre-volume failures have nothing to wipe).
        if region.exists() {
            let cmds = volume::wipe_commands(&self.data_dir, id)
                .map_err(ExecuteError::Teardown)?;
            // unmap (idempotent: absence is what we assert)
            let _ = self.run(&cmds[0]);
            report.mapper_absent = !Path::new(&format!(
                "/dev/mapper/{}",
                volume::mapper_name(id)
            ))
            .exists();
            // crypto-erase: key slots + LUKS header — the DEK is gone (E-6)
            report.keyslots_erased = self.run(&cmds[1]).is_ok();
            // discard: hole punch → TRIM to the backing device
            report.region_trimmed = self.run(&cmds[2]).is_ok();
        }

        // 3. mock-KMS entry removal
        match volume::load_keys(&self.keys_file) {
            Ok(mut keys) => {
                if keys.remove(id).is_some() {
                    volume::save_keys(&self.keys_file, &keys)
                        .map_err(|e| ExecuteError::Teardown(format!("keys: {e}")))?;
                }
            }
            Err(_) => report.keyfile_entry_removed = false,
        }

        // 4. region scan — the §7.2 probe
        if region.exists() && !markers.is_empty() {
            report.region_clean = volume::region_clean(&region, markers)
                .map_err(ExecuteError::Teardown)?;
        }

        // 5. netns removal — absence is the desired state (idempotent; a create
        //    that failed before the netns existed must not fail its rollback).
        //    remove_commands now also clears the root-side veth (netns deletion
        //    detaches it, never deletes it) — tolerate "not found" on each.
        let listing = CommandSpec::new(&["ip", "netns", "list"])
            .run_output()
            .unwrap_or_default();
        if listing.lines().any(|l| l.starts_with(&netns::ns_name(id))) {
            for c in netns::remove_commands(id) {
                // removal runs tolerate "not found" (absence is the desired state)
                let _ = self.run(&c);
            }
        } else {
            // netns already gone — still clear a detached root veth leftover
            let _ = CommandSpec::new(&["ip", "link", "del", &netns::veth_root(id)]).run();
        }
        Ok(report)
    }
}
