//! Chrome h2 ground-truth rig: real Chromium talks to a raw-boring TLS
//! acceptor, we dump every h2 frame it sends in plaintext.
//!
//! Run: cargo run --example chrome_h2_probe [url-path]
//! Output: JSON on stdout: { frames: [ {type, flags, stream, hex} ... ] }
//! stderr: decoded header list + priority block.
//!
//! This is the capture that parity tests are generated from. NEVER
//! hand-edit the resulting expectations: re-run with a new Chrome and diff.
//! Dev-only rig: the unsafe server code never ships (examples are not
//! part of the release binary).

#[cfg(target_os = "linux")]
use boring_sys as bs;

#[cfg(target_os = "linux")]
use std::net::TcpListener;
#[cfg(target_os = "linux")]
use std::os::fd::AsRawFd;
#[cfg(target_os = "linux")]
use std::process::Command;

/// ALPN select callback: hand h2 back when the client offers it.
#[cfg(target_os = "linux")]
extern "C" fn alpn_select(
    _ssl: *mut bs::SSL,
    out: *mut *const u8,
    out_len: *mut u8,
    input: *const u8,
    input_len: std::os::raw::c_uint,
    _arg: *mut std::os::raw::c_void,
) -> std::os::raw::c_int {
    unsafe {
        let input = std::slice::from_raw_parts(input, input_len as usize);
        let mut off = 0;
        while off < input.len() {
            let l = input[off] as usize;
            if off + 1 + l > input.len() {
                break;
            }
            let proto = &input[off + 1..off + 1 + l];
            if proto == b"h2" {
                *out = proto.as_ptr();
                *out_len = l as u8;
                return 0; // SSL_TLSEXT_ERR_OK: selected
            }
            off += 1 + l;
        }
        3 // SSL_TLSEXT_ERR_NOACK: no overlap, h1 fallback
    }
}

#[cfg(not(target_os = "linux"))]
fn main() {
    donsetch::diag!("chrome_h2_probe is a Linux-only dev rig");
}

