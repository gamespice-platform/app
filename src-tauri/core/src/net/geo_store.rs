//! The geolocation database as a managed local file, and the estimate built on it.
//!
//! Everything here is **opt-in**. [`GeoService`] does nothing until the caller
//! says estimation is enabled: with it off no file is downloaded and no address
//! is resolved or looked up (every entry point takes `enabled` and refuses
//! first, see [`require_enabled`]). The settings flag itself is
//! `AppSettings::estimate_latency`; this module never reads settings, so the
//! gate is explicit at each call and testable.
//!
//! # Source, licence, token
//!
//! The data is **IP2Location LITE** (default file `DB5LITECSVIPV6`: country to
//! lat/lon, IPv4 + IPv6), a free download that needs an account's download token
//! and is licensed **CC BY-SA 4.0**, which requires attribution: [`ATTRIBUTION`]
//! is shown beside the setting. The source is a URL template, `{token}` being
//! replaced by the token ([`GeoConfig`]); LITE downloads are rate-limited by
//! IP2Location (a handful per day), so a failure says what the server answered.
//!
//! The token is a credential: it lives in the `SecureStore`, never in
//! `settings.json`, and the template and token are settable only through
//! `geo_configure` (never `settings_patch`), because a token substituted into a
//! webview-chosen URL would be a way to mail it to a stranger.
//!
//! # Own address
//!
//! The user's public IP is learned from `self_ip_url` (default `api.ipify.org`,
//! one plain-text GET, cached for an hour), then looked up locally like any
//! server. That one request is the only thing sent anywhere besides the download.

use std::collections::HashMap;
use std::io::{BufReader, Read};
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;

use super::geo::{estimate_ping_ms, GeoDb};

/// The HTTP client type the service takes, so the app crate needs no direct
/// `reqwest` dependency to hold one.
pub type HttpClient = reqwest::Client;
use crate::error::{AppError, AppResult};

pub const ATTRIBUTION: &str =
    "This product includes IP2Location LITE data available from https://lite.ip2location.com.";
pub const LICENSE: &str = "CC BY-SA 4.0";
pub const LICENSE_URL: &str = "https://creativecommons.org/licenses/by-sa/4.0/";
pub const DEFAULT_SOURCE: &str =
    "https://www.ip2location.com/download/?token={token}&file=DB5LITECSVIPV6";
pub const DEFAULT_SELF_IP_URL: &str = "https://api.ipify.org";
/// The SecureStore name of the download token.
pub const TOKEN_NAME: &str = "geoip-token";
/// Days before the database is fetched again. IP2Location publishes monthly.
pub const REFRESH_DAYS: u64 = 30;
/// After a failed refresh, how long before the next automatic attempt.
const RETRY_AFTER_FAIL: Duration = Duration::from_secs(3600);
/// A LITE CSV zip is ~30 MB; this stops a wrong URL filling the disk.
const MAX_DOWNLOAD: u64 = 1 << 30;
const SELF_IP_TTL: Duration = Duration::from_secs(3600);

