//! The game catalog end to end: the website's fixtures served by a local API
//! server, read through the real API client and the catalog policy (remote,
//! 304, stale cache, compiled snapshot), and served install plans taken all the
//! way through consent, deploy and the ledger's undo.
//!
//! The pickers' lists are the website's own fixtures, as copied into
//! `src/lib/api/fixtures/catalog/wire`. The served plans for deploying are made
//! here from the shipped manifests, because a plan pins downloads by URL and
//! hash and the test's downloads live on a loopback HTTPS server. Forge's
//! signature is the shipped `.sig`, checked against the built-in keys.
//!
//! A second game, Valheim with a BepInEx layer, goes through the same pipeline
//! from two test-made manifests: nothing in the pipeline knows Minecraft.
//!
//! `TMC_API_BASE` is read once per process, so every test shares one API
//! server and keeps to its own paths (its own app id). Unix only: the fake
//! Forge installer is a shell script.

#![cfg(unix)]

mod support;

use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use serde_json::{json, Value};

use tmc_core::api::ApiClient;
use tmc_core::auth::AuthState;
use tmc_core::catalog::{self, types::Source, CatalogClient};
use tmc_core::deploy::Strategy;
use tmc_core::layers::plan::{self, PlanRequest, PlanSource};
use tmc_core::layers::progress::{PlanStepEvent, StepStatus};
use tmc_core::layers::{canonical, consent};
use tmc_core::library::db::LibraryDb;
use tmc_core::library::deploy::{
    deploy_sandbox, install_plan, plan_request, purge_sandbox, SandboxCtx,
};
use tmc_core::library::sandbox::{NewSandbox, Sandbox};
use tmc_core::logging::Audit;
use tmc_core::plugins::apps::AppPlugins;
use tmc_core::plugins::JailRoots;
use tmc_core::secure::SecureStore;
use tmc_core::settings::AppSettings;

use support::{repo, serve, sha, snapshot, Served};

const GAME: &str = "1.21.4";
const FABRIC: &str = "0.16.9";
const FORGE: &str = "54.1.0";
const FORGE_ID: &str = "1.21.4-forge-54.1.0";

const VERSION_JSON: &[u8] = br#"{"id":"1.21.4","type":"release"}"#;
const PROFILE_JSON: &[u8] = br#"{"id":"fabric-loader-0.16.9-1.21.4","inheritsFrom":"1.21.4"}"#;
const INSTALLER_JAR: &[u8] = b"PK\x03\x04 not really a jar";

fn fixture(name: &str) -> String {
    let path = repo().join("src/lib/api/fixtures/catalog").join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

// --------------------------------------------------------------- API server

/// One answer of the API server.
#[derive(Clone)]
struct Route {
    status: u16,
    body: String,
    etag: Option<String>,
    /// Drop the connection without answering, as a dead network does.
    offline: bool,
}

#[derive(Default)]
struct Api {
    routes: Mutex<HashMap<String, Route>>,
    hits: Mutex<HashMap<String, usize>>,
}

impl Api {
    /// Serve `data` in the API's envelope at `path` (a catalog client path).
    fn ok(&self, path: &str, data: &str, etag: Option<&str>) {
        let data: Value = serde_json::from_str(data).expect("fixture parses");
        self.route(
            path,
            200,
            json!({ "ok": true, "data": data }).to_string(),
            etag,
        );
    }

    fn route(&self, path: &str, status: u16, body: String, etag: Option<&str>) {
        let route = Route {
            status,
            body,
            etag: etag.map(str::to_string),
            offline: false,
        };
        self.routes
            .lock()
            .unwrap()
            .insert(format!("/api/app/v1{path}"), route);
    }

    fn offline(&self, path: &str) {
        let mut routes = self.routes.lock().unwrap();
        let route = routes.entry(format!("/api/app/v1{path}")).or_insert(Route {
            status: 200,
            body: String::new(),
            etag: None,
            offline: true,
        });
        route.offline = true;
    }

    fn hits(&self, path: &str) -> usize {
        self.hits
            .lock()
            .unwrap()
            .get(&format!("/api/app/v1{path}"))
            .copied()
            .unwrap_or(0)
    }
}

/// The one API server of this test binary, `TMC_API_BASE` pointing at it.
fn api() -> &'static Api {
    static API: OnceLock<&'static Api> = OnceLock::new();
    API.get_or_init(|| {
        if !cfg!(debug_assertions) {
            panic!("the API base override exists only in debug builds");
        }
        let api: &'static Api = Box::leak(Box::default());
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { return };
                std::thread::spawn(move || answer(api, stream));
            }
        });
        std::env::set_var("TMC_API_BASE", format!("http://127.0.0.1:{port}"));
        api
    })
}

