# guest/ — platform image build (doc 08 action 11; doc 07 §2)

Minimal guest rootfs for the direct-CLH runtime: **pinned kernel + initramfs +
BusyBox rootfs + platform init**, running the workload command. Pinned by digest —
the signing pipeline is P0 (doc 07 §2); until then the manifest sha256s are the pin,
recorded into the run's fixtures (§7.1).

The guest never configures itself (doc 14 §2 principle 2): the address arrives on
the kernel command line we build (`porche.ip6=<addr>,<gw>` — E-25), applied by the
platform init; the workload command arrives as `porche.cmd=<shell string>`.

## Build

```sh
make fetch          # download pinned sources, verify sha256 (see below)
make kernel         # defconfig + fragment → out/vmlinux + out/kernel
make rootfs         # busybox + init → out/initramfs.cpio.gz
make manifest       # out/manifest.txt — the digest pin (§7.1 fixture)
```

Output lands in `out/`; install into the host-agent image dir:

```sh
D=sha256:$(jq -r '.image_sha256' out/manifest.txt)   # or any stable name you pin
mkdir -p /var/lib/porche/images/$D
cp out/kernel out/initramfs.cpio.gz /var/lib/porche/images/$D/
cp out/initramfs.cpio.gz /var/lib/porche/images/$D/initramfs
```

(For PoC runs the digest *is* the directory name; the catalog object is P0.)

## Pin discipline

`versions.env` records versions and sha256s. `make fetch` **fails** on a sha256
mismatch and refuses an empty pin. To pin a new version: set the version, run
`make fetch record=1` (downloads, records the sha256 into versions.env), **review
the source of the download**, and commit the pin — the record run trusts the first
fetch; the commit is where a human attests it.

## Guest/kernel contract (what the host-agent relies on)

| Key | Meaning | Consumer |
|---|---|---|
| `console=ttyS0` | serial console — the marker channel | harness (PERF-1 T1) |
| `porche.ip6=<addr>,<gw>` | the /128 + gateway (E-25) | init.sh |
| `porche.cmd=<string>` | workload command (sh -c) | init.sh |
| `porche.ready` printed to /dev/console | boot-complete marker | harness |
| `porche.done rc=<n>` printed | command exit | loop tests |
| `/dev/vda` | the tier-E volume (LUKS2 mapper, raw) | workload (markers) |

WS-C verifies the whole contract on the node; the kernel config fragment lists
exactly what the guest needs (virtio-blk/net, v6, serial console, devtmpfs, ACPI
poweroff) — nothing else is enabled on purpose (SEC-3 spirit: minimal surface).
