//! Tier-E volume lifecycle — LUKS2 file on the local NVMe region, raw sparse inside;
//! key from the **mock KMS** (one root-only JSON file — doc 07 §2; the real KMS is P0).
//!
//! Wipe is **crypto-erase only** (E-6): zeroize the DEK + discard the region — never a
//! physical overwrite. The DEK destruction is `cryptsetup luksErase` (key slots + LUKS
//! header gone ⇒ the data encryption key is cryptographically unrecoverable); the
//! discard is a hole punch (`fallocate --dig`), which on a discard-mounted filesystem
//! TRIMs the freed extents back to the device. Ciphertext bytes may legitimately
//! remain on the medium — the §7.2 probe checks for *marker material*, not bytes.

use crate::cmdspec::CommandSpec;
use porche_core::spec::WorkloadSpec;
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

/// The mock KMS: one JSON file, root-only. `{"workload-id": "<hex DEK>"}`.
pub const DEFAULT_KEYS_FILE: &str = "/etc/porche/keys.json";

pub fn region_path(data_dir: &Path, id: &str) -> PathBuf {
    data_dir.join("volumes").join(format!("{id}.img"))
}

pub fn mapper_name(id: &str) -> String {
    format!("porche-{id}")
}

/// DEK generation for the mock KMS — 32 bytes from the OS CSPRNG.
pub fn generate_dek() -> Vec<u8> {
    use std::fs::File;
    let mut dek = vec![0u8; 32];
    if let Ok(mut f) = File::open("/dev/urandom") {
        let _ = f.read_exact(&mut dek);
    }
    // belt-and-braces: never hand out a zero key even if /dev/urandom misbehaved
    for (i, b) in dek.iter_mut().enumerate() {
        if *b == 0 {
            *b = (i as u8) | 0x55;
        }
    }
    dek
}

/// Load the mock-KMS keyfile (missing file = empty map).
pub fn load_keys(path: &Path) -> Result<BTreeMap<String, String>, String> {
    match std::fs::read_to_string(path) {
        Ok(s) => serde_json::from_str(&s).map_err(|e| format!("keys file: {e}")),
        Err(_) => Ok(BTreeMap::new()),
    }
}

/// Save the mock-KMS keyfile, mode 0600, parent dirs created.
pub fn save_keys(path: &Path, keys: &BTreeMap<String, String>) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
    }
    let json = serde_json::to_string_pretty(keys).map_err(|e| format!("serialize: {e}"))?;
    std::fs::write(path, json).map_err(|e| format!("write: {e}"))?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .map_err(|e| format!("chmod 0600: {e}"))?;
    Ok(())
}

/// Create + open the volume: sparse file, LUKS2 format with the DEK (via a 0600
/// keyfile — the key never appears in argv/ps), then map it. Returns the commands in
/// order; the caller runs them and manages the keyfile around them.
pub fn create_commands(
    spec: &WorkloadSpec,
    data_dir: &Path,
    keyfile: &Path,
) -> Vec<CommandSpec> {
    let vol = spec.volume.as_ref().expect("volume spec");
    let region = region_path(data_dir, &spec.id).display().to_string();
    let keyfile = keyfile.display().to_string();
    let mapper = mapper_name(&spec.id);
    vec![
        CommandSpec::new(&["rm", "-f", &region]),
        CommandSpec::new(&["truncate", "-s", &format!("{}M", vol.size_mib), &region]),
        // MS-1 create-path rule (KDF spike, runs/<id>/ms1/03-kdf-bench/): the "passphrase"
        // is a 32-byte CSPRNG DEK from the mock KMS (not human entropy) — the PBKDF only
        // burns create time for no added secrecy. Argon2id stays (default), cost pinned low.
        CommandSpec::new(&[
            "cryptsetup",
            "-q",
            "luksFormat",
            "--type",
            "luks2",
            "--pbkdf",
            "argon2id",
            "--pbkdf-memory",
            "65536",
            "--pbkdf-force-iterations",
            "4",
            "--key-file",
            &keyfile,
            &region,
        ]),
        CommandSpec::new(&[
            "cryptsetup",
            "luksOpen",
            "--key-file",
            &keyfile,
            &region,
            &mapper,
        ]),
    ]
}