fn answer(api: &Api, mut stream: std::net::TcpStream) {
    let mut request = Vec::new();
    let mut buf = [0u8; 2048];
    while !request.windows(4).any(|w| w == b"\r\n\r\n") {
        match stream.read(&mut buf) {
            Ok(0) | Err(_) => return,
            Ok(n) => request.extend_from_slice(&buf[..n]),
        }
    }
    let text = String::from_utf8_lossy(&request).into_owned();
    let target = text.split_whitespace().nth(1).unwrap_or("/").to_string();
    let if_none_match = text.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case("if-none-match")
            .then(|| value.trim().to_string())
    });

    *api.hits.lock().unwrap().entry(target.clone()).or_default() += 1;
    let route = api.routes.lock().unwrap().get(&target).cloned();
    let (status, body, etag) = match route {
        Some(r) if r.offline => return,
        Some(r) if r.etag.is_some() && r.etag == if_none_match => (304, String::new(), r.etag),
        Some(r) => (r.status, r.body, r.etag),
        None => (
            404,
            json!({ "ok": false, "code": "not_found", "message": "Not found." }).to_string(),
            None,
        ),
    };
    let etag = etag.map(|e| format!("etag: {e}\r\n")).unwrap_or_default();
    let head = format!(
        "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\n{etag}content-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body.as_bytes());
}

fn api_client(dir: &Path) -> ApiClient {
    api();
    ApiClient::new(
        Arc::new(AuthState::new()),
        Arc::new(SecureStore::new(dir)),
        "test",
    )
    .expect("api client")
}

// ------------------------------------------------------------ Served plans

/// An artifact on the loopback HTTPS server, as the wire names it.
fn wire_pin(port: u16, path: &str, body: &[u8]) -> Value {
    json!({ "url": format!("https://127.0.0.1:{port}{path}"), "mirrors": [], "sha256": sha(body), "size": body.len() })
}

/// One plan layer: the manifest, its canonical hash, and what fills it.
fn layer(manifest: &str, signature: Option<&str>, vars: Value, artifacts: Value) -> Value {
    let doc = canonical::parse(manifest).expect("manifest parses");
    let hash = sha(&canonical::canonical_bytes(&doc).expect("canonical"));
    json!({
        "manifestKey": doc["id"],
        "revision": doc["revision"],
        "sha256": hash,
        "exec": signature.is_some(),
        "signature": signature,
        "manifest": doc,
        "bindings": { "vars": vars, "artifacts": artifacts },
    })
}

fn shipped(rel: &str) -> String {
    std::fs::read_to_string(repo().join("manifests").join(rel)).expect(rel)
}

/// The bare signature the shipped `.sig` carries; the wire sends only that.
fn shipped_forge_signature() -> String {
    let sig: Value =
        serde_json::from_str(&shipped("minecraft/loaders/forge.manifest.json.sig")).unwrap();
    sig["signature"].as_str().expect("a signature").to_string()
}

/// Forge's manifest signed by a key that is not built in.
fn foreign_forge_signature() -> String {
    use base64::Engine;
    use ed25519_dalek::Signer;
    let doc = canonical::parse(&shipped("minecraft/loaders/forge.manifest.json")).unwrap();
    let key = ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]);
    let sig = key.sign(&canonical::canonical_bytes(&doc).unwrap());
    base64::engine::general_purpose::STANDARD.encode(sig.to_bytes())
}

