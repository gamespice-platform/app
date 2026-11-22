//! Install plans end to end: the shipped Minecraft manifests, a catalog and a
//! runtime served over a real HTTPS socket, layers built on disk, deployed with
//! every strategy this machine supports, and undone.
//!
//! The catalog and the Forge signature are made here: the catalog because the
//! shipped snapshot pins nothing yet, the signature because the test needs a
//! second revision of the Forge manifest and only a key it owns can sign one.
//! The manifests themselves are the ones in `manifests/minecraft`, as shipped.
//!
//! The "installer" is a shell script packed as a Java runtime: it writes what
//! the Forge installer writes, fails, or plants an escaping symlink, as a
//! control file says. Unix only for that reason.

#![cfg(unix)]

use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use base64::Engine;
use ed25519_dalek::Signer;
use serde_json::{json, Value};
use sha2::Digest;

use tmc_core::deploy::{self, Strategy};
use tmc_core::error::AppError;
use tmc_core::layers::plan::{self, InstallPlan, PlanSource, Trust};
use tmc_core::layers::{canonical, consent};
use tmc_core::library::db::LibraryDb;
use tmc_core::library::deploy::{deploy_sandbox, install_plan, launch_plan, purge_sandbox, SandboxCtx};
use tmc_core::library::sandbox::{NewSandbox, Sandbox};
use tmc_core::logging::Audit;
use tmc_core::plugins::apps::AppPlugins;
use tmc_core::plugins::JailRoots;
use tmc_core::settings::AppSettings;

const GAME: &str = "1.21.4";
const FABRIC: &str = "0.16.9";
const FORGE: &str = "54.1.0";
const FORGE_WITHDRAWN: &str = "54.0.9";
const FORGE_ID: &str = "1.21.4-forge-54.1.0";

const VERSION_JSON: &[u8] = br#"{"id":"1.21.4","type":"release"}"#;
const PROFILE_JSON: &[u8] = br#"{"id":"fabric-loader-0.16.9-1.21.4","inheritsFrom":"1.21.4"}"#;
const INSTALLER_JAR: &[u8] = b"PK\x03\x04 not really a jar";

// ------------------------------------------------------------ HTTPS fixture

/// Files served by path over TLS, and how often each was asked for.
#[derive(Clone, Default)]
struct Served {
    files: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    hits: Arc<Mutex<HashMap<String, usize>>>,
}

impl Served {
    fn put(&self, path: &str, body: Vec<u8>) {
        self.files.lock().unwrap().insert(path.to_string(), body);
    }

    fn hits(&self, path: &str) -> usize {
        self.hits.lock().unwrap().get(path).copied().unwrap_or(0)
    }

    fn total(&self) -> usize {
        self.hits.lock().unwrap().values().sum()
    }
}

