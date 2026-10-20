#!/usr/bin/env bash
# MS-1.M4 KDF spike (doc 07 §8.2/§8.6): bench cryptsetup luksFormat param sets for
# the 256 MiB tier-E volume range on node 1's region disk, plus the pre-format-reuse
# variant (one-time Argon2id; luksOpen per cycle). Output: raw.csv (s) on stdout path.
set -euo pipefail
cd /root/poc/ms1/kdfbench
umask 077
[ -f bench.key ] || head -c 32 /dev/urandom > bench.key
CSV=raw.csv
[ -f "$CSV" ] || echo 'ts,variant,volume_mib,action,seconds' > "$CSV"

run() {
    local variant="$1" sz="$2" n="$3" extra="$4"
    for _ in $(seq 1 "$n"); do
        rm -f t.img; truncate -s "${sz}M" t.img
        s=$(date +%s.%N)
        # shellcheck disable=SC2086  # extra is intentionally word-split (param set)
        if ! cryptsetup -q luksFormat --type luks2 --key-file bench.key $extra t.img 2>/tmp/kdf.err; then
            echo "FMTFAIL variant=$variant extra='$extra': $(cat /tmp/kdf.err)" >&2; return 1
        fi
        e=$(date +%s.%N)
        sec=$(awk -v a="$s" -v b="$e" 'BEGIN{printf "%.3f", b-a}')
        echo "$(date -u +%Y-%m-%dT%H:%M:%SZ),$variant,$sz,luksFormat,$sec" >> "$CSV"
    done
}

echo "== baseline default argon2id, 256M x3 =="
run default-argon2id 256 3 ""
echo "== memory 64M default iters, 256M x3 =="
run mem64 256 3 "--pbkdf-memory 65536"
echo "== memory 64M force iters 4 (argon2id minimum), 256M x3 =="
run mem64-iter4 256 3 "--pbkdf-memory 65536 --pbkdf-force-iterations 4"
echo "== memory 256M force iters 4 (RT headroom), 256M x3 =="
run mem256-iter4 256 3 "--pbkdf-force-iterations 4"
echo "== size probe 512M mem64-iter4 x2 =="
run mem64-iter4-512 512 2 "--pbkdf-memory 65536 --pbkdf-force-iterations 4"
echo "== size probe 128M mem64-iter4 x2 =="
run mem64-iter4-128 128 2 "--pbkdf-memory 65536 --pbkdf-force-iterations 4"

echo "== reuse variant: one-time format, then open/close x3 =="
rm -f t.img; truncate -s 256M t.img
s=$(date +%s.%N)
cryptsetup -q luksFormat --type luks2 --key-file bench.key --pbkdf-memory 65536 --pbkdf-force-iterations 4 t.img
e=$(date +%s.%N)
sec=$(awk -v a="$s" -v b="$e" 'BEGIN{printf "%.3f", b-a}')
echo "$(date -u +%Y-%m-%dT%H:%M:%SZ),reuse-onetime,256,luksFormat-once,$sec" >> "$CSV"
for _ in 1 2 3; do
    s=$(date +%s.%N)
    cryptsetup open --key-file bench.key t.img benchmap
    cryptsetup close benchmap
    e=$(date +%s.%N); sec=$(awk -v a="$s" -v b="$e" 'BEGIN{printf "%.3f", b-a}')
    echo "$(date -u +%Y-%m-%dT%H:%M:%SZ),reuse-percycle,256,luksOpen+close,$sec" >> "$CSV"
done

echo "== raw.csv =="
cat "$CSV"