#[cfg(target_os = "linux")]
fn main() {
    let root = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".into());
    let cert = format!("{root}/tests/landmarks/h2cert.pem\0");
    let key = format!("{root}/tests/landmarks/h2key.pem\0");

    let listener = TcpListener::bind("127.0.0.1:18443").unwrap();
    let path = std::env::args().nth(1).unwrap_or_else(|| "/".into());

    let browser = std::thread::spawn({
        let path = path.clone();
        move || {
            let url = format!("https://127.0.0.1:18443{path}");
            Command::new("/usr/bin/chromium")
                .args([
                    "--headless=new",
                    "--ignore-certificate-errors",
                    "--no-first-run",
                    "--no-default-browser-check",
                    "--disable-background-networking",
                    "--user-data-dir=/tmp/chrome-h2-probe",
                    "--dump-dom",
                    &url,
                ])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .ok();
        }
    });

    // Server side: raw boring_sys (the shipped boring crate is client-only).
    let ssl_ctx = unsafe { bs::SSL_CTX_new(bs::TLS_server_method()) };
    assert!(!ssl_ctx.is_null());
    assert_eq!(
        unsafe {
            bs::SSL_CTX_use_certificate_file(
                ssl_ctx,
                cert.as_ptr() as *const _,
                bs::SSL_FILETYPE_PEM,
            )
        },
        1
    );
    assert_eq!(
        unsafe {
            bs::SSL_CTX_use_PrivateKey_file(ssl_ctx, key.as_ptr() as *const _, bs::SSL_FILETYPE_PEM)
        },
        1
    );
    unsafe {
        bs::SSL_CTX_set_alpn_select_cb(ssl_ctx, Some(alpn_select), std::ptr::null_mut());
    }

    let (tcp, _) = listener.accept().unwrap();
    let tls = unsafe { bs::SSL_new(ssl_ctx) };
    assert!(!tls.is_null());
    let fd = tcp.as_raw_fd();
    let _ = tcp.set_read_timeout(Some(std::time::Duration::from_secs(10)));
    assert_eq!(unsafe { bs::SSL_set_fd(tls, fd) }, 1);
    unsafe {
        loop {
            let r = bs::SSL_accept(tls);
            if r == 1 {
                break;
            }
            let err = bs::SSL_get_error(tls, r);
            if !(err == bs::SSL_ERROR_WANT_READ || err == bs::SSL_ERROR_WANT_WRITE) {
                panic!("SSL_accept failed: err {err}");
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    let mut buf = Vec::new();
    let mut tmp = [0u8; 8192];
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
    unsafe {
        loop {
            let n = bs::SSL_read(tls, tmp.as_mut_ptr() as *mut _, tmp.len() as i32);
            if n > 0 {
                buf.extend_from_slice(&tmp[..n as usize]);
                if complete(&buf) {
                    break;
                }
            } else {
                let err = bs::SSL_get_error(tls, n);
                if !(err == bs::SSL_ERROR_WANT_READ || err == bs::SSL_ERROR_WANT_WRITE) {
                    break;
                }
            }
            if std::time::Instant::now() > deadline {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
    donsetch::diag!("captured {} bytes", buf.len());
    parse_and_print(&buf);
    decode_and_print(&buf);

    // Minimal answer so Chromium can finish cleanly.
    let body = b"<html><body>ok</body></html>";
    let res = format!(
        "HTTP/1.1 200 OK\r\ncontent-length: {}\r\ncontent-type: text/html\r\n\r\n",
        body.len()
    );
    unsafe {
        let _ = bs::SSL_write(tls, res.as_ptr() as *const _, res.len() as i32);
        let _ = bs::SSL_write(tls, body.as_ptr() as *const _, body.len() as i32);
        bs::SSL_free(tls);
        bs::SSL_CTX_free(ssl_ctx);
    }
    let _ = tcp.set_nonblocking(true);
    std::thread::sleep(std::time::Duration::from_millis(300));
    browser.join().ok();
}

#[cfg(target_os = "linux")]
fn complete(buf: &[u8]) -> bool {
    let mut off = 0usize;
    while off + 9 <= buf.len() {
        let len = u24(&buf[off..off + 3]) as usize;
        let ty = buf[off + 3];
        let flags = buf[off + 4];
        if off + 9 + len > buf.len() {
            return false;
        }
        if ty == 0x1 && flags & 0x1 != 0 {
            return true;
        }
        off += 9 + len;
    }
    false
}

#[cfg(target_os = "linux")]
fn u24(b: &[u8]) -> u32 {
    ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32
}

#[cfg(target_os = "linux")]
fn parse_and_print(buf: &[u8]) {
    let start = if buf.starts_with(b"PRI * HTTP/2.0") {
        24
    } else {
        0
    };
    let mut frames = Vec::new();
    let mut off = start;
    while off + 9 <= buf.len() {
        let len = u24(&buf[off..off + 3]) as usize;
        if off + 9 + len > buf.len() {
            break;
        }
        let (ty, flags, sid) = (
            buf[off + 3],
            buf[off + 4],
            ((buf[off + 5] as u32) << 24)
                | ((buf[off + 6] as u32) << 16)
                | ((buf[off + 7] as u32) << 8)
                | buf[off + 8] as u32,
        );
        let body = &buf[off + 9..off + 9 + len];
        frames.push(serde_json::json!({
            "type": ty,
            "flags": flags,
            "stream": sid,
            "hex": body.iter().map(|x| format!("{x:02x}")).collect::<String>(),
        }));
        off += 9 + len;
    }
    println!("{}", serde_json::json!({ "frames": frames }));
}

/// Decode the first request HEADERS with OUR hpack decoder: proves
/// both Chrome's header list and our decoder on real Chrome bytes.
#[cfg(target_os = "linux")]
fn decode_and_print(buf: &[u8]) {
    let mut off = if buf.starts_with(b"PRI * HTTP/2.0") {
        24
    } else {
        0
    };
    while off + 9 <= buf.len() {
        let len = u24(&buf[off..off + 3]) as usize;
        if buf[off + 3] == 0x1 {
            let body = &buf[off + 9..off + 9 + len];
            if !body.is_empty() && body[4] & 0x20 != 0 {
                donsetch::diag!(
                    "priority: E={} dep={} weight={}",
                    body[0] >> 7,
                    (((body[0] & 0x7f) as u32) << 24)
                        | ((body[1] as u32) << 16)
                        | ((body[2] as u32) << 8)
                        | body[3] as u32,
                    body[4]
                );
            }
            let hpack = if !body.is_empty() && body[0] & 0x80 != 0 {
                &body[5..]
            } else {
                body
            };
            let mut dec = donsetch::transport::h2::hpack::Decoder::new();
            if let Ok(hdrs) = dec.decode(hpack) {
                donsetch::diag!("decoded {} headers:", hdrs.len());
                for (k, v) in hdrs {
                    donsetch::diag!("  {k}: {v}");
                }
            }
            break;
        }
        off += 9 + len;
    }
}