/// The install plan the server would send for `req`, built from the shipped
/// Minecraft manifests with every download on the loopback server.
fn minecraft_plan(port: u16, req: &PlanRequest, forge_signature: &str) -> String {
    let version = layer(
        &shipped("minecraft/version.manifest.json"),
        None,
        json!({ "javaMajor": "21" }),
        json!({ "versionJson": wire_pin(port, "/mojang/1.21.4.json", VERSION_JSON) }),
    );
    let (loader, runtimes) = match req.loader.as_deref() {
        Some("fabric") => {
            let fabric = shipped("minecraft/loaders/fabric.manifest.json");
            let artifact = serde_json::from_str::<Value>(&fabric).unwrap()["steps"][0]["artifact"]
                .as_str()
                .expect("fabric downloads an artifact")
                .to_string();
            let pins = json!({ artifact: wire_pin(port, "/fabric/profile.json", PROFILE_JSON) });
            (layer(&fabric, None, json!({}), pins), json!([]))
        }
        Some("forge") => {
            let pins =
                json!({ "installer": wire_pin(port, "/forge/installer.jar", INSTALLER_JAR) });
            let forge = layer(
                &shipped("minecraft/loaders/forge.manifest.json"),
                Some(forge_signature),
                json!({}),
                pins,
            );
            let runtime = json!([{
                "kind": "java", "major": "21", "vendorLabel": "Test JRE", "entry": "bin/java",
                "stripSingleRoot": false, "artifact": wire_pin(port, "/runtime/jre.tar.gz", &fake_java())
            }]);
            (forge, runtime)
        }
        other => panic!("no plan for {other:?}"),
    };
    data(req, 412, vec![version, loader], runtimes)
}

fn data(req: &PlanRequest, app_id: i64, layers: Vec<Value>, runtimes: Value) -> String {
    json!({
        "app": { "id": app_id, "slug": req.app_slug },
        "request": {
            "gameVersion": req.game_version, "loader": req.loader, "loaderVersion": req.loader_version,
            "platform": req.platform, "side": req.side
        },
        "layers": layers,
        "runtimes": runtimes,
        "deprecated": null,
        "planVersion": 1,
    })
    .to_string()
}

/// The fake Java: a tar.gz holding `bin/java`, a script that writes what the
/// Forge installer writes into the folder it is given last.
fn fake_java() -> Vec<u8> {
    let script = format!(
        "#!/bin/sh\nfor a; do f=\"$a\"; done\n/bin/mkdir -p \"$f/versions/{FORGE_ID}\"\nprintf '{{\"id\":\"{FORGE_ID}\"}}' > \"$f/versions/{FORGE_ID}/{FORGE_ID}.json\"\necho installed\n"
    );
    let mut tar = tar::Builder::new(flate2::write::GzEncoder::new(
        Vec::new(),
        flate2::Compression::fast(),
    ));
    let mut header = tar::Header::new_gnu();
    header.set_size(script.len() as u64);
    header.set_mode(0o755);
    header.set_mtime(0);
    header.set_cksum();
    tar.append_data(&mut header, "bin/java", script.as_bytes())
        .expect("tar");
    tar.into_inner().expect("tar").finish().expect("gzip")
}

/// The BepInEx pack the Valheim layer downloads and unpacks.
fn bepinex_zip() -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    for (name, body) in BEPINEX {
        zip.start_file(*name, options).expect("zip entry");
        zip.write_all(body.as_bytes()).expect("zip write");
    }
    zip.finish().expect("zip").into_inner()
}

const BEPINEX: &[(&str, &str)] = &[
    ("BepInEx/core/BepInEx.dll", "not really an assembly"),
    ("doorstop_config.ini", "[General]\nenabled=true\n"),
    ("winhttp.dll", "not really a proxy"),
];

