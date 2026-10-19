# src/ — the PORCHE Rust workspace

One binary, role-selected at startup (I-13, doc 12 §3). The PoC implements the
**host-agent** role only (doc 07 §2) — the CLI with four verbs, shaped for
reconciliation (I-12): every verb is a desired-state-table transition plus a
convergence step through the `Executor` trait.

| Crate | Role |
|---|---|
| `porche-core` | role-neutral core: workload spec, desired-state table (SQLite — doc 12 §8, E-27), verb orchestration (`create/status/stop/wipe`) |
| `porche-clh` | Cloud Hypervisor API client (HTTP/1.1 over the local unix socket — E-4) + the frozen-device `VmConfig` (SEC-3) |
| `porche-host` | `porche host-agent` — the PoC CLI + the direct executor (netns, LUKS2, CLH process) |

## Build and test

```sh
cargo build            # debug build
cargo test             # unit tests — offline, no root, no CLH needed
cargo run -- host-agent --store /tmp/porche.sqlite3 --data-dir /tmp/porche \
    --dry-run create --spec examples/w1.json   # the full pipeline, printed not executed
```

## Design notes (ratified in doc 07 §8.4)

- **Sync-first core**: the PoC verbs are synchronous; tokio/tonic arrive with the P0
  long-running agent (doc 12 §4). What must survive into P0 is the shape — store +
  verbs + executor seam — not the async runtime.
- **Boring dependencies only** (doc 12 §4): serde, rusqlite (bundled), clap, thiserror.
- **Test-first** (doc 07 §5): orchestration, store, CLH client transport and every
  command argv are unit-tested offline; the on-node integration (real CLH, real
  cryptsetup, real netns) is the WS-C burst work (doc 07 §8.5).
- The CLH API JSON field names mirror the v54 openapi subset we use; they are
  **verified against a live socket during WS-C** (marked in `porche-clh/src/config.rs`).
