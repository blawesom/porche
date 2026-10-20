//! Minimal HTTP/1.1 client for the Cloud Hypervisor API over its unix socket.
//!
//! Boring on purpose (doc 12 §4): plain HTTP/1.1 with Content-Length and a JSON body;
//! no TLS (the socket is local and 0600), no async (sync-first core, doc 07 §8.4).
//! One VMM process per VM — the agent spawns `cloud-hypervisor --api-socket ...`
//! (jailed inside the workload netns, doc 14 §2) and talks to it here.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

/// CLH API paths are served under this prefix (v54 openapi). Verified against a live
/// socket during the WS-C burst (doc 07 §8.5) — the unit tests pin the transport, the
/// field names get pinned on the node.
pub const API_PREFIX: &str = "/api/v1";

#[derive(Debug, thiserror::Error)]
pub enum ClhError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("malformed response: {0}")]
    Bad(String),
    #[error("http {code}: {body}")]
    Status { code: u16, body: String },
}

pub struct ClhClient {
    socket: PathBuf,
}

impl ClhClient {
    pub fn new(socket: impl Into<PathBuf>) -> Self {
        Self {
            socket: socket.into(),
        }
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket
    }

    /// Liveness check — used while waiting for a just-spawned VMM to open its socket.
    pub fn vmm_ping(&self) -> Result<(), ClhError> {
        self.request("GET", "/vmm.ping", None).map(|_| ())
    }

    pub fn vm_create(&self, body: &str) -> Result<(), ClhError> {
        self.request("PUT", "/vm.create", Some(body)).map(|_| ())
    }

    pub fn vm_boot(&self) -> Result<(), ClhError> {
        self.request("PUT", "/vm.boot", None).map(|_| ())
    }

    pub fn vm_shutdown(&self) -> Result<(), ClhError> {
        self.request("PUT", "/vm.shutdown", None).map(|_| ())
    }

    /// Remove the VM from the VMM (after shutdown).
    pub fn vm_delete(&self) -> Result<(), ClhError> {
        self.request("PUT", "/vm.delete", None).map(|_| ())
    }

    /// Terminate the VMM process itself (after vm_shutdown + vm_delete).
    pub fn vmm_shutdown(&self) -> Result<(), ClhError> {
        self.request("PUT", "/vmm.shutdown", None).map(|_| ())
    }

    fn request(&self, method: &str, path: &str, body: Option<&str>) -> Result<String, ClhError> {
        let body = body.unwrap_or("");
        let path = format!("{API_PREFIX}{path}");
        let req = format!(
            "{method} {path} HTTP/1.1\r\n\
             Host: localhost\r\n\
             Content-Type: application/json\r\n\
             Content-Length: {}\r\n\
             Connection: close\r\n\
             \r\n\
             {body}",
            body.len()
        );

        let mut stream = UnixStream::connect(&self.socket)?;
        // Fail loudly instead of hanging: the VMM is local — 10 s is already gross.
        stream.set_read_timeout(Some(std::time::Duration::from_secs(10)))?;
        stream.write_all(req.as_bytes())?;

        // Read the response by protocol, NOT to EOF: the VMM keeps the connection
        // open after responding (observed on v54 — an EOF-based read hangs forever).
        let mut buf: Vec<u8> = Vec::new();
        let mut chunk = [0u8; 4096];
        let head_end = loop {
            if let Some(pos) = find_subslice(&buf, b"\r\n\r\n") {
                break pos;
            }
            let n = stream.read(&mut chunk)?;
            if n == 0 {
                return Err(ClhError::Bad("connection closed before headers".into()));
            }
            buf.extend_from_slice(&chunk[..n]);
        };

        let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
        let content_length: usize = head
            .lines()
            .find_map(|l| {
                let (k, v) = l.split_once(':')?;
                k.trim()
                    .eq_ignore_ascii_case("content-length")
                    .then(|| v.trim().parse().ok())
                    .flatten()
            })
            .unwrap_or(0);
        let body_start = head_end + 4;
        while buf.len() < body_start + content_length {
            let n = stream.read(&mut chunk)?;
            if n == 0 {
                break; // short body — surface what we have; status check decides
            }
            buf.extend_from_slice(&chunk[..n]);
        }

        let status_line = head
            .lines()
            .next()
            .ok_or_else(|| ClhError::Bad("empty response".into()))?;
        let code: u16 = status_line
            .split_whitespace()
            .nth(1)
            .and_then(|c| c.parse().ok())
            .ok_or_else(|| ClhError::Bad(format!("bad status line: {status_line}")))?;
        let payload = String::from_utf8_lossy(&buf[body_start..]).into_owned();
        if !(200..300).contains(&code) {
            return Err(ClhError::Status {
                code,
                body: payload,
            });
        }
        Ok(payload)
    }
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::sync::mpsc;