/// A version manifest and a BepInEx loader manifest for Valheim, in the same
/// vocabulary as Minecraft's: download, extract into the layer. Data only.
fn valheim_plan(port: u16, req: &PlanRequest) -> String {
    let version = json!({
        "$schema": "../../schemas/loader-manifest.v1.schema.json",
        "app": "valheim", "id": "valheim/version", "kind": "version",
        "revision": 1, "schemaVersion": 1, "steps": [], "title": "Valheim",
        "downloadHosts": []
    });
    let bepinex = json!({
        "$schema": "../../../schemas/loader-manifest.v1.schema.json",
        "app": "valheim", "id": "valheim/loader/bepinex", "kind": "loader", "loader": "bepinex",
        "revision": 1, "schemaVersion": 1, "title": "BepInEx",
        "requires": ["valheim/version"],
        "downloadHosts": [],
        "artifacts": {
            "pack": {
                "description": "The BepInEx pack, mirrored by the importer",
                "platform": "any",
                "sourceTemplate": "https://github.com/BepInEx/BepInEx/releases/download/v{loaderVersion}/BepInEx_win_x64_{loaderVersion}.zip"
            }
        },
        "steps": [
            { "action": "download", "artifact": "pack", "to": { "root": "downloads", "path": "BepInEx_{loaderVersion}.zip" } },
            {
                "action": "extract",
                "from": { "root": "downloads", "path": "BepInEx_{loaderVersion}.zip" },
                "to": { "root": "layer", "path": "files" }
            }
        ]
    });
    let layers = vec![
        layer(&version.to_string(), None, json!({}), json!({})),
        layer(
            &bepinex.to_string(),
            None,
            json!({}),
            json!({ "pack": wire_pin(port, "/bepinex.zip", &bepinex_zip()) }),
        ),
    ];
    data(req, 892970, layers, json!([]))
}

// ------------------------------------------------------------------- World

struct World {
    tmp: tempfile::TempDir,
    db: LibraryDb,
    api: ApiClient,
    plugins: AppPlugins,
    roots: JailRoots,
    exec_roots: tmc_core::detect::DetectRoots,
    settings: AppSettings,
    audit: Arc<Audit>,
    http: reqwest::Client,
    staging: PathBuf,
    backups: PathBuf,
    local: PathBuf,
    data_root: PathBuf,
    port: u16,
    served: Served,
    steps: Arc<Mutex<Vec<PlanStepEvent>>>,
}

const STOCK: [(&str, &str); 3] = [
    (
        "launcher_profiles.json",
        "{\n  \"profiles\" : {},\n  \"version\" : 3\n}\n",
    ),
    ("options.txt", "fov:70\n"),
    ("valheim_Data/boot.config", "gfx-enable-gfx-jobs=1\n"),
];

fn world() -> World {
    let tmp = tempfile::tempdir().expect("tempdir");
    let base = tmp.path().to_path_buf();
    let dir = |name: &str| {
        let d = base.join(name);
        std::fs::create_dir_all(&d).unwrap();
        d
    };

    let served = Served::default();
    let port = serve(&dir("tls"), served.clone());
    served.put("/mojang/1.21.4.json", VERSION_JSON.to_vec());
    served.put("/fabric/profile.json", PROFILE_JSON.to_vec());
    served.put("/forge/installer.jar", INSTALLER_JAR.to_vec());
    served.put("/runtime/jre.tar.gz", fake_java());
    served.put("/bepinex.zip", bepinex_zip());

    World {
        db: LibraryDb::open(base.join("library.db")).expect("db"),
        api: api_client(&dir("secure")),
        plugins: AppPlugins::load(&repo().join("plugins")),
        roots: JailRoots {
            data: dir("data"),
            cache: dir("cache"),
        },
        exec_roots: tmc_core::detect::DetectRoots {
            home: Some(dir("home")),
            ..Default::default()
        },
        settings: AppSettings {
            download_dir: Some(dir("downloads").display().to_string()),
            ..Default::default()
        },
        audit: Arc::new(Audit::new(base.join("audit.jsonl"))),
        http: tmc_core::http::builder()
            .danger_accept_invalid_certs(true)
            .build()
            .expect("client"),
        staging: dir("staging"),
        backups: dir("backups"),
        local: dir("local-mods"),
        data_root: dir("appdata"),
        port,
        served,
        steps: Arc::default(),
        tmp,
    }
}

