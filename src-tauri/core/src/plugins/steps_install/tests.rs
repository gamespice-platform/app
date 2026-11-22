use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::json;
use tempfile::TempDir;

use super::*;
use crate::logging::Audit;
use crate::plugins::jail::Jail;
use crate::plugins::manifest::{check_plugin_steps, parse_install_steps};

/// The argv that makes this test binary run only [`probe`].
const PROBE_ARGS: [&str; 5] = [
    "--exact",
    "plugins::steps_install::tests::probe",
    "--ignored",
    "--nocapture",
    "--test-threads=1",
];

/// The stand-in installer. A no-op unless the cwd has a `probe.mode`, which
/// only the tests below write, into a layer's `work/`.
#[test]
#[ignore = "run as a child process by the runInstaller tests"]
fn probe() {
    let Ok(mode) = std::fs::read_to_string("probe.mode") else {
        return;
    };

    match mode.trim() {
        "dump" => {
            let mut lines = vec![std::env::current_dir().unwrap().display().to_string()];
            let mut vars: Vec<String> = std::env::vars().map(|(k, v)| format!("{k}={v}")).collect();

            vars.sort();
            lines.extend(vars);
            std::fs::write("probe.out", lines.join("\n")).unwrap();
        }
        "fork" => {
            std::fs::write("probe.mode", "sleep").unwrap();

            let mut child = std::process::Command::new(std::env::current_exe().unwrap())
                .args(PROBE_ARGS)
                .spawn()
                .unwrap();

            std::fs::write("probe.pid", child.id().to_string()).unwrap();
            std::thread::sleep(Duration::from_secs(60));
            let _ = child.wait();
        }
        "sleep" => std::thread::sleep(Duration::from_secs(60)),
        "exit3" => std::process::exit(3),
        "touch" => std::fs::write("../files/made.txt", "made").unwrap(),
        other => panic!("unknown probe mode {other}"),
    }
}

struct Fixture {
    _tmp: TempDir,
    layer: PathBuf,
    downloads: PathBuf,
    manifest: Manifest,
    jail: Jail,
    audit: Audit,
    http: reqwest::Client,
}

impl Fixture {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let layer = tmp.path().join("layer.partial");
        let downloads = tmp.path().join("downloads");

        std::fs::create_dir_all(&layer).unwrap();
        std::fs::create_dir_all(&downloads).unwrap();

        let manifest = install_manifest("loader.test", "Test loader", &["example.com".into()]);
        let jail = Jail::build(&manifest, &install_roots(&layer, &downloads, None)).unwrap();
        let audit = Audit::new(tmp.path().join("audit.jsonl"));

        Self {
            _tmp: tmp,
            layer,
            downloads,
            manifest,
            jail,
            audit,
            http: crate::http::client(),
        }
    }

    fn exec(&self) -> Executor<'_> {
        Executor {
            manifest: &self.manifest,
            jail: &self.jail,
            http: &self.http,
            audit: &self.audit,
            downloads: None,
        }
    }

    fn work(&self) -> PathBuf {
        self.layer.join("work")
    }

    async fn install(&self, steps: serde_json::Value, env: &InstallEnv) -> InstallReport {
        let steps = parse_install_steps(&steps).expect("steps parse");
        self.exec().run_install(&steps, env).await
    }
}

fn probe_env() -> InstallEnv {
    let mut env = InstallEnv::default();

    env.runtimes.insert(
        "probe".into(),
        RuntimeExe {
            exe: std::env::current_exe().unwrap(),
            java: false,
        },
    );

    env
}

fn probe_step(expect: &[&str], timeout: Option<u64>) -> serde_json::Value {
    let mut step = json!({
        "action": "runInstaller",
        "program": "{runtime.probe}",
        "args": PROBE_ARGS,
        "cwd": "{layerWork}",
        "expectFiles": expect.iter().map(|p| json!({ "root": "layer", "path": p })).collect::<Vec<_>>(),
    });

    if let Some(t) = timeout {
        step["timeoutSecs"] = json!(t);
    }

    step
}

/// Writes the probe's mode where the installer's cwd will be.
fn arm(fx: &Fixture, mode: &str) {
    std::fs::create_dir_all(fx.work()).unwrap();
    std::fs::write(fx.work().join("probe.mode"), mode).unwrap();
}

