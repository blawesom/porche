# src/ — the PORCHE Rust workspace

One binary, role-selected at startup (I-13, doc 12 §3). The PoC implements the
**host-agent** role (doc 07 §2) — the CLI with four verbs, shaped for
reconciliation (I-12): every verb is a desired-state-table transition plus a
convergence step through the `Executor` trait. The MS-1 console-host
(`porche-web`, E-28/E-29) serves the user and admin consoles over the same
PoC adapter — `Runner::Shell` only: there is no `--dry` surface, ever (E-29).

| Crate | Role |
|---|---|
| `porche-core` | role-neutral core: workload spec, desired-state table (SQLite — doc 12 §8, E-27), verb orchestration (`create/status/stop/wipe`) |
| `porche-clh` | Cloud Hypervisor API client (HTTP/1.1 over the local unix socket — E-4) + the frozen-device `VmConfig` (SEC-3) |
| `porche-host` | `porche host-agent` — the PoC CLI + the direct executor (netns, LUKS2, CLH process) |
| `porche-web` | the MS-1 console-host: `/w` user console + `/a` admin console, two-token AuthN domains (E-8); one binary role-selected with `--surface user\|admin` |

## Build and test

```sh
cargo build                 # debug build
cargo build --release       # release build
cargo test --workspace      # unit tests — offline, no root, no CLH needed
```

## MS-1 console-host (`porche-web`)

Live from node 1 only; nothing is published (no HAProxy, no public binding —
bind stays `127.0.0.1`). Two processes, one per surface role:

| Surface | Flag | Port | Token (flag / env) | Paths |
|---|---|---|---|---|
| user | `--surface user` | **18081** | `--user-token` / `PORCHE_WEB_USER_TOKEN` | `/w`, `/w/new`, `/w/{id}`, `/w/volumes`, `/w/volumes/{id}` |
| admin | `--surface admin` | **18082** | `--admin-token` / `PORCHE_WEB_ADMIN_TOKEN` | `/a`, `/a/host` |

```sh
porche-web --surface user  --store /var/lib/porche/desired-state.sqlite3 \
    --data-dir /var/lib/porche --keys-file /etc/porche/keys.json \
    --user-token "$PORCHE_WEB_USER_TOKEN" --admin-token "$PORCHE_WEB_ADMIN_TOKEN"
porche-web --surface admin --store /var/lib/porche/desired-state.sqlite3 \
    --data-dir /var/lib/porche --keys-file /etc/porche/keys.json \
    --user-token "$PORCHE_WEB_USER_TOKEN" --admin-token "$PORCHE_WEB_ADMIN_TOKEN"
```

Both processes need both tokens so that a cross-domain request is answered with
the contract's strict **401** (not a 404 from the "wrong" binary); `/w` accepts
ONLY the user token, `/a` ONLY the admin token.

Reachability is SSH-loopback forward only (copy-runnable):

```sh
ssh -L 127.0.0.1:18081:127.0.0.1:18081 -L 127.0.0.1:18082:127.0.0.1:18082 outscale@node1
```

then `http://127.0.0.1:18081/w` (user) and `http://127.0.0.1:18082/a` (admin).
Any unauthenticated request — on either surface — is answered with a direct
**401 HTML page** (the surface's login form; no redirect workaround): sign in
by submitting the token (GET `?token=` sets the `porche_token` cookie and 303s
to the clean path) or presenting `Authorization: Bearer <token>` on any page.
The wrong-domain token is the same strict 401 (UAT negative check `n1`).

### UAT (MS-1.M6)

User-acceptance screenshots upload path:

```text
runs/20261020-M1/ms1/uat/*.png
```

Every page/operation is walked through a live browser and recorded in
`runs/20261020-M1/ms1/user-test.md` (the milestone is not complete without the
operator-signed user test).

### State derivation — `done (rc=N)` (MS-1 rule)

The console display state `done (rc=N)` is **derived from observable signals
only**: the guest's console marker `porche.done rc=N` in
`$DATA_DIR/consoles/{id}.log` **AND** the workload's VM API socket being gone
(`$DATA_DIR/run/{id}.sock`) — never from the store enum alone (a `running` row
can be observably done the moment the job finished and powered off, see
`runs/20261020-M1/ms1/02-create/results.md`). An ordered `StopWorkload` stays
`stopped`; `reboot: Power down` with a gone socket and no done marker yields
`done (rc=?)`. The Events timeline is the store's `created_at`/`updated_at`
transitions plus the console markers `porche.ready`, `porche.done rc=N` and
`reboot: Power down`; marker wall-times are clearly marked `≈` estimates
anchored on the kernel's monotonic `[secs]` prefixes and the log file mtime.