impl World {
    fn client(&self) -> CatalogClient<'_, ApiClient> {
        CatalogClient::new(&self.db, &self.api, 60)
    }

    fn ctx<'a>(&'a self, source: &'a PlanSource) -> SandboxCtx<'a> {
        let steps = self.steps.clone();
        SandboxCtx {
            plugins: &self.plugins,
            roots: &self.roots,
            settings: &self.settings,
            http: &self.http,
            audit: &self.audit,
            exec_roots: &self.exec_roots,
            downloads: None,
            staging_root: &self.staging,
            backup_root: &self.backups,
            local_root: &self.local,
            data_root: &self.data_root,
            log_dir: &self.data_root,
            plan_source: Some(source),
            on_plan_step: Some(Arc::new(move |e| steps.lock().unwrap().push(e))),
        }
    }

    fn game(&self, name: &str) -> PathBuf {
        let game = self.tmp.path().join("games").join(name);
        for (rel, body) in STOCK {
            let path = game.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, body).unwrap();
        }
        game
    }

    #[allow(clippy::too_many_arguments)]
    fn sandbox(
        &self,
        game: &Path,
        app: (i64, &str),
        strategy: Strategy,
        version: &str,
        loader: &str,
        lv: &str,
    ) -> Sandbox {
        let id = self
            .db
            .sandbox_create(&NewSandbox {
                app_id: app.0,
                app_slug: Some(app.1.into()),
                app_name: None,
                name: format!("Pack {}", game.file_name().unwrap().to_string_lossy()),
                description: None,
                environment: Default::default(),
                strategy,
                game_version: Some(version.into()),
                loader: Some(loader.into()),
                loader_version: Some(lv.into()),
                preset: None,
                game_dir: Some(game.display().to_string()),
                options: BTreeMap::new(),
                cloud_sync: false,
                auto_update: false,
            })
            .expect("create");
        self.reload(id)
    }

    fn reload(&self, id: i64) -> Sandbox {
        self.db.sandbox_get(id).expect("get").expect("exists")
    }

    /// Fetch the served plan for `sandbox` through the catalog client.
    async fn fetch(&self, sandbox: &Sandbox) -> tmc_core::error::AppResult<Option<PlanSource>> {
        let req = plan_request(sandbox, None).expect("a plan request");
        Ok(catalog::plan_source(&self.client(), sandbox.app_id, &req)
            .await?
            .map(|(source, _)| source))
    }

    /// Consent, deploy, check, undo, and the folder is as it was.
    async fn round_trip(&self, sandbox: &Sandbox, source: &PlanSource, check: impl Fn(&Path)) {
        let game = PathBuf::from(sandbox.game_dir.as_deref().unwrap());
        let stock = snapshot(&game);
        let ctx = self.ctx(source);
        let plan = install_plan(sandbox, &ctx, None)
            .expect("plan")
            .expect("a managed install");
        consent::record(&self.db, sandbox.id, &plan, &plan.plan_hash).expect("consent");

        deploy_sandbox(&self.db, sandbox, &ctx, false)
            .await
            .unwrap_or_else(|e| panic!("deploy: {}", e.detail()));
        check(&game);

        purge_sandbox(&self.db, &self.reload(sandbox.id), &ctx).expect("purge");
        assert_eq!(snapshot(&game), stock, "undo left the folder changed");
    }
}

fn minecraft_request(loader: &str, version: &str) -> PlanRequest {
    PlanRequest {
        app_slug: "minecraft".into(),
        game_version: GAME.into(),
        loader: Some(loader.into()),
        loader_version: Some(version.into()),
        side: "client".into(),
        platform: plan::current_platform(),
        sandbox_id: String::new(),
        sandbox_name: String::new(),
        target_dir: String::new(),
    }
}

fn plan_path(app_id: i64, req: &PlanRequest) -> String {
    catalog::client::install_plan_path(
        app_id,
        &req.game_version,
        req.loader.as_deref(),
        req.loader_version.as_deref(),
        &req.platform,
        &req.side,
    )
}

// ------------------------------------------------------------------- Tests

