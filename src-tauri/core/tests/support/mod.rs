//! What the end-to-end tests share: files served over a real HTTPS socket on
//! loopback, the repository root, and a folder read back as bytes.

#![allow(dead_code)]

use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use sha2::Digest;

// ------------------------------------------------------------ HTTPS fixture

/// Files served by path over TLS, and how often each was asked for.
#[derive(Clone, Default)]
pub struct Served {
    pub files: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    pub hits: Arc<Mutex<HashMap<String, usize>>>,
}

impl Served {
    pub fn put(&self, path: &str, body: Vec<u8>) {
        self.files.lock().unwrap().insert(path.to_string(), body);
    }

    pub fn hits(&self, path: &str) -> usize {
        self.hits.lock().unwrap().get(path).copied().unwrap_or(0)
    }

    pub fn total(&self) -> usize {
        self.hits.lock().unwrap().values().sum()
    }
}

/// A self-signed certificate made by openssl, as DER (cert, PKCS#8 key).
pub fn certificate(dir: &Path) -> (Vec<u8>, Vec<u8>) {
    let run = |args: &[&str]| {
        let out = std::process::Command::new("openssl")
            .args(args)
            .current_dir(dir)
            .output()
            .expect("openssl runs");
        assert!(
            out.status.success(),
            "openssl {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    };

    run(&[
        "req",
        "-x509",
        "-newkey",
        "ec",
        "-pkeyopt",
        "ec_paramgen_curve:prime256v1",
        "-nodes",
        "-keyout",
        "key.pem",
        "-out",
        "cert.der",
        "-outform",
        "DER",
        "-days",
        "1",
        "-subj",
        "/CN=127.0.0.1",
    ]);
    run(&[
        "pkcs8", "-topk8", "-nocrypt", "-in", "key.pem", "-outform", "DER", "-out", "key.der",
    ]);

    (
        std::fs::read(dir.join("cert.der")).unwrap(),
        std::fs::read(dir.join("key.der")).unwrap(),
    )
}

/// Serve `served` over HTTPS on loopback; returns the port.
pub fn serve(dir: &Path, served: Served) -> u16 {
    use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

    let (cert, key) = certificate(dir);
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .expect("tls versions")
    .with_no_client_auth()
    .with_single_cert(
        vec![CertificateDer::from(cert)],
        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key)),
    )
    .expect("certificate");
    let config = Arc::new(config);

    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();

    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { return };
            let config = config.clone();
            let served = served.clone();

            std::thread::spawn(move || {
                let Ok(conn) = rustls::ServerConnection::new(config) else {
                    return;
                };
                let mut tls = rustls::StreamOwned::new(conn, stream);
                let mut request = Vec::new();
                let mut buf = [0u8; 2048];

                while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                    match tls.read(&mut buf) {
                        Ok(0) | Err(_) => return,
                        Ok(n) => request.extend_from_slice(&buf[..n]),
                    }
                }

                let line = String::from_utf8_lossy(&request);
                let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
                let body = served.files.lock().unwrap().get(&path).cloned();
                *served.hits.lock().unwrap().entry(path).or_default() += 1;

                let (status, body) = match body {
                    Some(b) => ("200 OK", b),
                    None => ("404 Not Found", Vec::new()),
                };
                let mut response = format!(
                    "HTTP/1.1 {status}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                    body.len()
                )
                .into_bytes();
                response.extend_from_slice(&body);

                let _ = tls.write_all(&response);
                tls.conn.send_close_notify();
                let _ = tls.flush();
            });
        }
    });

    port
}

pub fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

pub fn sha(bytes: &[u8]) -> String {
    hex::encode(sha2::Sha256::digest(bytes))
}

pub fn pin(port: u16, path: &str, body: &[u8]) -> Value {
    json!({ "url": format!("https://127.0.0.1:{port}{path}"), "sha256": sha(body), "size": body.len() })
}

/// Every file under `root`, symlinks read through, as (path, bytes).
pub fn snapshot(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(dir: &Path, base: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            let meta = std::fs::symlink_metadata(&path).unwrap();
            let rel = path
                .strip_prefix(base)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            if meta.is_dir() {
                walk(&path, base, out);
            } else {
                out.insert(
                    rel,
                    std::fs::read(&path).unwrap_or_else(|_| b"<dangling>".to_vec()),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}