    fn tmp_socket(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("porche-clh-test-{}-{name}.sock", std::process::id()));
        let _ = std::fs::remove_file(&p);
        p
    }

    /// One-shot mock VMM: reads a full request, replies, closes, and reports what it saw.
    fn mock_vmm(path: PathBuf, status: &str, reply_body: &str) -> (mpsc::Receiver<String>, ()) {
        let (tx, rx) = mpsc::channel();
        let listener = UnixListener::bind(&path).expect("bind mock socket");
        let status = status.to_string();
        let reply = reply_body.to_string();
        std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().expect("accept");
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            // read the request head
            let mut head = String::new();
            loop {
                let n = conn.read(&mut chunk).expect("read");
                if n == 0 {
                    break;
                }
                let part = String::from_utf8_lossy(&chunk[..n]).into_owned();
                head.push_str(&part);
                if let Some((h, rest)) = head.clone().split_once("\r\n\r\n") {
                    // enough once we have the announced body
                    let want: usize = h
                        .lines()
                        .find_map(|l| l.split(": ").next().zip(Some(l)))
                        .and_then(|(k, l)| (k.eq_ignore_ascii_case("content-length")).then(|| l))
                        .and_then(|l| l.split(' ').nth(1))
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(0);
                    if rest.len() >= want {
                        buf = head.clone().into_bytes();
                        break;
                    }
                }
            }
            let _ = buf;
            let resp = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n{reply}", reply.len());
            conn.write_all(resp.as_bytes()).unwrap();
            // keep the connection OPEN after responding — the client must complete
            // by Content-Length, never by EOF (the real VMM keeps it open: v54)
            std::thread::sleep(std::time::Duration::from_secs(2));
            tx.send(head).unwrap();
        });
        (rx, ())
    }

    #[test]
    fn vmm_ping_sends_wellformed_request() {
        let path = tmp_socket("ping");
        let (rx, _) = mock_vmm(path.clone(), "204 No Content", "");
        let client = ClhClient::new(&path);
        client.vmm_ping().expect("ping should succeed");
        let seen = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        assert!(seen.starts_with("GET /api/v1/vmm.ping HTTP/1.1\r\n"), "{seen}");
        assert!(seen.contains("Connection: close"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn vm_create_carries_json_body() {
        let path = tmp_socket("create");
        let (rx, _) = mock_vmm(path.clone(), "204 No Content", "");
        let client = ClhClient::new(&path);
        client
            .vm_create(r#"{"cpus":{"boot_vcpus":1}}"#)
            .expect("create should succeed");
        let seen = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        assert!(seen.starts_with("PUT /api/v1/vm.create HTTP/1.1\r\n"), "{seen}");
        assert!(seen.ends_with(r#"{"cpus":{"boot_vcpus":1}}"#));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn error_status_is_reported() {
        let path = tmp_socket("err");
        let (rx, _) = mock_vmm(path.clone(), "400 Bad Request", "{\"error\":\"boom\"}");
        let client = ClhClient::new(&path);
        let err = client.vm_boot().expect_err("should fail");
        assert!(err.to_string().contains("400"), "{err}");
        let _ = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        let _ = std::fs::remove_file(&path);
    }

    // Silence the unused-import lint for UnixStream (used only in the mock).
    #[allow(dead_code)]
    fn _type_check(_s: UnixStream) {}
}
