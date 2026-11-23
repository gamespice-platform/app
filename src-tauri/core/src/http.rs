//! The one place an HTTP client is started.
//!
//! reqwest 0.13's `rustls-no-provider` build panics at client construction
//! unless a rustls crypto provider is installed first. The provider here is
//! ring (the `rustls` dependency enables only that feature), because reqwest's
//! own `rustls` feature would bring `aws-lc-sys`, a C/cmake build on every
//! target. Every client goes through [`builder`] or [`client`] so the install
//! cannot be forgotten.

use std::sync::Once;

fn ensure_provider() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        // Err means another provider is already installed, which is fine.
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

/// `reqwest::Client::builder()` with the TLS provider installed.
pub fn builder() -> reqwest::ClientBuilder {
    ensure_provider();
    let builder = reqwest::Client::builder();
    #[cfg(target_os = "android")]
    let builder = builder.tls_backend_preconfigured(android_tls_config());
    builder
}

/// The rustls config every Android client uses.
///
/// reqwest's default verifier is rustls-platform-verifier, which on Android
/// reaches the JVM for the system trust store and `expect`s that the app handed
/// it a JNI context first. Nothing here does (that takes `JNI_OnLoad` plus the
/// verifier's Kotlin component in the Gradle project), so the first TLS
/// handshake panicked, and with `panic = "abort"` that is a crash on launch.
/// Verification stays on: this is the standard webpki verifier against the
/// Mozilla root set compiled in by `webpki-roots`.
#[cfg(target_os = "android")]
fn android_tls_config() -> rustls::ClientConfig {
    let roots = rustls::RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    let mut config = rustls::ClientConfig::builder_with_provider(std::sync::Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .expect("ring supports the default TLS protocol versions")
    .with_root_certificates(roots)
    .with_no_client_auth();
    // reqwest is built without `http2` here, so only HTTP/1.1 is offered.
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    config
}

/// `reqwest::Client::new()` with the TLS provider installed.
pub fn client() -> reqwest::Client {
    builder().build().expect("default reqwest client config is valid")
}
