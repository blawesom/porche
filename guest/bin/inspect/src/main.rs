//! inspect — the live-workload backend (PoC stand-in for a tenant-visible
//! service; doc 07). A std-only HTTP/1.0 server: every `GET /` reads /proc at
//! request time and answers JSON — kernel, cmdline (the workload's own
//! configuration: `porche.ip6`, `porche.cmd`), vCPUs, memory, IPv6 addresses,
//! mounts, block devices, uptime. No dependencies, no caching, one accept loop.

use std::io::{Read, Write};
use std::net::{TcpListener, ToSocketAddrs};

fn read_trim(path: &str) -> String {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn read_lines(path: &str) -> Vec<String> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' | '\r' | '\t' => out.push(' '),
            c if (c as u32) < 0x20 => {}
            c => out.push(c),
        }
    }
    out
}

/// 32 hex chars → 8 colon groups (the /proc/net/if_inet6 address format).
fn format_inet6_hex(hex: &str) -> Option<String> {
    if hex.len() != 32 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let mut groups = Vec::with_capacity(8);
    for i in (0..32).step_by(4) {
        groups.push(&hex[i..i + 4]);
    }
    Some(groups.join(":"))
}

fn v6_addrs() -> Vec<(String, String)> {
    let mut out = Vec::new();
    for l in read_lines("/proc/net/if_inet6") {
        let f: Vec<&str> = l.split_whitespace().collect();
        if f.len() < 6 {
            continue;
        }
        if let Some(addr) = format_inet6_hex(f[0]) {
            out.push((f[5].to_string(), addr));
        }
    }
    out
}

/// The live snapshot — everything read NOW, one request = one read.
fn snapshot_json() -> String {
    let kernel = json_escape(&read_trim("/proc/version"));
    let cmdline = json_escape(&read_trim("/proc/cmdline"));
    let uptime = json_escape(
        &read_trim("/proc/uptime")
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .to_string(),
    );

    let mut vcpu_count = 0usize;
    let mut model = String::new();
    for l in read_lines("/proc/cpuinfo") {
        if let Some(v) = l.strip_prefix("processor") {
            if v.starts_with(':') {
                vcpu_count += 1;
            }
        } else if model.is_empty() {
            if let Some(v) = l.strip_prefix("model name") {
                if v.starts_with(':') {
                    model = v[1..].trim().to_string();
                }
            }
        }
    }

    let mut mem_total = 0u64;
    let mut mem_avail = 0u64;
    for l in read_lines("/proc/meminfo") {
        let (k, v) = match l.split_once(':') {
            Some((k, v)) => (k, v.trim()),
            None => continue,
        };
        let kb = v.split_whitespace().next().and_then(|n| n.parse().ok()).unwrap_or(0);
        match k {
            "MemTotal" => mem_total = kb,
            "MemAvailable" => mem_avail = kb,
            _ => {}
        }
    }

    let mut mounts = Vec::new();
    for l in read_lines("/proc/mounts") {
        let f: Vec<&str> = l.split_whitespace().collect();
        if f.len() >= 3 {
            mounts.push(format!(
                "{{\"source\":\"{}\",\"mountpoint\":\"{}\",\"fstype\":\"{}\"}}",
                json_escape(f[0]),
                json_escape(f[1]),
                json_escape(f[2])
            ));
        }
    }

    let mut blocks = Vec::new();
    if let Ok(entries) = std::fs::read_dir("/sys/block") {
        let mut names: Vec<String> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        for n in names {
            let sectors = read_trim(&format!("/sys/block/{n}/size"))
                .parse::<u64>()
                .unwrap_or(0);
            let dev_model = json_escape(&read_trim(&format!("/sys/block/{n}/device/model")));
            blocks.push(format!(
                "{{\"dev\":\"{}\",\"size_kb\":{},\"model\":\"{}\"}}",
                json_escape(&n),
                sectors * 512 / 1024,
                dev_model
            ));
        }
    }

    let mut addrs = Vec::new();
    for (dev, addr) in v6_addrs() {
        addrs.push(format!(
            "{{\"dev\":\"{}\",\"addr\":\"{}\"}}",
            json_escape(&dev),
            json_escape(&addr)
        ));
    }

    format!(
        "{{\"porche\":\"live\",\
         \"kernel\":\"{kernel}\",\"cmdline\":\"{cmdline}\",\"uptime_s\":\"{uptime}\",\
         \"cpu\":{{\"count\":{vcpu_count},\"model\":\"{}\"}},\
         \"memory\":{{\"total_kb\":{mem_total},\"available_kb\":{mem_avail}}},\
         \"addrs\":[{}],\"mounts\":[{}],\"block\":[{}]}}",
        json_escape(&model),
        addrs.join(","),
        mounts.join(","),
        blocks.join(",")
    )
}

fn respond(stream: &mut std::net::TcpStream, code: &str, body: &str) {
    let resp = format!(
        "HTTP/1.0 {code}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(resp.as_bytes());
    let _ = stream.shutdown(std::net::Shutdown::Write);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (addr, port) = match args.as_slice() {
        [_, a, p] => (a.clone(), p.clone()),
        _ => {
            eprintln!("usage: inspect <bind-ip6> <port>  (the workload's own /128 — E-25)");
            std::process::exit(2);
        }
    };
    let bind = format!("[{addr}]:{port}");
    let listener = match bind
        .to_socket_addrs()
        .ok()
        .and_then(|mut i| i.next())
        .and_then(|a| TcpListener::bind(a).ok())
    {
        Some(l) => l,
        None => {
            eprintln!("inspect: cannot bind {bind}");
            std::process::exit(1);
        }
    };
    eprintln!("inspect: serving live snapshot on {bind}");

    for conn in listener.incoming() {
        let mut stream = match conn {
            Ok(s) => s,
            Err(_) => continue,
        };
        let mut buf = [0u8; 1024];
        let n = stream.read(&mut buf).unwrap_or(0);
        let first_line = String::from_utf8_lossy(&buf[..n])
            .lines()
            .next()
            .unwrap_or_default()
            .to_string();
        let path = first_line
            .split_whitespace()
            .nth(1)
            .unwrap_or_default()
            .to_string();
        if path == "/" || path.is_empty() {
            respond(&mut stream, "200 OK", &snapshot_json());
        } else {
            respond(&mut stream, "404 Not Found", "{\"error\":\"not found\"}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inet6_hex_formats_like_proc() {
        assert_eq!(
            format_inet6_hex("fd420001000100000000000000000001").as_deref(),
            Some("fd42:0001:0001:0000:0000:0000:0000:0001")
        );
        assert_eq!(format_inet6_hex("xyz"), None);
        assert_eq!(format_inet6_hex(""), None);
    }

    #[test]
    fn json_escape_neutralizes_quotes_and_control() {
        assert_eq!(json_escape("a\"b\\c"), "a\\\"b\\\\c");
        assert_eq!(json_escape("l1\nl2\t"), "l1 l2 ");
        assert_eq!(json_escape("\u{1}"), "");
    }

    #[test]
    fn snapshot_is_valid_json_with_the_live_keys() {
        // runs on the build host: /proc exists, /sys/block exists — the shape
        // is what matters, not the values
        let s = snapshot_json();
        for key in [
            "\"porche\":\"live\"",
            "\"kernel\":",
            "\"cmdline\":",
            "\"cpu\":{",
            "\"memory\":{",
            "\"addrs\":[",
            "\"mounts\":[",
            "\"block\":[",
        ] {
            assert!(s.contains(key), "missing {key} in {s}");
        }
    }
}
