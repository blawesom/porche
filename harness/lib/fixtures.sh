#!/usr/bin/env bash
# §7.1 fixture pinning: record what the run executed on, before anything else.
# Usage: source harness/lib/fixtures.sh && fixtures_record "$RUN_DIR"
set -u

fixtures_record() {
    local run_dir=$1
    # .../harness/lib/fixtures.sh → harness dir and repo root
    local harness_dir repo_root
    harness_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
    repo_root=$(cd "$harness_dir/.." && pwd)

    # platform image manifest: repo layout, or the node layout (node-setup.sh
    # drops it at /root/poc/manifest.txt)
    local manifest="$repo_root/guest/out/manifest.txt"
    [ -f "$manifest" ] || manifest=/root/poc/manifest.txt

    # CLH: PATH may not include /usr/local/bin (root secure_path)
    local clh_bin
    clh_bin=$(command -v cloud-hypervisor 2>/dev/null || echo /usr/local/bin/cloud-hypervisor)

    {
        echo "# fixtures (doc 07 §7.1) — $(date -u +%Y-%m-%dT%H:%M:%SZ)"
        echo
        echo "## Node OS + kernel"
        uname -r
        grep PRETTY_NAME /etc/os-release 2>/dev/null || echo "os-release: n/a"
        echo
        echo "## CLH"
        "$clh_bin" --version 2>/dev/null || echo "cloud-hypervisor: NOT FOUND"
        echo
        echo "## Guest image (action 11 artifacts)"
        cat "$manifest" 2>/dev/null || echo "guest manifest missing — build guest/ first"
        echo
        echo "## host-agent"
        echo "commit: $(git -C "$repo_root" rev-parse HEAD 2>/dev/null || echo unknown)"
        echo "binary: ${PORCHE_BIN:-porche (PATH)}"
        echo
        echo "## Hardening state (SEC-11 assert)"
        "$harness_dir/sysctl-assert.sh" 2>&1
        echo
        echo "## Offload / device allow-list state"
        echo "offloads_off: true (VmConfig default — SEC-7; delta run flips it)"
        echo "device surface: frozen (porche-clh VmConfig — SEC-3)"
    } > "$run_dir/fixtures.txt"

    echo "fixtures recorded: $run_dir/fixtures.txt"
}
