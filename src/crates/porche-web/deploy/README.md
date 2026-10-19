# porche-web on dev.laplane.net — deployment record (2026-10-19)

Two systemd services run the console-host in **dry mode** (Runner::Dry — the
full UX cycle, nothing executes on the host; live mode needs the metal node):

| Surface | URL | Port | Token file |
|---|---|---|---|
| user console (tenant) | `https://dev.laplane.net/porche/user` | 127.0.0.1:18081 | `/etc/porche-web/user.env` |
| admin console | `https://dev.laplane.net/porche/admin` | 127.0.0.1:18082 | `/etc/porche-web/admin.env` |

- Both share one runtime state: `--data-dir /home/outscale/porche-web`
  (SQLite store + `console-audit.jsonl` + the current platform image digest) —
  the admin audit page reads the same JSONL the user surface writes.
- Tokens are random 128-bit hex in root-only env files; a URL of the form
  `https://dev.laplane.net/porche/user?token=<t>` bootstraps the
  `porche_token` cookie and redirects clean.
- HAProxy fronts both (pass-through — the app is `--base-path` aware); the
  additions live in `haproxy-porche.conf.snippet`, installed with a backup at
  `/etc/haproxy/haproxy.cfg.bak-20261019`.
- Rebuild path: `cargo build --release -p porche-web` then
  `sudo systemctl restart porche-web-user porche-web-admin`.
