#!/usr/bin/env bash
# §7.1 fixture pinning: record what the run executed on, before anything else.
# Usage: source harness/lib/fixtures.sh && fixtures_record "$RUN_DIR"
set -u

fixtures_record() {
    local run_dir=$1
    local repo_root
    repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)

    {
        echo "# fixtures (doc 07 §7.1) — $(date -u +%Y-%m-%dT%H:%M:%SZ)"
        echo
        echo "## Node OS + kernel"
        uname -r
        grep PRETTY_NAME /etc/os-release 2>/dev/null || echo "os-release: n/a"
        echo
        echo "## CLH"
        cloud-hypervisor --version 2>/dev/null \
            || command -v cloud-hypervisor >/dev/null \
            || echo "cloud-hypervisor: NOT FOUND"
        echo
        echo "## Guest image (action 11 artifacts)"
        cat "$repo_root/guest/out/manifest.txt" 2>/dev/null \
            || echo "guest manifest missing — build guest/ first"
        echo
        echo "## host-agent"
        echo "commit: $(git -C "$repo_root" rev-parse HEAD 2>/dev/null || echo unknown)"
        echo "binary: ${PORCHE_BIN:-porche (PATH)}"
        echo
        echo "## Hardening state (SEC-11 assert)"
        "$repo_root/harness/sysctl-assert.sh" 2>&1
        echo
        echo "## Offload / device allow-list state"
        echo "offloads_off: true (VmConfig default — SEC-7; delta run flips it)"
        echo "device surface: frozen (porche-clh VmConfig — SEC-3)"
    } > "$run_dir/fixtures.txt"

    echo "fixtures recorded: $run_dir/fixtures.txt"
}