// ------------------------------------------------------------ refusals

#[test]
fn plugin_with_run_installer_is_refused() {
    let steps: Vec<Step> = serde_json::from_value(json!([probe_step(&[], None)])).unwrap();
    let err = check_plugin_steps(&steps, &[]).unwrap_err();

    assert!(
        err.to_string().contains("runInstaller is not allowed in plugins"),
        "{err}"
    );
}

#[tokio::test]
async fn plugin_executor_refuses_install_steps_without_an_install_run() {
    let fx = Fixture::new();
    let steps: Vec<Step> = serde_json::from_value(json!([probe_step(&[], None)])).unwrap();
    let report = fx.exec().run(&steps, &RunContext::default()).await;

    assert!(!report.ok);
    assert!(report
        .error
        .unwrap()
        .contains("runInstaller is not allowed in plugins"));
}

#[test]
fn install_manifest_using_game_dir_is_refused() {
    let err = parse_install_steps(&json!([
        { "action": "writeText", "path": { "root": "gameDir", "path": "x.txt" }, "content": "x" }
    ]))
    .unwrap_err();

    assert!(err.to_string().contains("gameDir is not valid in an install manifest"), "{err}");
}

// ------------------------------------------------------------ downloadList

fn list_step(fx: &Fixture) -> Step {
    let _ = fx;
    serde_json::from_value(json!({
        "action": "downloadList",
        "list": { "root": "downloads", "path": "list.json" },
        "pointer": "/libraries",
        "urlField": "/downloads/artifact/url",
        "pathField": "/downloads/artifact/path",
        "sha1Field": "/downloads/artifact/sha1",
        "to": { "root": "layer", "path": "files/libraries" }
    }))
    .unwrap()
}

#[tokio::test]
async fn download_list_refuses_a_list_no_pinned_download_wrote() {
    let fx = Fixture::new();
    let mut step = list_step(&fx);

    if let Step::DownloadList { list, .. } = &mut step {
        list.root = FsRoot::Layer;
        list.path = "work/list.json".into();
    }

    let report = fx
        .install(
            json!([
                {
                    "action": "writeText",
                    "path": { "root": "layer", "path": "work/list.json" },
                    "content": "{\"libraries\":[]}"
                },
                serde_json::to_value(&step).unwrap()
            ]),
            &InstallEnv::default(),
        )
        .await;

    assert!(!report.run.ok);
    assert_eq!(report.run.failed_at, Some(1));
    assert!(
        report.run.error.as_deref().unwrap().contains("was not written by a pinned download"),
        "{:?}",
        report.run.error
    );
}

/// A pinned list in `downloads/list.json` with the given body, as a pinned
/// `download` in the same run would have left it.
fn pinned_list<'e>(fx: &Fixture, env: &'e InstallEnv, body: &serde_json::Value) -> InstallRun<'e> {
    let path = fx.downloads.join("list.json");

    std::fs::write(&path, serde_json::to_vec(body).unwrap()).unwrap();

    let target = fx
        .jail
        .resolve(
            &PathRef {
                root: FsRoot::Downloads,
                path: "list.json".into(),
            },
            true,
        )
        .unwrap();

    InstallRun {
        env,
        pinned: HashMap::from([(target.clone(), sha256_file(&target).unwrap())]),
        artifacts: HashMap::new(),
    }
}

#[tokio::test]
async fn download_list_refuses_an_entry_with_no_hash() {
    let fx = Fixture::new();
    let env = InstallEnv::default();
    let list = json!({ "libraries": [
        { "downloads": { "artifact": {
            "url": "https://example.com/a.jar", "path": "a/a.jar",
            "sha1": "a9993e364706816aba3e25717850c26c9cd0d89d" } } },
        { "downloads": { "artifact": { "url": "https://example.com/b.jar", "path": "b/b.jar" } } }
    ]});
    let run = pinned_list(&fx, &env, &list);
    let err = super::run(&fx.exec(), &list_step(&fx), &run, &mut Vec::new())
        .await
        .unwrap_err();

    assert!(err.to_string().contains("entry 1 has no hash"), "{err}");
    // Nothing was fetched: every entry is checked before the first download.
    assert!(!fx.layer.join("files/libraries/a/a.jar").exists());
}