/// The pickers' lists: the website's fixtures fetched, revalidated with a
/// 304, served stale when the network drops, and the compiled snapshot when
/// nothing was ever fetched.
#[tokio::test(flavor = "multi_thread")]
async fn the_pickers_read_remote_then_cache_then_stale_then_compiled() {
    if !cfg!(debug_assertions) {
        return;
    }
    let world = world();
    let api = api();
    let client = world.client();

    let path = catalog::client::catalog_path(412, &[], 0);
    api.ok(&path, &fixture("wire/catalog.json"), Some("\"catalog-1\""));

    let fresh = catalog::catalog(&client, 412, "minecraft", &[], 0, false)
        .await
        .expect("catalog");
    assert_eq!(fresh.provenance.source, Source::Remote);
    assert_eq!(fresh.app.id, Some(412));
    assert!(fresh.loaders.iter().any(|l| l.key == "forge"));

    // Inside the TTL: no request at all.
    let cached = catalog::catalog(&client, 412, "minecraft", &[], 0, false)
        .await
        .expect("catalog");
    assert_eq!(
        (cached.provenance.source, cached.provenance.stale),
        (Source::Cache, false)
    );
    assert_eq!(api.hits(&path), 1);

    // Forced: revalidated with the ETag, answered 304.
    let revalidated = catalog::catalog(&client, 412, "minecraft", &[], 0, true)
        .await
        .expect("catalog");
    assert_eq!(
        (revalidated.provenance.source, revalidated.provenance.stale),
        (Source::Cache, false)
    );
    assert_eq!(revalidated.loaders, fresh.loaders);
    assert_eq!(api.hits(&path), 2);

    // The network drops: the cached copy, marked stale.
    api.offline(&path);
    let stale = catalog::catalog(&client, 412, "minecraft", &[], 0, true)
        .await
        .expect("catalog");
    assert_eq!(
        (stale.provenance.source, stale.provenance.stale),
        (Source::Cache, true)
    );
    assert_eq!(stale.versions, fresh.versions);

    // Never fetched and offline: the snapshot compiled into the build.
    let never = catalog::client::catalog_path(4120, &[], 0);
    api.offline(&never);
    let compiled = catalog::catalog(&client, 4120, "minecraft", &[], 0, false)
        .await
        .expect("catalog");
    assert_eq!(compiled.provenance.source, Source::Compiled);
    // Whatever the shipped snapshot lists (it pins no rows yet), and no error.
    let shipped: Value = serde_json::from_str(&shipped("minecraft/catalog.snapshot.json")).unwrap();
    assert_eq!(
        compiled.loaders.len(),
        shipped["loaders"].as_array().unwrap().len()
    );

    // Loader versions take the same road.
    let lv_path = catalog::client::loader_versions_path(412, "forge", Some(GAME), &[], 0).unwrap();
    api.ok(
        &lv_path,
        &fixture("wire/loader-versions.json"),
        Some("\"lv-1\""),
    );
    let lv = catalog::loader_versions(
        &client,
        412,
        "minecraft",
        "forge",
        Some(GAME),
        &[],
        0,
        false,
    )
    .await
    .expect("loader versions");
    assert_eq!(lv.provenance.source, Source::Remote);
    assert_eq!(lv.items[0].version, FORGE);
    api.offline(&lv_path);
    let lv = catalog::loader_versions(&client, 412, "minecraft", "forge", Some(GAME), &[], 0, true)
        .await
        .expect("loader versions");
    assert_eq!(
        (lv.provenance.source, lv.provenance.stale),
        (Source::Cache, true)
    );

    let lv_never =
        catalog::client::loader_versions_path(4120, "fabric", Some(GAME), &[], 0).unwrap();
    api.offline(&lv_never);
    let lv = catalog::loader_versions(
        &client,
        4120,
        "minecraft",
        "fabric",
        Some(GAME),
        &[],
        0,
        false,
    )
    .await
    .expect("loader versions");
    assert_eq!(lv.provenance.source, Source::Compiled);
}

