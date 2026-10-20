# guest/ — the PoC platform image (doc 07 §2, doc 08 action 11)

Build: `make` (fetches pinned sources, builds `out/kernel` + `out/initramfs.cpio.gz`,
writes `out/manifest.txt` — the §7.1 digest fixture).

Components:
- kernel 6.18.57 **bzImage** (CLH boots it via the Linux boot protocol; vmlinux
  lacks the PVH header — observed on v54.0, kept as the MS-1 pin).
- BusyBox 1.36.1 static — the entire rootfs userland.
- `build/init.sh` — platform init (E-25): applies `porche.ip6=<addr>,<gw>` from
  the kernel cmdline, prints `porche.ready` (PERF-1 T1 marker), runs the base64
  `porche.cmd=` workload, prints `porche.done rc=<rc>`, `poweroff -f`.

Digest pinning: `versions.env` holds the sha256 of every fetched source;
`make fetch` verifies, refuses to build on mismatch. The runtime artifact digests
land in `out/manifest.txt`, which MS-1 commits under `runs/<id>/ms1/01-guest-manifest.txt`;
the console-boot copy (`porche.ready` visible on serial) is committed alongside.

MS-1 freeze: no tenant-service stand-ins (`guest/bin/inspect` pruned at E-29) —
workloads are a finite `echo` job or a `sleep`, proof comes from boot markers.