#[tokio::test]
async fn download_list_refuses_a_list_changed_after_its_pin() {
    let fx = Fixture::new();
    let env = InstallEnv::default();
    let run = pinned_list(&fx, &env, &json!({ "libraries": [] }));

    std::fs::write(fx.downloads.join("list.json"), "{\"libraries\":[{}]}").unwrap();

    let err = super::run(&fx.exec(), &list_step(&fx), &run, &mut Vec::new())
        .await
        .unwrap_err();

    assert!(err.to_string().contains("changed after its pinned download"), "{err}");
}

#[tokio::test]
async fn download_list_refuses_an_entry_path_outside_its_folder() {
    let fx = Fixture::new();
    let env = InstallEnv::default();
    let list = json!({ "libraries": [
        { "downloads": { "artifact": {
            "url": "https://example.com/a.jar", "path": "../../escape.jar",
            "sha1": "a9993e364706816aba3e25717850c26c9cd0d89d" } } }
    ]});
    let run = pinned_list(&fx, &env, &list);

    assert!(super::run(&fx.exec(), &list_step(&fx), &run, &mut Vec::new())
        .await
        .is_err());
}

#[tokio::test]
async fn download_list_refuses_a_host_not_in_download_hosts() {
    let fx = Fixture::new();
    let env = InstallEnv::default();
    let list = json!({ "libraries": [
        { "downloads": { "artifact": {
            "url": "https://evil.example.net/a.jar", "path": "a/a.jar",
            "sha1": "a9993e364706816aba3e25717850c26c9cd0d89d" } } }
    ]});
    let run = pinned_list(&fx, &env, &list);

    assert!(super::run(&fx.exec(), &list_step(&fx), &run, &mut Vec::new())
        .await
        .is_err());
}

#[tokio::test]
async fn download_list_honours_max_and_skips_entries_already_present() {
    let fx = Fixture::new();
    let env = InstallEnv::default();
    let list = json!({ "libraries": [
        { "downloads": { "artifact": {
            "url": "https://example.com/a.jar", "path": "a/a.jar",
            "sha1": "a9993e364706816aba3e25717850c26c9cd0d89d" } } },
        { "name": "natives-only, no artifact" }
    ]});
    let run = pinned_list(&fx, &env, &list);
    let mut step = list_step(&fx);

    // Already there with the listed sha1 ("abc"): nothing to fetch, so this
    // passes with no network.
    std::fs::create_dir_all(fx.layer.join("files/libraries/a")).unwrap();
    std::fs::write(fx.layer.join("files/libraries/a/a.jar"), "abc").unwrap();

    let mut applied = Vec::new();
    super::run(&fx.exec(), &step, &run, &mut applied).await.unwrap();
    assert_eq!(applied.len(), 1);

    if let Step::DownloadList { max, .. } = &mut step {
        *max = Some(1);
    }

    let err = super::run(&fx.exec(), &step, &run, &mut Vec::new())
        .await
        .unwrap_err();

    assert!(err.to_string().contains("more than the 1 allowed"), "{err}");
}