/// The install plan fixtures: the app's resolves; the website's is refused
/// for its placeholder hashes; a withdrawn build is an error carrying the
/// server's note; offline with nothing cached is the compiled source.
#[tokio::test(flavor = "multi_thread")]
async fn served_plan_fixtures_resolve_refuse_and_withdraw() {
    if !cfg!(debug_assertions) {
        return;
    }
    let world = world();
    let api = api();
    let client = world.client();
    let mut req = minecraft_request("fabric", FABRIC);
    req.platform = "linux-x64".into();

    // The app's fixture: canonical hashes, so it resolves.
    api.ok(&plan_path(412, &req), &fixture("install-plan.json"), None);
    let (source, provenance) = catalog::plan_source(&client, 412, &req)
        .await
        .expect("plan")
        .expect("served");
    assert_eq!(provenance.source, Source::Remote);
    let resolved = plan::resolve(&source, &req)
        .expect("resolves")
        .expect("managed");
    let keys: Vec<_> = resolved
        .layers
        .iter()
        .map(|l| l.manifest_key.as_str())
        .collect();
    assert_eq!(keys, ["minecraft/version", "minecraft/loader/fabric"]);

    // The website's fixture, verbatim: its layer hashes are placeholders.
    api.ok(
        &plan_path(413, &req),
        &fixture("wire/install-plan.json"),
        None,
    );
    let err = catalog::plan_source(&client, 413, &req)
        .await
        .expect_err("placeholder hashes");
    assert!(
        err.detail().contains("does not match its hash"),
        "{}",
        err.detail()
    );

    // Withdrawn: 410 with the website's error fixture.
    let withdrawn = minecraft_request("forge", "54.0.9");
    api.route(
        &plan_path(412, &withdrawn),
        410,
        fixture("wire/error.json"),
        None,
    );
    let err = catalog::plan_source(&client, 412, &withdrawn)
        .await
        .expect_err("withdrawn");
    assert!(
        err.detail().contains("Crashes on world load"),
        "{}",
        err.detail()
    );

    // Offline and never fetched: no served plan, so deploy uses the compiled one.
    let offline = minecraft_request("fabric", "0.16.8");
    api.offline(&plan_path(412, &offline));
    assert!(catalog::plan_source(&client, 412, &offline)
        .await
        .expect("fallback")
        .is_none());
}

/// Fabric: served plan → consent → deploy → undo, byte for byte.
#[tokio::test(flavor = "multi_thread")]
async fn a_served_fabric_plan_deploys_and_undoes() {
    if !cfg!(debug_assertions) {
        return;
    }
    let world = world();
    let game = world.game("fabric");
    // Its own app id: the fixtures test serves this same request for 412.
    let sandbox = world.sandbox(
        &game,
        (4122, "minecraft"),
        Strategy::Direct,
        GAME,
        "fabric",
        FABRIC,
    );
    let req = plan_request(&sandbox, None).unwrap();
    api().ok(
        &plan_path(4122, &req),
        &minecraft_plan(world.port, &req, ""),
        None,
    );

    let source = world.fetch(&sandbox).await.expect("fetch").expect("served");
    world
        .round_trip(&sandbox, &source, |game| {
            let id = format!("fabric-loader-{FABRIC}-{GAME}");
            assert_eq!(
                std::fs::read(game.join(format!("versions/{id}/{id}.json"))).unwrap(),
                PROFILE_JSON
            );
            assert_eq!(
                std::fs::read(game.join("versions/1.21.4/1.21.4.json")).unwrap(),
                VERSION_JSON
            );
            let profiles: Value = serde_json::from_slice(
                &std::fs::read(game.join("launcher_profiles.json")).unwrap(),
            )
            .unwrap();
            assert_eq!(
                profiles["profiles"][format!("tmc-{}", sandbox.id)]["lastVersionId"],
                id
            );
        })
        .await;
    assert_eq!(world.served.hits("/fabric/profile.json"), 1);
}