/// The one gate every entry point goes through.
pub fn require_enabled(enabled: bool) -> AppResult<()> {
    if enabled {
        Ok(())
    } else {
        Err(AppError::invalid(
            "Latency estimates are switched off (Settings, App, Servers).",
        ))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct GeoConfig {
    /// URL template; `{token}` is replaced by the stored download token.
    pub source: String,
    /// A URL that answers with the caller's public IP as plain text.
    pub self_ip_url: String,
}

impl Default for GeoConfig {
    fn default() -> Self {
        Self {
            source: DEFAULT_SOURCE.into(),
            self_ip_url: DEFAULT_SELF_IP_URL.into(),
        }
    }
}

/// https, or http to this machine (for a mirror or a test); no credentials.
pub fn validate_url(raw: &str) -> AppResult<url::Url> {
    let url =
        url::Url::parse(raw).map_err(|e| AppError::invalid(format!("Not a valid URL: {e}")))?;
    let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));

    if !(url.scheme() == "https" || (url.scheme() == "http" && loopback)) {
        return Err(AppError::invalid("The URL must use https."));
    }

    if !url.username().is_empty() || url.password().is_some() {
        return Err(AppError::invalid("The URL must not carry credentials."));
    }

    Ok(url)
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    Idle,
    Downloading,
    Converting,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GeoStatus {
    pub phase: Phase,
    pub bytes_done: u64,
    pub bytes_total: Option<u64>,
    /// The last download's failure, kept until the next attempt starts.
    pub error: Option<String>,
    /// Size of the database file on disk.
    pub size_bytes: Option<u64>,
    /// When the database was built from a download, unix seconds.
    pub updated_at: Option<u64>,
    pub ranges_v4: u64,
    pub ranges_v6: u64,
    pub has_token: bool,
    pub source: String,
    pub self_ip_url: String,
    pub attribution: &'static str,
    pub license: &'static str,
    pub license_url: &'static str,
}

#[derive(Default)]
struct Progress {
    phase: Option<Phase>,
    done: u64,
    total: Option<u64>,
    error: Option<String>,
    failed_at: Option<Instant>,
}

pub struct GeoService {
    dir: PathBuf,
    progress: Mutex<Progress>,
    db: RwLock<Option<Arc<GeoDb>>>,
    hosts: Mutex<HashMap<String, Option<(f64, f64)>>>,
    me: Mutex<Option<(Instant, (f64, f64))>>,
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl GeoService {
    /// `dir` is `<data>/geoip`; nothing is created until a download.
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            progress: Mutex::default(),
            db: RwLock::new(None),
            hosts: Mutex::default(),
            me: Mutex::default(),
        }
    }

    fn db_path(&self) -> PathBuf {
        self.dir.join("ip2location.gsgeo")
    }

    fn config_path(&self) -> PathBuf {
        self.dir.join("config.json")
    }

    pub fn config(&self) -> GeoConfig {
        std::fs::read_to_string(self.config_path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Validate and store the source template and own-IP URL.
    pub fn set_config(&self, config: &GeoConfig) -> AppResult<()> {
        // `{token}` is not valid in a host, so validate with a stand-in.
        validate_url(&config.source.replace("{token}", "x"))?;
        validate_url(&config.self_ip_url)?;
        std::fs::create_dir_all(&self.dir)?;
        std::fs::write(self.config_path(), serde_json::to_string_pretty(config)?)?;
        Ok(())
    }

    pub fn status(&self, has_token: bool) -> GeoStatus {
        let p = self.progress.lock().unwrap();
        let cfg = self.config();
        let db = self.cached();
        let (v4, v6) = db.as_ref().map(|d| d.counts()).unwrap_or((0, 0));

        GeoStatus {
            phase: p.phase.unwrap_or(Phase::Idle),
            bytes_done: p.done,
            bytes_total: p.total,
            error: p.error.clone(),
            size_bytes: std::fs::metadata(self.db_path()).ok().map(|m| m.len()),
            updated_at: db.as_ref().map(|d| d.built_at()),
            ranges_v4: v4 as u64,
            ranges_v6: v6 as u64,
            has_token,
            source: cfg.source,
            self_ip_url: cfg.self_ip_url,
            attribution: ATTRIBUTION,
            license: LICENSE,
            license_url: LICENSE_URL,
        }
    }

    /// The loaded database, reading the file on first use. The read guard is
    /// dropped before `load_quiet` takes the write lock (holding both deadlocks).
    fn cached(&self) -> Option<Arc<GeoDb>> {
        let held = self.db.read().unwrap().clone();
        held.or_else(|| self.load_quiet())
    }

    /// Read the file into memory if it is there and sound.
    fn load_quiet(&self) -> Option<Arc<GeoDb>> {
        let bytes = std::fs::read(self.db_path()).ok()?;
        let db = Arc::new(GeoDb::from_bytes(bytes).ok()?);
        *self.db.write().unwrap() = Some(Arc::clone(&db));
        Some(db)
    }

    /// Remove the database and everything derived from it.
    pub fn delete(&self) -> AppResult<()> {
        if self.progress.lock().unwrap().phase.is_some() {
            return Err(AppError::invalid(
                "A download is running; wait for it to finish.",
            ));
        }

        match std::fs::remove_file(self.db_path()) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }

        *self.db.write().unwrap() = None;
        self.hosts.lock().unwrap().clear();
        *self.me.lock().unwrap() = None;
        self.progress.lock().unwrap().error = None;
        Ok(())
    }

    /// Whether an automatic (re)download is due: no database, or an old one,
    /// and not just failed.
    pub fn refresh_due(&self, refresh_days: u64) -> bool {
        let p = self.progress.lock().unwrap();

        if p.phase.is_some() || p.failed_at.is_some_and(|t| t.elapsed() < RETRY_AFTER_FAIL) {
            return false;
        }

        drop(p);

        match self.cached() {
            None => true,
            Some(db) => now().saturating_sub(db.built_at()) > refresh_days * 86_400,
        }
    }

    /// Download and install the database. `enabled` is the opt-in; with it
    /// false nothing is fetched. One run at a time.
    pub async fn download(
        &self,
        enabled: bool,
        token: Option<String>,
        client: &reqwest::Client,
    ) -> AppResult<()> {
        require_enabled(enabled)?;

        {
            let mut p = self.progress.lock().unwrap();

            if p.phase.is_some() {
                return Err(AppError::invalid("A download is already running."));
            }

            *p = Progress {
                phase: Some(Phase::Downloading),
                ..Progress::default()
            };
        }

        let result = self.fetch_and_install(token, client).await;

        let mut p = self.progress.lock().unwrap();
        p.phase = None;

        if let Err(e) = &result {
            p.error = Some(e.to_string());
            p.failed_at = Some(Instant::now());
        }

        result
    }

    async fn fetch_and_install(
        &self,
        token: Option<String>,
        client: &reqwest::Client,
    ) -> AppResult<()> {
        let cfg = self.config();

        let url = if cfg.source.contains("{token}") {
            let token = token.filter(|t| !t.trim().is_empty()).ok_or_else(|| {
                AppError::invalid(
                    "A download token is needed. Create a free account at https://lite.ip2location.com, copy the download token from its Download page, and paste it into the token field.",
                )
            })?;
            let enc: String =
                url::form_urlencoded::byte_serialize(token.trim().as_bytes()).collect();
            cfg.source.replace("{token}", &enc)
        } else {
            cfg.source.clone()
        };

        validate_url(&url)?;

        tokio::fs::create_dir_all(&self.dir).await?;
        let part = self.dir.join("download.part");

        let outcome = async {
            let resp = client
                .get(&url)
                .timeout(Duration::from_secs(900))
                .send()
                .await
                .map_err(|e| AppError::Network(format!("The download could not start: {}", redact(&e.to_string(), &url))))?;

            if !resp.status().is_success() {
                return Err(AppError::Network(format!(
                    "The database server answered {}. A wrong or expired token, or the daily download limit, are the usual causes.",
                    resp.status()
                )));
            }

            self.progress.lock().unwrap().total = resp.content_length();

            if resp.content_length().is_some_and(|n| n > MAX_DOWNLOAD) {
                return Err(AppError::invalid("The download is larger than any geolocation database; refusing it."));
            }

            let mut file = tokio::fs::File::create(&part).await?;
            let mut stream = resp.bytes_stream();
            let mut done = 0u64;

            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|e| AppError::Network(format!("The download broke off: {e}")))?;
                done += chunk.len() as u64;

                if done > MAX_DOWNLOAD {
                    return Err(AppError::invalid("The download is larger than any geolocation database; refusing it."));
                }

                file.write_all(&chunk).await?;
                self.progress.lock().unwrap().done = done;
            }

            file.flush().await?;
            drop(file);

            self.progress.lock().unwrap().phase = Some(Phase::Converting);

            let (src, dst) = (part.clone(), self.db_path());
            let db = tokio::task::spawn_blocking(move || convert_download(&src, &dst))
                .await
                .map_err(|e| AppError::Internal(format!("conversion task: {e}")))??;

            Ok(db)
        }
        .await;

        let _ = tokio::fs::remove_file(&part).await;

        let db = outcome?;
        *self.db.write().unwrap() = Some(Arc::new(db));
        self.hosts.lock().unwrap().clear();
        *self.me.lock().unwrap() = None;
        Ok(())
    }

    /// Estimated round trip, in ms, from this machine to each host.
    ///
    /// `Ok` entries are `None` for a host the database cannot place or DNS
    /// cannot resolve; the call as a whole fails (loudly) only when there is no
    /// database or our own location cannot be determined.
    pub async fn estimate(
        &self,
        enabled: bool,
        hosts: &[String],
        client: &reqwest::Client,
    ) -> AppResult<Vec<Option<u32>>> {
        require_enabled(enabled)?;

        let db = self
            .cached()
            .ok_or_else(|| AppError::invalid("The geolocation database is not downloaded yet."))?;

        let me = self.own_coords(&db, client).await?;
        let mut out = Vec::with_capacity(hosts.len());

        for host in hosts {
            let cached = self.hosts.lock().unwrap().get(host).copied();

            let at = match cached {
                Some(at) => at,
                None => {
                    let at = resolve(host).await.and_then(|ip| db.lookup(ip));
                    self.hosts.lock().unwrap().insert(host.clone(), at);
                    at
                }
            };

            out.push(at.map(|(lat, lon)| estimate_ping_ms(me.0, me.1, lat, lon)));
        }

        Ok(out)
    }

    async fn own_coords(&self, db: &GeoDb, client: &reqwest::Client) -> AppResult<(f64, f64)> {
        if let Some((at, c)) = *self.me.lock().unwrap() {
            if at.elapsed() < SELF_IP_TTL {
                return Ok(c);
            }
        }

        let url = self.config().self_ip_url;
        validate_url(&url)?;

        let text = client
            .get(&url)
            .timeout(Duration::from_secs(10))
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|e| {
                AppError::Network(format!(
                    "Could not learn this machine's public address from {url}: {e}"
                ))
            })?
            .text()
            .await
            .map_err(|e| {
                AppError::Network(format!("Could not read the public address answer: {e}"))
            })?;

        let ip: IpAddr = text
            .trim()
            .parse()
            .map_err(|_| AppError::Network(format!("{url} did not answer with an IP address.")))?;

        let c = db.lookup(ip).ok_or_else(|| {
            AppError::invalid(format!(
                "The database cannot place this machine's public address ({ip})."
            ))
        })?;

        *self.me.lock().unwrap() = Some((Instant::now(), c));
        Ok(c)
    }
}