/// A self-signed certificate made by openssl, as DER (cert, PKCS#8 key).
fn certificate(dir: &Path) -> (Vec<u8>, Vec<u8>) {
    let run = |args: &[&str]| {
        let out = std::process::Command::new("openssl")
            .args(args)
            .current_dir(dir)
            .output()
            .expect("openssl runs");
        assert!(out.status.success(), "openssl {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    };

    run(&[
        "req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:prime256v1", "-nodes",
        "-keyout", "key.pem", "-out", "cert.der", "-outform", "DER", "-days", "1", "-subj",
        "/CN=127.0.0.1",
    ]);
    run(&["pkcs8", "-topk8", "-nocrypt", "-in", "key.pem", "-outform", "DER", "-out", "key.der"]);

    (
        std::fs::read(dir.join("cert.der")).unwrap(),
        std::fs::read(dir.join("key.der")).unwrap(),
    )
}

/// Serve `served` over HTTPS on loopback; returns the port.
fn serve(dir: &Path, served: Served) -> u16 {
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
                let Ok(conn) = rustls::ServerConnection::new(config) else { return };
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

// ----------------------------------------------------------------- Sources

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn sha(bytes: &[u8]) -> String {
    hex::encode(sha2::Sha256::digest(bytes))
}

fn pin(port: u16, path: &str, body: &[u8]) -> Value {
    json!({ "url": format!("https://127.0.0.1:{port}{path}"), "sha256": sha(body), "size": body.len() })
}

/// The fake Java: a tar.gz holding `bin/java`, a script steered by `control`.
fn runtime_archive(control: &Path) -> Vec<u8> {
    let script = format!(
        r#"#!/bin/sh
for a; do f="$a"; done
mode=$(/bin/cat '{control}' 2>/dev/null)
if [ "$mode" = fail ]; then echo "installer failed on purpose"; exit 1; fi
/bin/mkdir -p "$f/versions/{id}"
printf '{{"id":"{id}"}}' > "$f/versions/{id}/{id}.json"
if [ "$mode" = escape ]; then /bin/ln -s /etc/passwd "$f/escape"; fi
exit 0
"#,
        control = control.display(),
        id = FORGE_ID,
    );

    let mut tar = tar::Builder::new(flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast()));
    let mut header = tar::Header::new_gnu();
    header.set_size(script.len() as u64);
    header.set_mode(0o755);
    header.set_cksum();
    tar.append_data(&mut header, "bin/java", script.as_bytes()).expect("tar");

    tar.into_inner().expect("tar").finish().expect("gzip")
}

/// The test's own exec key, deterministic.
fn signing_key() -> ed25519_dalek::SigningKey {
    ed25519_dalek::SigningKey::from_bytes(&[7u8; 32])
}

/// The plan source: the shipped manifests, Forge at `forge_revision` and
/// signed by the test key, and a catalog pinning everything to the fixture.
fn source(port: u16, forge_revision: u64) -> PlanSource {
    let read = |rel: &str| std::fs::read_to_string(repo().join("manifests").join(rel)).expect(rel);

    let version = read("minecraft/version.manifest.json");
    let fabric = read("minecraft/loaders/fabric.manifest.json");
    let forge = read("minecraft/loaders/forge.manifest.json")
        .replacen("\"revision\": 1,", &format!("\"revision\": {forge_revision},"), 1);
    assert!(forge.contains(&format!("\"revision\": {forge_revision},")));

    let key = signing_key();
    let doc = canonical::parse(&forge).expect("forge parses");
    let signature = key.sign(&canonical::canonical_bytes(&doc).expect("canonical"));
    let b64 = base64::engine::general_purpose::STANDARD;
    let sig = json!({ "alg": "ed25519", "keyId": "test-key", "signature": b64.encode(signature.to_bytes()) })
        .to_string();
    let keys = json!([{ "keyId": "test-key", "ed25519": b64.encode(key.verifying_key().to_bytes()) }]).to_string();

    let fabric_artifact = serde_json::from_str::<Value>(&fabric).unwrap()["steps"][0]["artifact"]
        .as_str()
        .expect("fabric downloads an artifact")
        .to_string();

    let catalog = json!({
        "versions": { "items": [{ "versionId": GAME, "managed": true }] },
        "versionBindings": {
            GAME: {
                "vars": { "javaMajor": "21" },
                "artifacts": { "versionJson": pin(port, "/mojang/1.21.4.json", VERSION_JSON) }
            }
        },
        "loaders": [
            { "key": "fabric", "name": "Fabric", "managed": true },
            { "key": "forge", "name": "Forge", "managed": true }
        ],
        "loaderVersions": {
            "fabric": [{ "version": FABRIC }],
            "forge": [
                { "version": FORGE },
                { "version": FORGE_WITHDRAWN, "deprecated": { "withdrawn": true, "note": "it corrupts worlds" } }
            ]
        },
        "builds": [
            {
                "loader": "fabric", "loaderVersion": FABRIC, "gameVersion": GAME,
                "manifestKey": "minecraft/loader/fabric", "revision": 1,
                "bindings": { "artifacts": { fabric_artifact: pin(port, "/fabric/profile.json", PROFILE_JSON) } }
            },
            {
                "loader": "forge", "loaderVersion": FORGE, "gameVersion": GAME,
                "manifestKey": "minecraft/loader/forge", "revision": forge_revision,
                "bindings": { "artifacts": { "installer": pin(port, "/forge/installer.jar", INSTALLER_JAR) } }
            },
            {
                "loader": "forge", "loaderVersion": FORGE_WITHDRAWN, "gameVersion": GAME,
                "manifestKey": "minecraft/loader/forge", "revision": forge_revision,
                "bindings": { "artifacts": { "installer": pin(port, "/forge/installer.jar", INSTALLER_JAR) } }
            }
        ]
    })
    .to_string();

    let runtime = RUNTIME.with(|r| r.borrow().clone()).expect("runtime archive made first");
    let runtimes = json!({ "runtimes": [{
        "kind": "java", "major": "21", "platform": plan::current_platform(),
        "vendorLabel": "Test JRE", "entry": "bin/java", "stripSingleRoot": false,
        "artifact": pin(port, "/runtime/jre.tar.gz", &runtime)
    }] })
    .to_string();

    PlanSource::from_entries(
        Trust::Official,
        [
            ("keys/exec-keys.json", keys.as_str()),
            ("minecraft/catalog.snapshot.json", catalog.as_str()),
            ("minecraft/version.manifest.json", version.as_str()),
            ("minecraft/loaders/fabric.manifest.json", fabric.as_str()),
            ("minecraft/loaders/forge.manifest.json", forge.as_str()),
            ("minecraft/loaders/forge.manifest.json.sig", sig.as_str()),
            ("runtimes/java.snapshot.json", runtimes.as_str()),
        ],
    )
    .expect("source")
}

thread_local! {
    static RUNTIME: std::cell::RefCell<Option<Vec<u8>>> = const { std::cell::RefCell::new(None) };
}

// ------------------------------------------------------------------- World

struct World {
    tmp: tempfile::TempDir,
    db: LibraryDb,
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
    control: PathBuf,
    served: Served,
    port: u16,
    source: PlanSource,
}

/// The launcher profiles a user already has, oddly formatted on purpose: an
/// untouched file must come back byte for byte, whatever its layout.
const PROFILES: &str = "{\n  \"profiles\" : {\n    \"other\" : {\"lastVersionId\":\"1.20.1\",\"name\":\"Other\",\"type\":\"custom\"}\n  },\n  \"version\" : 3\n}\n";

const STOCK: [(&str, &str); 3] = [
    ("launcher_profiles.json", PROFILES),
    ("options.txt", "fov:70\n"),
    ("saves/World/level.dat", "level"),
];

fn world() -> World {
    let tmp = tempfile::tempdir().expect("tempdir");
    let base = tmp.path().to_path_buf();
    let dir = |name: &str| {
        let d = base.join(name);
        std::fs::create_dir_all(&d).unwrap();
        d
    };

    let control = base.join("installer-mode");
    RUNTIME.with(|r| *r.borrow_mut() = Some(runtime_archive(&control)));

    let served = Served::default();
    let port = serve(&dir("tls"), served.clone());
    served.put("/mojang/1.21.4.json", VERSION_JSON.to_vec());
    served.put("/fabric/profile.json", PROFILE_JSON.to_vec());
    served.put("/forge/installer.jar", INSTALLER_JAR.to_vec());
    served.put("/runtime/jre.tar.gz", RUNTIME.with(|r| r.borrow().clone()).unwrap());

    let plugins = AppPlugins::load(&repo().join("plugins"));
    assert!(plugins.sandbox_spec("minecraft").is_some(), "the shipped minecraft plugin loads");

    // A home with the Flatpak launcher in it, for the launch plan.
    let home = dir("home");
    let launcher = home.join(".local/share/flatpak/exports/bin/com.mojang.Minecraft");
    std::fs::create_dir_all(launcher.parent().unwrap()).unwrap();
    std::fs::write(&launcher, "#!/bin/sh\n").unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&launcher, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    let settings = AppSettings {
        download_dir: Some(dir("downloads").display().to_string()),
        ..Default::default()
    };

    World {
        db: LibraryDb::open(base.join("library.db")).expect("db"),
        plugins,
        roots: JailRoots { data: dir("data"), cache: dir("cache") },
        exec_roots: tmc_core::detect::DetectRoots {
            home: Some(home),
            ..Default::default()
        },
        settings,
        audit: Arc::new(Audit::new(base.join("audit.jsonl"))),
        http: tmc_core::http::builder()
            .danger_accept_invalid_certs(true)
            .build()
            .expect("client"),
        staging: dir("staging"),
        backups: dir("backups"),
        local: dir("local-mods"),
        data_root: dir("appdata"),
        control,
        source: source(port, 1),
        served,
        port,
        tmp,
    }
}

impl World {
    fn ctx(&self) -> SandboxCtx<'_> {
        self.ctx_with(&self.source)
    }

    fn ctx_with<'a>(&'a self, source: &'a PlanSource) -> SandboxCtx<'a> {
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
        }
    }

    fn installer(&self, mode: &str) {
        std::fs::write(&self.control, mode).unwrap();
    }

    /// A fresh instance folder holding the stock files.
    fn game(&self, name: &str) -> PathBuf {
        let game = self.tmp.path().join("games").join(name);
        for (rel, body) in STOCK {
            let path = game.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, body).unwrap();
        }
        game
    }

    fn sandbox(&self, game: &Path, strategy: Strategy, loader: &str, version: &str) -> Sandbox {
        let id = self
            .db
            .sandbox_create(&NewSandbox {
                app_id: 1,
                app_slug: Some("minecraft".into()),
                app_name: Some("Minecraft".into()),
                name: format!("Pack {}", game.file_name().unwrap().to_string_lossy()),
                description: None,
                environment: Default::default(),
                strategy,
                game_version: Some(GAME.into()),
                loader: Some(loader.into()),
                loader_version: Some(version.into()),
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

    fn plan(&self, sandbox: &Sandbox) -> InstallPlan {
        install_plan(sandbox, &self.ctx(), None).expect("plan").expect("a managed install")
    }

    fn agree(&self, sandbox: &Sandbox) {
        let plan = self.plan(sandbox);
        consent::record(&self.db, sandbox.id, &plan, &plan.plan_hash).expect("consent");
    }
}

/// Every file under `root`, symlinks read through, as (path, bytes).
fn snapshot(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(dir: &Path, base: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            let meta = std::fs::symlink_metadata(&path).unwrap();
            let rel = path.strip_prefix(base).unwrap().to_string_lossy().into_owned();
            if meta.is_dir() {
                walk(&path, base, out);
            } else {
                out.insert(rel, std::fs::read(&path).unwrap_or_else(|_| b"<dangling>".to_vec()));
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

/// What the deploy patch's undo writes when it has to re-render a file: four
/// spaces, sorted keys, no trailing newline.
fn render4(doc: &Value) -> Vec<u8> {
    let mut out = Vec::new();
    let fmt = serde_json::ser::PrettyFormatter::with_indent(b"    ");
    let mut ser = serde_json::Serializer::with_formatter(&mut out, fmt);
    serde::Serialize::serialize(doc, &mut ser).unwrap();
    out
}

fn profiles(game: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(game.join("launcher_profiles.json")).unwrap()).unwrap()
}

fn strategies(world: &World, game: &Path) -> Vec<Strategy> {
    deploy::available_strategies(&world.staging, game)
        .into_iter()
        .filter(|s| s.available)
        .filter_map(|s| Strategy::parse(&s.strategy))
        .filter(|s| *s != Strategy::Usvfs)
        .collect()
}

fn layer_dir(world: &World, plan: &InstallPlan, key: &str) -> PathBuf {
    let layer = plan.layers.iter().find(|l| l.manifest_key == key).expect(key);
    world.data_root.join("layers/minecraft").join(&layer.layer_hash)
}

// ------------------------------------------------------------------- Tests

/// Tests 1-3: a Fabric plan builds its layers, deploys with every strategy,
/// adds its launcher profile, and undoes to the exact bytes it found.
#[tokio::test(flavor = "multi_thread")]
async fn a_fabric_plan_deploys_with_every_strategy_and_undoes_byte_for_byte() {
    let world = world();
    let probe = world.game("probe");
    let available = strategies(&world, &probe);
    assert!(available.contains(&Strategy::Direct), "{available:?}");

    for strategy in available {
        let game = world.game(strategy.as_str());
        let stock = snapshot(&game);
        let sandbox = world.sandbox(&game, strategy, "fabric", FABRIC);

        let report = deploy_sandbox(&world.db, &sandbox, &world.ctx(), false)
            .await
            .unwrap_or_else(|e| panic!("{strategy:?}: {}", e.detail()));
        assert_eq!(report.used, strategy.as_str(), "{strategy:?} fell back: {:?}", report.fell_back);

        let fabric_id = format!("fabric-loader-{FABRIC}-{GAME}");
        assert_eq!(
            std::fs::read(game.join(format!("versions/{fabric_id}/{fabric_id}.json"))).unwrap(),
            PROFILE_JSON
        );
        assert_eq!(std::fs::read(game.join("versions/1.21.4/1.21.4.json")).unwrap(), VERSION_JSON);

        let ours = &profiles(&game)["profiles"][format!("tmc-{}", sandbox.id)];
        assert_eq!(ours["lastVersionId"], fabric_id);
        assert_eq!(ours["gameDir"], game.display().to_string());
        assert_eq!(profiles(&game)["profiles"]["other"]["name"], "Other");

        let state = world.reload(sandbox.id).install_state.expect("install state");
        assert_eq!(state.launch_vars["launchVersionId"], fabric_id);

        purge_sandbox(&world.db, &world.reload(sandbox.id), &world.ctx()).expect("purge");
        assert_eq!(snapshot(&game), stock, "{strategy:?} left the folder changed");
    }

    // Test 3, second half: another profile edited between deploy and undo.
    let game = world.game("edited");
    let original = json!({
        "profiles": { "other": { "lastVersionId": "1.20.1", "name": "Other", "type": "custom" } },
        "version": 3
    });
    std::fs::write(game.join("launcher_profiles.json"), render4(&original)).unwrap();
    let sandbox = world.sandbox(&game, Strategy::Direct, "fabric", FABRIC);
    deploy_sandbox(&world.db, &sandbox, &world.ctx(), false).await.expect("deploy");

    let mut edited = profiles(&game);
    edited["profiles"]["other"]["name"] = json!("Other, renamed by the user");
    std::fs::write(game.join("launcher_profiles.json"), render4(&edited)).unwrap();

    purge_sandbox(&world.db, &world.reload(sandbox.id), &world.ctx()).expect("purge");

    let mut expected = original.clone();
    expected["profiles"]["other"]["name"] = json!("Other, renamed by the user");
    assert_eq!(
        String::from_utf8(std::fs::read(game.join("launcher_profiles.json")).unwrap()).unwrap(),
        String::from_utf8(render4(&expected)).unwrap()
    );
}

/// Test 4: undo leaves our profile alone once the user changed it.
#[tokio::test(flavor = "multi_thread")]
async fn undo_keeps_a_patched_value_the_user_changed() {
    let world = world();
    let game = world.game("kept");
    let sandbox = world.sandbox(&game, Strategy::Direct, "fabric", FABRIC);
    deploy_sandbox(&world.db, &sandbox, &world.ctx(), false).await.expect("deploy");

    let ours = format!("tmc-{}", sandbox.id);
    let mut doc = profiles(&game);
    doc["profiles"][&ours]["name"] = json!("My own name");
    std::fs::write(game.join("launcher_profiles.json"), render4(&doc)).unwrap();

    purge_sandbox(&world.db, &world.reload(sandbox.id), &world.ctx()).expect("purge");

    let after = profiles(&game);
    assert_eq!(after["profiles"][&ours]["name"], "My own name");
    assert_eq!(after["profiles"]["other"]["name"], "Other");
    assert!(!game.join("versions").join(format!("fabric-loader-{FABRIC}-{GAME}")).exists());
}

/// Tests 5 and 8: an installer that fails, or one that plants a symlink out of
/// its layer, changes nothing in the game folder.
#[tokio::test(flavor = "multi_thread")]
async fn a_failed_or_escaping_build_leaves_the_game_folder_untouched() {
    let world = world();

    for (mode, expect) in [("fail", "The installer failed"), ("escape", "escape")] {
        let game = world.game(mode);
        let stock = snapshot(&game);
        let sandbox = world.sandbox(&game, Strategy::Direct, "forge", FORGE);
        world.agree(&sandbox);
        world.installer(mode);

        let e = deploy_sandbox(&world.db, &sandbox, &world.ctx(), false)
            .await
            .expect_err(mode);
        assert!(e.detail().contains(expect), "{mode}: {}", e.detail());
        if mode == "fail" {
            // The message names a log that outlives the discarded build.
            let log = e.detail().rsplit("The installer's log is ").next().unwrap().trim_end_matches('.').to_string();
            let log = std::fs::read_to_string(&log).unwrap_or_else(|_| panic!("no log at {log}"));
            assert!(log.contains("installer failed on purpose"), "{log}");
        }
        assert_eq!(snapshot(&game), stock, "{mode} changed the game folder");
        assert!(world.reload(sandbox.id).install_state.is_none());
    }
}

/// Test 6, 7 and 12: a second sandbox reuses the built layers, a tampered
/// layer is rebuilt, and a Forge launch names the Forge version.
#[tokio::test(flavor = "multi_thread")]
async fn layers_are_reused_rebuilt_when_tampered_and_name_the_launch_version() {
    let world = world();

    let first = world.sandbox(&world.game("first"), Strategy::Direct, "fabric", FABRIC);
    deploy_sandbox(&world.db, &first, &world.ctx(), false).await.expect("first");
    let after_first = world.served.total();
    assert_eq!(world.served.hits("/fabric/profile.json"), 1);

    // Test 6.
    let second_game = world.game("second");
    let second = world.sandbox(&second_game, Strategy::Direct, "fabric", FABRIC);
    deploy_sandbox(&world.db, &second, &world.ctx(), false).await.expect("second");
    assert_eq!(world.served.total(), after_first, "the second sandbox downloaded again");
    assert!(second_game.join("versions/1.21.4/1.21.4.json").is_file());

    // Test 7: same size, other bytes.
    let plan = world.plan(&second);
    let fabric_id = format!("fabric-loader-{FABRIC}-{GAME}");
    let file = layer_dir(&world, &plan, "minecraft/loader/fabric")
        .join(format!("files/versions/{fabric_id}/{fabric_id}.json"));
    std::fs::write(&file, vec![b'x'; PROFILE_JSON.len()]).unwrap();

    let third = world.sandbox(&world.game("third"), Strategy::Direct, "fabric", FABRIC);
    deploy_sandbox(&world.db, &third, &world.ctx(), false).await.expect("third");
    assert_eq!(world.served.hits("/fabric/profile.json"), 2, "the tampered layer was not rebuilt");
    assert_eq!(world.served.hits("/mojang/1.21.4.json"), 1, "an intact layer was rebuilt");
    assert_eq!(std::fs::read(&file).unwrap(), PROFILE_JSON);

    // Test 12.
    let game = world.game("forge");
    let forge = world.sandbox(&game, Strategy::Direct, "forge", FORGE);
    world.agree(&forge);
    world.installer("ok");
    deploy_sandbox(&world.db, &forge, &world.ctx(), false)
        .await
        .unwrap_or_else(|e| panic!("{}", e.detail()));
    assert!(game.join(format!("versions/{FORGE_ID}/{FORGE_ID}.json")).is_file());
    assert!(!game.join("work").exists() && !game.join("launcher_profiles.json.tmp").exists());
    assert_eq!(profiles(&game)["profiles"][format!("tmc-{}", forge.id)]["lastVersionId"], FORGE_ID);

    let launch = launch_plan(&world.db, &world.reload(forge.id), &world.ctx()).expect("launch plan");
    let at = launch.args.iter().position(|a| a == "--version").expect("--version");
    assert_eq!(launch.args[at + 1], FORGE_ID, "{:?}", launch.args);
}

/// Test 9: a withdrawn loader version is refused before anything is built.
#[tokio::test(flavor = "multi_thread")]
async fn a_withdrawn_build_is_refused_at_resolve() {
    let world = world();
    let game = world.game("withdrawn");
    let stock = snapshot(&game);
    let sandbox = world.sandbox(&game, Strategy::Direct, "forge", FORGE_WITHDRAWN);

    let e = install_plan(&sandbox, &world.ctx(), None).expect_err("withdrawn");
    assert!(e.detail().contains("withdrawn") && e.detail().contains("corrupts worlds"), "{}", e.detail());

    let e = deploy_sandbox(&world.db, &sandbox, &world.ctx(), false).await.expect_err("withdrawn");
    assert!(e.detail().contains("withdrawn"), "{}", e.detail());
    assert_eq!(world.served.total(), 0);
    assert_eq!(snapshot(&game), stock);
}

/// Test 10: an installer plan waits for consent, and a new manifest revision
/// asks again.
#[tokio::test(flavor = "multi_thread")]
async fn consent_is_asked_again_after_a_revision_bump() {
    let world = world();
    let game = world.game("consent");
    let stock = snapshot(&game);
    let sandbox = world.sandbox(&game, Strategy::Direct, "forge", FORGE);
    world.installer("ok");

    let hash = match deploy_sandbox(&world.db, &sandbox, &world.ctx(), false).await {
        Err(AppError::NeedsConsent(hash)) => hash,
        other => panic!("expected NeedsConsent, got {other:?}"),
    };
    assert_eq!(world.served.total(), 0, "nothing is downloaded before consent");
    assert_eq!(snapshot(&game), stock);

    let plan = world.plan(&sandbox);
    assert_eq!(plan.plan_hash, hash);
    assert!(plan.consents.iter().any(|c| c.key == "exec:minecraft/loader/forge"));
    assert!(plan.consents.iter().any(|c| c.key == "runtime:java-21"));
    consent::record(&world.db, sandbox.id, &plan, "not-the-hash").expect_err("a stale hash");
    consent::record(&world.db, sandbox.id, &plan, &hash).expect("consent");
    deploy_sandbox(&world.db, &sandbox, &world.ctx(), false)
        .await
        .unwrap_or_else(|e| panic!("{}", e.detail()));

    let bumped = source(world.port, 2);
    let ctx = world.ctx_with(&bumped);
    let sandbox = world.reload(sandbox.id);
    let plan2 = install_plan(&sandbox, &ctx, None).expect("plan").expect("managed");
    assert_ne!(plan2.plan_hash, hash);
    assert!(matches!(
        consent::check(&world.db, sandbox.id, &plan2),
        Err(AppError::NeedsConsent(h)) if h == plan2.plan_hash
    ));
    assert!(matches!(
        deploy_sandbox(&world.db, &sandbox, &ctx, false).await,
        Err(AppError::NeedsConsent(_))
    ));
}