#[test]
fn sha1_matches_the_fips_vectors() {
    assert_eq!(sha1_hex(b"abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
    assert_eq!(sha1_hex(b""), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
    assert_eq!(
        sha1_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
        "84983e441c3bd26ebaae4aa1f95129e5e54670f1"
    );
}

// ------------------------------------------------------------ mergeJson

fn vanilla() -> serde_json::Value {
    json!({
        "id": "1.20.1",
        "mainClass": "net.minecraft.client.main.Main",
        "libraries": [{ "name": "com.mojang:brigadier:1.1.8" }, { "name": "org.lwjgl:lwjgl:3.3.1" }],
        "arguments": { "game": ["--username", "${auth_player_name}"], "jvm": ["-cp", "${classpath}"] },
        "assetIndex": { "id": "5" }
    })
}

fn forge_child() -> serde_json::Value {
    json!({
        "id": "1.20.1-forge-47.2.0",
        "inheritsFrom": "1.20.1",
        "mainClass": "cpw.mods.bootstraplauncher.BootstrapLauncher",
        "libraries": [{ "name": "net.minecraftforge:forge:1.20.1-47.2.0" }],
        "arguments": { "game": ["--launchTarget", "forgeclient"] }
    })
}

#[test]
fn merge_json_follows_mojang_inherits_from() {
    let rules: Vec<MergeRule> = serde_json::from_value(json!([
        { "pointer": "/libraries", "mode": "append" },
        { "pointer": "/arguments/game", "mode": "append" }
    ]))
    .unwrap();
    let merged = merge_json(&vanilla(), &forge_child(), &rules).unwrap();

    assert_eq!(merged["mainClass"], "cpw.mods.bootstraplauncher.BootstrapLauncher");
    assert_eq!(merged["id"], "1.20.1-forge-47.2.0");
    assert_eq!(
        merged["libraries"],
        json!([
            { "name": "com.mojang:brigadier:1.1.8" },
            { "name": "org.lwjgl:lwjgl:3.3.1" },
            { "name": "net.minecraftforge:forge:1.20.1-47.2.0" }
        ])
    );
    assert_eq!(
        merged["arguments"]["game"],
        json!(["--username", "${auth_player_name}", "--launchTarget", "forgeclient"])
    );
    // Untouched by the child: kept from the parent.
    assert_eq!(merged["arguments"]["jvm"], json!(["-cp", "${classpath}"]));
    assert_eq!(merged["assetIndex"]["id"], "5");
}

#[test]
fn merge_json_modes() {
    let base = json!({ "a": [1, 2], "o": { "x": 1 } });
    let over = json!({ "a": [2, 3], "o": { "y": 2 } });
    let rule = |pointer: &str, mode: &str| -> Vec<MergeRule> {
        serde_json::from_value(json!([{ "pointer": pointer, "mode": mode }])).unwrap()
    };

    assert_eq!(merge_json(&base, &over, &rule("/a", "prepend")).unwrap()["a"], json!([2, 3, 1, 2]));
    assert_eq!(merge_json(&base, &over, &rule("/a", "union")).unwrap()["a"], json!([1, 2, 3]));
    assert_eq!(merge_json(&base, &over, &rule("/o", "replace")).unwrap()["o"], json!({ "y": 2 }));
    assert_eq!(merge_json(&base, &over, &[]).unwrap()["o"], json!({ "x": 1, "y": 2 }));
    assert!(merge_json(&base, &over, &rule("/o", "append")).is_err());
}

#[tokio::test]
async fn merge_json_step_writes_the_merged_version_json() {
    let fx = Fixture::new();

    std::fs::create_dir_all(fx.layer.join("files")).unwrap();
    std::fs::write(fx.downloads.join("1.20.1.json"), vanilla().to_string()).unwrap();
    std::fs::write(fx.layer.join("files/child.json"), forge_child().to_string()).unwrap();

    let report = fx
        .install(
            json!([{
                "action": "mergeJson",
                "base": { "root": "downloads", "path": "1.20.1.json" },
                "overlay": { "root": "layer", "path": "files/child.json" },
                "to": { "root": "layer", "path": "files/versions/{launchVersionId}.json" },
                "rules": [{ "pointer": "/libraries", "mode": "append" }]
            }]),
            &InstallEnv {
                values: Values(HashMap::from([("launchVersionId".into(), "forge-47".into())])),
                ..Default::default()
            },
        )
        .await;

    assert!(report.run.ok, "{:?}", report.run.error);

    let out: serde_json::Value =
        serde_json::from_slice(&std::fs::read(fx.layer.join("files/versions/forge-47.json")).unwrap())
            .unwrap();

    assert_eq!(out["libraries"].as_array().unwrap().len(), 3);
    assert_eq!(out["mainClass"], "cpw.mods.bootstraplauncher.BootstrapLauncher");
}

// ------------------------------------------------------------ the run

#[tokio::test]
async fn a_step_whose_when_does_not_match_is_reported_as_skipped() {
    let fx = Fixture::new();
    let env = InstallEnv {
        when: WhenContext {
            side: "server".into(),
            ..Default::default()
        },
        ..Default::default()
    };
    let report = fx
        .install(
            json!([
                {
                    "action": "writeText",
                    "when": { "side": "client" },
                    "path": { "root": "layer", "path": "files/launcher_profiles.json" },
                    "content": "{}",
                    "ifMissing": true
                },
                { "action": "mkdir", "path": { "root": "layer", "path": "files/mods" } }
            ]),
            &env,
        )
        .await;

    assert!(report.run.ok, "{:?}", report.run.error);
    assert_eq!(report.skipped, vec![0]);
    assert!(!fx.layer.join("files/launcher_profiles.json").exists());
    assert!(fx.layer.join("files/mods").is_dir());
}

#[tokio::test]
async fn write_text_if_missing_keeps_an_existing_file() {
    let fx = Fixture::new();

    std::fs::create_dir_all(fx.layer.join("files")).unwrap();
    std::fs::write(fx.layer.join("files/p.json"), "mine").unwrap();

    let report = fx
        .install(
            json!([{
                "action": "writeText",
                "path": { "root": "layer", "path": "files/p.json" },
                "content": "{}",
                "ifMissing": true
            }]),
            &InstallEnv::default(),
        )
        .await;

    assert!(report.run.ok, "{:?}", report.run.error);
    assert_eq!(std::fs::read_to_string(fx.layer.join("files/p.json")).unwrap(), "mine");
}

#[tokio::test]
async fn an_unbound_artifact_or_placeholder_fails_the_step() {
    let fx = Fixture::new();
    let report = fx
        .install(
            json!([{
                "action": "download",
                "artifact": "installer",
                "to": { "root": "downloads", "path": "forge-installer.jar" }
            }]),
            &InstallEnv::default(),
        )
        .await;

    assert!(report.run.error.as_deref().unwrap().contains("has no binding"));

    let report = fx
        .install(
            json!([{ "action": "mkdir", "path": { "root": "layer", "path": "files/{nope}" } }]),
            &InstallEnv::default(),
        )
        .await;

    assert!(!report.run.ok);
    assert!(!fx.layer.join("files/{nope}").exists());
}

#[tokio::test]
async fn require_file_checks_presence_and_digest() {
    let fx = Fixture::new();

    std::fs::write(fx.downloads.join("abc.txt"), "abc").unwrap();

    let ok = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    let step = |path: &str, sha: &str| {
        json!([{ "action": "requireFile", "path": { "root": "downloads", "path": path }, "sha256": sha }])
    };

    assert!(fx.install(step("abc.txt", ok), &InstallEnv::default()).await.run.ok);
    assert!(!fx.install(step("abc.txt", &"0".repeat(64)), &InstallEnv::default()).await.run.ok);
    assert!(!fx.install(step("none.txt", ok), &InstallEnv::default()).await.run.ok);
}

// ------------------------------------------------------------ runInstaller

fn probe_out(fx: &Fixture) -> (PathBuf, HashMap<String, String>) {
    let text = std::fs::read_to_string(fx.work().join("probe.out")).expect("probe ran");
    let mut lines = text.lines();
    let cwd = PathBuf::from(lines.next().unwrap());
    let vars = lines
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();

    (cwd, vars)
}

#[tokio::test]
async fn run_installer_clears_the_environment_and_runs_in_work() {
    // In the app's own environment, so a leak would show in the child's.
    std::env::set_var("JAVA_TOOL_OPTIONS", "-javaagent:/tmp/evil.jar");
    std::env::set_var("TMC_PROBE_SECRET", "leak");

    let fx = Fixture::new();
    let mut env = probe_env();

    env.proxy = vec![
        ("HTTPS_PROXY".into(), "http://proxy.invalid:3128".into()),
        ("_JAVA_OPTIONS".into(), "-Dx=1".into()),
    ];
    arm(&fx, "dump");

    let report = fx.install(json!([probe_step(&[], None)]), &env).await;

    assert!(report.run.ok, "{:?}", report.run.error);

    let (cwd, vars) = probe_out(&fx);
    let work = fx.work().canonicalize().unwrap();

    assert_eq!(cwd.canonicalize().unwrap(), work);

    for key in FORBIDDEN_ENV {
        assert!(!vars.contains_key(key), "{key} reached the installer");
    }

    assert!(!vars.contains_key("TMC_PROBE_SECRET"));

    let mut keys: Vec<&str> = vars.keys().map(String::as_str).collect();
    keys.sort();
    assert_eq!(
        keys,
        [
            "APPDATA",
            "HOME",
            "HTTPS_PROXY",
            "LOCALAPPDATA",
            "PATH",
            "TEMP",
            "TMP",
            "TMPDIR",
            "USERPROFILE"
        ]
    );
    assert_eq!(
        Path::new(&vars["PATH"]),
        std::env::current_exe().unwrap().parent().unwrap()
    );
    assert_eq!(Path::new(&vars["HOME"]), fx.work().join("home"));
    assert_eq!(Path::new(&vars["TMPDIR"]), fx.work().join("tmp"));
    assert!(fx.work().join("installer.log").is_file());
}

#[test]
fn java_gets_java_home_and_never_the_option_variables() {
    let env = installer_env(
        Path::new("/rt/java-17/bin/java"),
        true,
        Path::new("/l/work"),
        &[("JAVA_TOOL_OPTIONS".into(), "x".into()), ("CLASSPATH".into(), "y".into())],
    );
    let get = |k: &str| env.iter().find(|(key, _)| key == k).map(|(_, v)| v.as_str());

    assert_eq!(get("JAVA_HOME"), Some("/rt/java-17"));
    assert_eq!(get("PATH"), Some("/rt/java-17/bin"));
    assert_eq!(get("JAVA_TOOL_OPTIONS"), None);
    assert_eq!(get("CLASSPATH"), None);
}

#[tokio::test]
async fn run_installer_checks_expect_files() {
    let fx = Fixture::new();

    arm(&fx, "touch");
    let report = fx.install(json!([probe_step(&["files/made.txt"], None)]), &probe_env()).await;
    assert!(report.run.ok, "{:?}", report.run.error);

    arm(&fx, "touch");
    let report = fx
        .install(json!([probe_step(&["files/made.txt", "files/never.txt"], None)]), &probe_env())
        .await;

    assert!(!report.run.ok);
    assert!(
        report.run.error.as_deref().unwrap().contains("did not produce files/never.txt"),
        "{:?}",
        report.run.error
    );
}

#[tokio::test]
async fn run_installer_fails_on_a_non_zero_exit() {
    let fx = Fixture::new();

    arm(&fx, "exit3");

    let report = fx.install(json!([probe_step(&[], None)]), &probe_env()).await;

    assert!(!report.run.ok);
    assert!(report.run.error.as_deref().unwrap().contains("installer failed"), "{:?}", report.run.error);
}

#[tokio::test]
async fn run_installer_refuses_another_program_or_cwd() {
    let fx = Fixture::new();
    let mut step = probe_step(&[], None);

    step["program"] = json!("/bin/sh");
    assert!(!fx.install(json!([step.clone()]), &probe_env()).await.run.ok);

    step["program"] = json!("{runtime.probe}");
    step["cwd"] = json!("{layerFiles}");
    assert!(!fx.install(json!([step]), &probe_env()).await.run.ok);
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn run_installer_timeout_kills_the_grandchild_too() {
    let fx = Fixture::new();

    arm(&fx, "fork");

    let started = Instant::now();
    let report = fx.install(json!([probe_step(&[], Some(3))]), &probe_env()).await;

    assert!(!report.run.ok);
    assert!(report.run.error.as_deref().unwrap().contains("did not finish within 3s"));
    assert!(started.elapsed() < Duration::from_secs(20));

    let pid = std::fs::read_to_string(fx.work().join("probe.pid")).expect("the probe forked");
    let stat = format!("/proc/{}/stat", pid.trim());
    let deadline = Instant::now() + Duration::from_secs(2);

    loop {
        // Gone, or a zombie waiting for its new parent to reap it.
        let alive = std::fs::read_to_string(&stat)
            .ok()
            .and_then(|s| s.rsplit_once(')').map(|(_, rest)| rest.trim_start().chars().next()))
            .flatten()
            .is_some_and(|state| state != 'Z' && state != 'X');

        if !alive {
            break;
        }

        assert!(Instant::now() < deadline, "grandchild {} survived the timeout", pid.trim());
        std::thread::sleep(Duration::from_millis(50));
    }
}