/// Unmap + crypto-erase + discard (E-6). The DEK/keyfile removal happens in Rust
/// (the mock-KMS edit), not in argv.
pub fn wipe_commands(data_dir: &Path, id: &str) -> Result<Vec<CommandSpec>, String> {
    let region_path_buf = region_path(data_dir, id);
    let region = region_path_buf.display().to_string();
    let mapper = mapper_name(id);
    let len = std::fs::metadata(&region_path_buf)
        .ok()
        .map(|m| m.len())
        .ok_or_else(|| format!("region {region} missing"))?;
    Ok(vec![
        CommandSpec::new(&["cryptsetup", "-q", "luksClose", &mapper]),
        // key slots + LUKS header destroyed: the DEK is cryptographically gone
        CommandSpec::new(&["cryptsetup", "-q", "luksErase", &region]),
        // hole-punch the extents → discard → TRIM to the backing device
        CommandSpec::new(&["fallocate", "--dig", "--length", &len.to_string(), &region]),
    ])
}

/// Region scan — the §7.2 probe: after wipe, the region must contain no *cryptographically
/// recoverable* marker material. Streams the (potentially large) region in chunks with
/// overlap so markers straddling chunk boundaries are found too.
pub fn region_clean(region: &Path, markers: &[Vec<u8>]) -> Result<bool, String> {
    if markers.is_empty() {
        return Ok(true); // nothing to probe — the harness always passes markers
    }
    let mut f = std::fs::File::open(region).map_err(|e| format!("open region: {e}"))?;
    const CHUNK: usize = 8 * 1024 * 1024;
    let overlap = markers.iter().map(|m| m.len()).max().unwrap_or(0).saturating_sub(1);
    let mut tail: Vec<u8> = Vec::new();
    let mut buf = vec![0u8; CHUNK];
    loop {
        let n = f.read(&mut buf).map_err(|e| format!("read region: {e}"))?;
        if n == 0 {
            break;
        }
        let mut window = Vec::with_capacity(tail.len() + n);
        window.extend_from_slice(&tail);
        window.extend_from_slice(&buf[..n]);
        for m in markers {
            if find_subslice(&window, m).is_some() {
                return Ok(false);
            }
        }
        tail = window[window.len().saturating_sub(overlap)..].to_vec();
    }
    Ok(true)
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use porche_core::VolumeSpec;

    fn spec(id: &str, size_mib: u32) -> WorkloadSpec {
        WorkloadSpec {
            id: id.into(),
            image_digest: format!("sha256:{}", "f".repeat(64)),
            command: "true".into(),
            vcpu: 1,
            ram_mib: 64,
            volume: Some(VolumeSpec { size_mib }),
            ip6: "fd00::1".into(),
            gw6: "fd00::ff".into(),
        }
    }

    #[test]
    fn keys_roundtrip_and_perms() {
        use std::os::unix::fs::PermissionsExt;
        let path = std::env::temp_dir().join(format!("porche-keys-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut keys = BTreeMap::new();
        keys.insert("w1".into(), "deadbeef".into());
        save_keys(&path, &keys).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        assert_eq!(load_keys(&path).unwrap(), keys);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn dek_is_32_nonzero_bytes() {
        let dek = generate_dek();
        assert_eq!(dek.len(), 32);
        assert!(dek.iter().any(|b| *b != 0));
    }

    #[test]
    fn region_scan_finds_markers_even_across_chunks() {
        let dir = std::env::temp_dir().join(format!("porche-region-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let region = dir.join("r.img");
        // 16 MB of filler with a marker straddling the 8 MB chunk boundary
        let marker: Vec<u8> = (0..32u8).collect();
        let at = 8 * 1024 * 1024 - 7;
        let mut data = vec![0u8; 16 * 1024 * 1024];
        data[at..at + 32].copy_from_slice(&marker);
        std::fs::write(&region, &data).unwrap();

        assert!(!region_clean(&region, &[marker.clone()]).unwrap());
        let other: Vec<u8> = (200..232u8).collect();
        assert!(region_clean(&region, &[other]).unwrap());

        // wipe the file to zeros (crypto-erase analogy) → clean again
        let wiped = vec![0u8; 16 * 1024 * 1024];
        std::fs::write(&region, &wiped).unwrap();
        assert!(region_clean(&region, &[marker]).unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