/// An IP literal, or the first address DNS gives (IPv4 preferred).
async fn resolve(host: &str) -> Option<IpAddr> {
    if let Ok(ip) = host.trim_matches(['[', ']']).parse::<IpAddr>() {
        return Some(ip);
    }

    let addrs: Vec<_> =
        tokio::time::timeout(Duration::from_secs(5), tokio::net::lookup_host((host, 0)))
            .await
            .ok()?
            .ok()?
            .collect();

    addrs
        .iter()
        .find(|a| a.is_ipv4())
        .or(addrs.first())
        .map(|a| a.ip())
}

/// Keep the token out of an error message.
fn redact(msg: &str, url: &str) -> String {
    msg.replace(url, "<download url>")
}

/// Unzip (or read) the downloaded file, build the compact database and write it
/// atomically to `dst`. A file that is not a zip is almost always IP2Location
/// answering with a refusal in plain text, which is quoted back.
pub fn convert_download(src: &Path, dst: &Path) -> AppResult<GeoDb> {
    let mut head = [0u8; 4];
    let n = std::fs::File::open(src)?.read(&mut head)?;

    let db = if n == 4 && &head[..2] == b"PK" {
        let mut zip = zip::ZipArchive::new(std::fs::File::open(src)?)
            .map_err(|e| AppError::invalid(format!("The download is not a readable zip: {e}")))?;

        let mut found = None;

        for i in 0..zip.len() {
            let f = zip
                .by_index(i)
                .map_err(|e| AppError::invalid(format!("Reading the zip failed: {e}")))?;

            if f.name()
                .is_ok_and(|n| n.to_ascii_lowercase().ends_with(".csv"))
            {
                found = Some(i);
                break;
            }
        }

        let idx =
            found.ok_or_else(|| AppError::invalid("The downloaded zip holds no CSV file."))?;

        let entry = zip
            .by_index(idx)
            .map_err(|e| AppError::invalid(format!("Reading the CSV from the zip failed: {e}")))?;

        GeoDb::from_csv(BufReader::new(entry), now())?
    } else {
        let mut text = String::new();
        std::fs::File::open(src)?
            .take(300)
            .read_to_string(&mut text)
            .ok();
        let text = text.trim();

        return Err(AppError::invalid(if text.is_empty() {
            "The download was empty.".to_string()
        } else {
            format!("The database server did not send a database: \"{text}\"")
        }));
    };

    let tmp = dst.with_extension("tmp");
    std::fs::write(&tmp, db.to_bytes())?;
    std::fs::rename(&tmp, dst)?;
    Ok(db)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const CSV: &str = "\"16777216\",\"16777471\",\"AU\",\"Australia\",\"Queensland\",\"Brisbane\",\"-27.4748\",\"153.0171\"\n\"134744064\",\"134744319\",\"US\",\"United States\",\"California\",\"Mountain View\",\"37.405992\",\"-122.078515\"\n\"134744320\",\"4294967295\",\"-\",\"-\",\"-\",\"-\",\"0.000000\",\"0.000000\"\n";

    fn zipped(csv: &str) -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        let mut w = zip::ZipWriter::new(&mut buf);
        w.start_file(
            "IP2LOCATION-LITE-DB5.CSV",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        w.write_all(csv.as_bytes()).unwrap();
        w.finish().unwrap();
        buf.into_inner()
    }

    /// One-shot local HTTP server answering every request with `status` + `body`.
    async fn serve(status: &'static str, body: Vec<u8>) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut s, _)) = l.accept().await {
                let body = body.clone();
                tokio::spawn(async move {
                    let mut buf = [0u8; 2048];
                    let _ = s.read(&mut buf).await;
                    let head = format!(
                        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = s.write_all(head.as_bytes()).await;
                    let _ = s.write_all(&body).await;
                });
            }
        });
        format!("http://{addr}")
    }

    fn svc() -> (tempfile::TempDir, GeoService) {
        let d = tempfile::tempdir().unwrap();
        let s = GeoService::new(d.path().join("geoip"));
        (d, s)
    }

    #[tokio::test]
    async fn disabled_means_no_download_and_no_lookup() {
        let (_d, s) = svc();
        let client = crate::http::client();
        // The URL would be reached if the gate were not first: it is unroutable.
        s.set_config(&GeoConfig {
            source: "https://example.invalid/{token}".into(),
            ..Default::default()
        })
        .unwrap();

        assert!(s.download(false, Some("t".into()), &client).await.is_err());
        assert!(s
            .estimate(false, &["8.8.8.8".into()], &client)
            .await
            .is_err());
        assert!(!s.db_path().exists(), "no database was written");
        assert!(require_enabled(true).is_ok() && require_enabled(false).is_err());
    }

    #[tokio::test]
    async fn downloads_converts_estimates_and_deletes() {
        let (_d, s) = svc();
        let base = serve("200 OK", zipped(CSV)).await;
        let me = serve("200 OK", b"1.0.0.5\n".to_vec()).await;
        s.set_config(&GeoConfig {
            source: format!("{base}/dl?token={{token}}"),
            self_ip_url: me,
        })
        .unwrap();
        let client = crate::http::client();

        s.download(true, Some("tok".into()), &client).await.unwrap();

        let st = s.status(true);
        assert_eq!(st.ranges_v4, 3);
        assert!(st.size_bytes.unwrap() > 24 && st.updated_at.is_some() && st.error.is_none());

        // Brisbane (me) to Mountain View: a long way, so well over the floor.
        let got = s
            .estimate(true, &["8.8.8.8".into(), "10.9.9.9".into()], &client)
            .await
            .unwrap();
        assert!(got[0].unwrap() > 150, "{got:?}");
        assert_eq!(got[1], None, "unplaceable host");

        // Same place as me: the floor.
        assert_eq!(
            s.estimate(true, &["1.0.0.77".into()], &client)
                .await
                .unwrap()[0],
            Some(7)
        );

        s.delete().unwrap();
        assert!(s.status(true).size_bytes.is_none());
        assert!(s
            .estimate(true, &["8.8.8.8".into()], &client)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn failures_are_loud_and_leave_no_database() {
        let (_d, s) = svc();
        let client = crate::http::client();

        // No token with a templated source.
        let e = s
            .download(true, None, &client)
            .await
            .unwrap_err()
            .to_string();
        assert!(e.contains("token"), "{e}");
        assert!(s.status(false).error.unwrap().contains("token"));

        // HTTP error.
        let base = serve("403 Forbidden", b"nope".to_vec()).await;
        s.set_config(&GeoConfig {
            source: format!("{base}/x"),
            ..Default::default()
        })
        .unwrap();
        let e = s
            .download(true, None, &client)
            .await
            .unwrap_err()
            .to_string();
        assert!(e.contains("403"), "{e}");

        // 200 with a plain-text refusal instead of a zip.
        let base = serve(
            "200 OK",
            b"THIS FILE CAN ONLY BE DOWNLOADED 5 TIMES A DAY".to_vec(),
        )
        .await;
        s.set_config(&GeoConfig {
            source: format!("{base}/x"),
            ..Default::default()
        })
        .unwrap();
        let e = s
            .download(true, None, &client)
            .await
            .unwrap_err()
            .to_string();
        assert!(e.contains("5 TIMES"), "{e}");

        assert!(s.status(false).size_bytes.is_none());
        assert!(!s.dir.join("download.part").exists());
    }

    #[test]
    fn urls_must_be_https_or_loopback_without_credentials() {
        assert!(validate_url("https://a.example/x").is_ok());
        assert!(validate_url("http://127.0.0.1:1/x").is_ok());
        assert!(validate_url("http://a.example/x").is_err());
        assert!(validate_url("https://u:p@a.example/x").is_err());
        let (_d, s) = svc();
        assert!(s
            .set_config(&GeoConfig {
                source: "http://evil.example/{token}".into(),
                ..Default::default()
            })
            .is_err());
    }

    #[test]
    fn refresh_is_due_when_missing_or_old_and_backs_off_after_failure() {
        let (_d, s) = svc();
        assert!(s.refresh_due(30));
        let db = GeoDb::from_csv(CSV.as_bytes(), now()).unwrap();
        std::fs::create_dir_all(&s.dir).unwrap();
        std::fs::write(s.db_path(), db.to_bytes()).unwrap();
        assert!(!s.refresh_due(30));
        let old = GeoDb::from_csv(CSV.as_bytes(), now() - 40 * 86_400).unwrap();
        std::fs::write(s.db_path(), old.to_bytes()).unwrap();
        *s.db.write().unwrap() = None;
        assert!(s.refresh_due(30));
        s.progress.lock().unwrap().failed_at = Some(Instant::now());
        assert!(!s.refresh_due(30));
    }
}
