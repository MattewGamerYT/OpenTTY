use std::io::Read;
use std::os::fd::{AsFd, AsRawFd, OwnedFd};
use std::time::{Duration, Instant};

use nix::fcntl::{FcntlArg, OFlag, fcntl};
use nix::poll::{PollFd, PollFlags, PollTimeout, poll};

const MAX_TRANSFER_BYTES: usize = 16 * 1024 * 1024;
const TRANSFER_TIMEOUT: Duration = Duration::from_millis(1200);

pub fn read_fd_with_timeout(fd: OwnedFd) -> Vec<u8> {
    if let Ok(flags) = fcntl(fd.as_raw_fd(), FcntlArg::F_GETFL) {
        let nonblock = OFlag::from_bits_truncate(flags) | OFlag::O_NONBLOCK;
        let _ = fcntl(fd.as_raw_fd(), FcntlArg::F_SETFL(nonblock));
    }

    let mut file = std::fs::File::from(fd);
    let mut out = Vec::new();
    let mut chunk = [0u8; 8192];
    let deadline = Instant::now() + TRANSFER_TIMEOUT;

    loop {
        if out.len() >= MAX_TRANSFER_BYTES {
            break;
        }

        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }

        let timeout = PollTimeout::from((remaining.as_millis().min(200) as u16).max(1));
        let mut fds = [PollFd::new(file.as_fd(), PollFlags::POLLIN)];
        match poll(&mut fds, timeout) {
            Ok(0) => continue,
            Ok(_) => {}
            Err(nix::errno::Errno::EINTR) => continue,
            Err(_) => break,
        }

        loop {
            match file.read(&mut chunk) {
                Ok(0) => return out,
                Ok(n) => {
                    let take = n.min(MAX_TRANSFER_BYTES - out.len());
                    out.extend_from_slice(&chunk[..take]);
                    if out.len() >= MAX_TRANSFER_BYTES {
                        return out;
                    }
                }
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => return out,
            }
        }
    }

    out
}

pub fn pick_text_mime(mimes: &[String]) -> Option<String> {
    for preferred in [
        "text/plain;charset=utf-8",
        "text/plain;charset=UTF-8",
        "UTF8_STRING",
        "text/plain",
        "TEXT",
        "STRING",
    ] {
        if let Some(found) = mimes.iter().find(|m| m.eq_ignore_ascii_case(preferred)) {
            return Some(found.clone());
        }
    }
    mimes
        .iter()
        .find(|m| m.starts_with("text/plain"))
        .cloned()
}

pub fn pick_dnd_mime(mimes: &[String]) -> Option<String> {
    if let Some(uri_list) = mimes
        .iter()
        .find(|m| m.eq_ignore_ascii_case("text/uri-list"))
    {
        return Some(uri_list.clone());
    }
    if let Some(kde_urls) = mimes
        .iter()
        .find(|m| m.eq_ignore_ascii_case("application/x-kde-urilist"))
    {
        return Some(kde_urls.clone());
    }
    pick_text_mime(mimes)
}

/// Parses `text/uri-list` payload into space-separated local filesystem paths (or URIs),
/// shell-quoting entries with spaces or special characters for safe insertion into Fish/Bash.
pub fn parse_uri_list_for_terminal(raw: &[u8], shell_quote: bool) -> String {
    let text = String::from_utf8_lossy(raw);
    let mut items = Vec::new();

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let resolved = if let Some(rest) = trimmed.strip_prefix("file://") {
            let path_part = if let Some(after_local) = rest.strip_prefix("localhost/") {
                format!("/{after_local}")
            } else if rest.starts_with('/') {
                rest.to_string()
            } else {
                // UNC or remote host file://host/path
                format!("//{rest}")
            };
            percent_decode(&path_part)
        } else {
            trimmed.to_string()
        };

        if shell_quote {
            items.push(shell_quote_arg(&resolved));
        } else {
            items.push(resolved);
        }
    }

    items.join(" ")
}

pub fn normalize_pasted_text(raw: &[u8], bracketed_paste: bool) -> Vec<u8> {
    let text = String::from_utf8_lossy(raw);
    let normalized = text.replace("\r\n", "\r").replace('\n', "\r");
    if bracketed_paste {
        // Strip embedded ESC sequences from untrusted clipboard content so it cannot break out of \x1b[201~.
        let safe: String = normalized.chars().filter(|&c| c != '\x1b').collect();
        let mut out = Vec::with_capacity(safe.len() + 12);
        out.extend_from_slice(b"\x1b[200~");
        out.extend_from_slice(safe.as_bytes());
        out.extend_from_slice(b"\x1b[201~");
        out
    } else {
        normalized.into_bytes()
    }
}

pub fn shell_quote_arg(s: &str) -> String {
    if !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'.' | b'_' | b'-' | b'+' | b':'))
    {
        return s.to_string();
    }

    let mut quoted = String::with_capacity(s.len() + 2);
    quoted.push('\'');
    for ch in s.chars() {
        if ch == '\'' {
            quoted.push_str("'\\''");
        } else {
            quoted.push(ch);
        }
    }
    quoted.push('\'');
    quoted
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2]))
        {
            out.push((hi << 4) | lo);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

pub fn decode_base64(input: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity((input.len() * 3) / 4);
    let mut buf = 0u32;
    let mut bits = 0u8;

    for &b in input {
        if b.is_ascii_whitespace() {
            continue;
        }
        if b == b'=' {
            break;
        }
        let val = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        };
        buf = (buf << 6) | val as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((buf >> bits) & 0xFF) as u8);
        }
    }

    Some(out)
}