/// Forge: the served plan carries the shipped signature, the fake installer
/// runs, every step reports its progress, and undo restores the folder.
#[tokio::test(flavor = "multi_thread")]
async fn a_served_forge_plan_runs_its_installer_reports_steps_and_undoes() {
    if !cfg!(debug_assertions) {
        return;
    }
    let world = world();
    let game = world.game("forge");
    let sandbox = world.sandbox(
        &game,
        (412, "minecraft"),
        Strategy::Direct,
        GAME,
        "forge",
        FORGE,
    );
    let req = plan_request(&sandbox, None).unwrap();
    api().ok(
        &plan_path(412, &req),
        &minecraft_plan(world.port, &req, &shipped_forge_signature()),
        None,
    );

    let source = world.fetch(&sandbox).await.expect("fetch").expect("served");
    world
        .round_trip(&sandbox, &source, |game| {
            let installed = game.join(format!("versions/{FORGE_ID}/{FORGE_ID}.json"));
            assert!(installed.is_file(), "the installer's output is deployed");
            let state = world
                .reload(sandbox.id)
                .install_state
                .expect("install state");
            assert_eq!(state.launch_vars["launchVersionId"], FORGE_ID);
        })
        .await;

    let steps = world.steps.lock().unwrap().clone();
    let forge: Vec<_> = steps
        .iter()
        .filter(|e| e.layer == "minecraft/loader/forge")
        .collect();
    assert!(!forge.is_empty(), "no plan-step events: {steps:?}");
    assert!(forge
        .iter()
        .all(|e| e.sandbox_id == sandbox.id && !e.plan_hash.is_empty()));
    assert!(forge.iter().any(|e| e.status == StepStatus::Running));
    let total = forge[0].total;
    for index in 0..total {
        let last = forge
            .iter()
            .rev()
            .find(|e| e.index == index)
            .map(|e| e.status);
        assert!(
            matches!(last, Some(StepStatus::Done | StepStatus::Skipped)),
            "step {index} of {total} ended {last:?}: {forge:?}"
        );
    }
}

/// A served exec layer signed by a key this build does not carry is refused
/// before anything is downloaded or run.
#[tokio::test(flavor = "multi_thread")]
async fn a_served_plan_signed_by_a_foreign_key_is_refused() {
    if !cfg!(debug_assertions) {
        return;
    }
    let world = world();
    let game = world.game("foreign");
    let stock = snapshot(&game);
    // Its own app id, so this route does not clash with the Forge test's.
    let sandbox = world.sandbox(
        &game,
        (4121, "minecraft"),
        Strategy::Direct,
        GAME,
        "forge",
        FORGE,
    );
    let req = plan_request(&sandbox, None).unwrap();
    api().ok(
        &plan_path(4121, &req),
        &minecraft_plan(world.port, &req, &foreign_forge_signature()),
        None,
    );

    let source = world.fetch(&sandbox).await.expect("fetch").expect("served");
    let ctx = world.ctx(&source);
    let refused = match install_plan(&sandbox, &ctx, None) {
        Err(e) => e.detail(),
        Ok(_) => deploy_sandbox(&world.db, &sandbox, &ctx, false)
            .await
            .expect_err("refused")
            .detail(),
    };
    assert!(refused.to_lowercase().contains("signature"), "{refused}");
    assert_eq!(world.served.hits("/forge/installer.jar"), 0);
    assert_eq!(world.served.hits("/runtime/jre.tar.gz"), 0);
    assert_eq!(snapshot(&game), stock);
}

/// A second game through the same pipeline: Valheim with a BepInEx layer that
/// downloads a zip and unpacks it, deployed into the game folder and undone.
#[tokio::test(flavor = "multi_thread")]
async fn a_second_game_goes_through_the_same_pipeline() {
    if !cfg!(debug_assertions) {
        return;
    }
    let world = world();
    assert!(
        world.plugins.sandbox_spec("valheim").is_some(),
        "the shipped valheim plugin loads"
    );
    let game = world.game("valheim");
    let sandbox = world.sandbox(
        &game,
        (892970, "valheim"),
        Strategy::Direct,
        "0.219.16",
        "bepinex",
        "5.4.23.2",
    );
    let req = plan_request(&sandbox, None).unwrap();
    api().ok(
        &plan_path(892970, &req),
        &valheim_plan(world.port, &req),
        None,
    );

    let source = world.fetch(&sandbox).await.expect("fetch").expect("served");
    world
        .round_trip(&sandbox, &source, |game| {
            for (rel, body) in BEPINEX {
                assert_eq!(
                    std::fs::read_to_string(game.join(rel)).unwrap(),
                    *body,
                    "{rel}"
                );
            }
        })
        .await;
    assert_eq!(world.served.hits("/bepinex.zip"), 1);
}
