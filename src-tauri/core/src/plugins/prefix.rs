//! Finding a Steam game's Proton prefix.
//!
//! Proton keeps one Wine prefix per game at
//! `<library>/steamapps/compatdata/<appid>/pfx`, and a Windows game's documents,
//! saves and many of its mods live INSIDE it (`drive_c/users/steamuser/...`).
//! On Linux that is where "Documents" and "AppData" are.
//!
//! The location is derived, never configured, and never taken from a plugin:
//!
//!   1. the configured game folder must be `<library>/steamapps/common/<dir>`;
//!   2. `<library>/steamapps/appmanifest_<id>.acf` whose `installdir` is `<dir>`
//!      names the app id (digits only, so it cannot carry a path);
//!   3. `<library>/steamapps/compatdata/<id>/pfx` must exist and, once symlinks
//!      are resolved, still lie inside `<library>/steamapps/compatdata`.
//!
//! Anything else, and every platform but Linux, is `None`, and a grant for the
//! `prefix` root is then dropped like any root that is not configured. What a
//! plugin may do INSIDE the prefix is the jail's business
//! ([`super::jail::PREFIX_ALLOWED`]).

use std::path::{Path, PathBuf};

/// The prefix for the game installed at `game_dir`, if there is one.
#[cfg(target_os = "linux")]
pub fn steam_prefix(game_dir: &Path) -> Option<PathBuf> {
    resolve(game_dir)
}

#[cfg(not(target_os = "linux"))]
pub fn steam_prefix(_game_dir: &Path) -> Option<PathBuf> {
    None
}

/// The platform-independent part, so it is testable everywhere.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn resolve(game_dir: &Path) -> Option<PathBuf> {
    let game = crate::canon::canonicalize_or_keep(game_dir);

    let install = game.file_name()?.to_str()?.to_string();
    let common = game.parent()?;

    if common.file_name()? != "common" {
        return None;
    }

    let steamapps = common.parent()?;

    if steamapps.file_name()? != "steamapps" {
        return None;
    }

    let app_id = app_id_for(steamapps, &install)?;

    let compatdata = steamapps.join("compatdata");
    let pfx = compatdata.join(&app_id).join("pfx");

    let real = std::fs::canonicalize(&pfx).ok()?;
    let real_compat = std::fs::canonicalize(&compatdata).ok()?;

    // A prefix linked elsewhere is not ours to hand out.
    if !real.is_dir() || !real.starts_with(&real_compat) {
        return None;
    }

    Some(real)
}

fn app_id_for(steamapps: &Path, install: &str) -> Option<String> {
    for entry in std::fs::read_dir(steamapps).ok()?.flatten().take(5000) {
        let name = entry.file_name();
        let name = name.to_str()?;

        let Some(id) = name
            .strip_prefix("appmanifest_")
            .and_then(|n| n.strip_suffix(".acf"))
        else {
            continue;
        };

        if id.is_empty() || id.len() > 12 || !id.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }

        // A manifest is a few hundred bytes; refuse a hostile big one.
        let Ok(meta) = entry.metadata() else { continue };

        if !meta.is_file() || meta.len() > 256 * 1024 {
            continue;
        }

        let Ok(text) = std::fs::read_to_string(entry.path()) else {
            continue;
        };

        if installdir(&text).as_deref() == Some(install) {
            return Some(id.to_string());
        }
    }

    None
}

/// The `installdir` value of an `appmanifest_*.acf` (Valve's KeyValues text).
fn installdir(text: &str) -> Option<String> {
    for line in text.lines() {
        let mut quoted = line.split('"').skip(1).step_by(2);

        let Some(key) = quoted.next() else { continue };

        if key.eq_ignore_ascii_case("installdir") {
            return quoted.next().map(str::to_string);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::jail::Jail;
    use crate::plugins::manifest::{FsGrant, FsRoot, Manifest, PathRef};
    use std::collections::HashMap;

    /// `lib/steamapps/{common/Game, appmanifest_7.acf, compatdata/7/pfx/...}`.
    fn library(tmp: &Path) -> (PathBuf, PathBuf) {
        let steamapps = tmp.join("lib/steamapps");
        let game = steamapps.join("common/Game");
        let docs = steamapps.join("compatdata/7/pfx/drive_c/users/steamuser/Documents");

        std::fs::create_dir_all(&game).expect("game");
        std::fs::create_dir_all(&docs).expect("docs");
        std::fs::write(
            steamapps.join("appmanifest_7.acf"),
            "\"AppState\"\n{\n\t\"appid\"\t\t\"7\"\n\t\"installdir\"\t\t\"Game\"\n}\n",
        )
        .expect("acf");

        (game, steamapps.join("compatdata/7/pfx"))
    }

    fn manifest(path: &str) -> Manifest {
        let mut m: Manifest = serde_json::from_str(
            r#"{"manifestVersion":1,"id":"app.test","name":"t","version":"1","author":"t","permissions":{}}"#,
        )
        .expect("manifest");

        m.permissions.fs.push(FsGrant {
            root: FsRoot::Prefix,
            path: path.into(),
            write: true,
        });

        m
    }

    fn jail(base: &Path, path: &str) -> crate::error::AppResult<Jail> {
        let mut available: HashMap<&'static str, PathBuf> = HashMap::new();
        available.insert("prefix", base.to_path_buf());

        Jail::build(&manifest(path), &available)
    }

    fn pr(path: &str) -> PathRef {
        PathRef {
            root: FsRoot::Prefix,
            path: path.into(),
        }
    }

    #[test]
    fn the_prefix_is_found_from_the_game_folder_via_its_manifest() {
        let tmp = tempfile::tempdir().expect("tmp");
        let (game, pfx) = library(tmp.path());

        assert_eq!(resolve(&game), Some(std::fs::canonicalize(pfx).expect("c")));
    }

    #[test]
    fn nothing_is_found_for_a_folder_that_is_not_a_steam_install() {
        let tmp = tempfile::tempdir().expect("tmp");
        let (_game, _) = library(tmp.path());

        let elsewhere = tmp.path().join("games/Game");
        std::fs::create_dir_all(&elsewhere).expect("dir");

        assert_eq!(resolve(&elsewhere), None);

        // Right shape, but the manifest names another install dir.
        let other = tmp.path().join("lib/steamapps/common/Other");
        std::fs::create_dir_all(&other).expect("dir");

        assert_eq!(resolve(&other), None);
    }

    #[test]
    fn a_prefix_linked_out_of_compatdata_is_refused() {
        let tmp = tempfile::tempdir().expect("tmp");
        let (game, pfx) = library(tmp.path());

        let outside = tmp.path().join("outside");
        std::fs::create_dir_all(&outside).expect("dir");
        std::fs::remove_dir_all(&pfx).expect("rm");

        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, &pfx).expect("link");

        assert_eq!(resolve(&game), None);
    }

    #[test]
    fn a_grant_outside_the_allowed_subtrees_is_refused() {
        let tmp = tempfile::tempdir().expect("tmp");
        let (_g, pfx) = library(tmp.path());

        for bad in [
            "",
            "drive_c",
            "system.reg",
            "drive_c/windows",
            "drive_c/users/steamuser",
            "drive_c/users/steamuser/Documents2",
            "drive_c/users/other/Documents",
            "drive_c/users/steamuser/Documents/../../..",
        ] {
            assert!(jail(&pfx, bad).is_err(), "{bad:?} must be refused");
        }

        assert!(jail(&pfx, "drive_c/users/steamuser/Documents/My Games").is_ok());
        assert!(jail(&pfx, "drive_c/users/steamuser/AppData/Local").is_ok());
    }

    #[test]
    fn a_path_cannot_leave_its_grant_or_the_prefix() {
        let tmp = tempfile::tempdir().expect("tmp");
        let (_g, pfx) = library(tmp.path());

        let j = jail(&pfx, "drive_c/users/steamuser/Documents").expect("jail");
        let docs = "drive_c/users/steamuser/Documents";

        assert!(j.resolve(&pr(&format!("{docs}/Game/a.ini")), true).is_ok());

        for bad in [
            format!("{docs}/../../../../system.reg"),
            "../escape".to_string(),
            "/etc/passwd".to_string(),
            "system.reg".to_string(),
            "drive_c/windows/system32/x.dll".to_string(),
        ] {
            assert!(j.resolve(&pr(&bad), true).is_err(), "{bad:?}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_inside_the_prefix_pointing_out_is_refused() {
        let tmp = tempfile::tempdir().expect("tmp");
        let (_g, pfx) = library(tmp.path());

        let secret = tmp.path().join("home/secret");
        std::fs::create_dir_all(&secret).expect("dir");
        std::fs::write(secret.join("id_rsa"), "k").expect("file");

        // Wine's default prefixes link Documents to the real home; the jail
        // treats that exactly like a hostile link.
        let docs = pfx.join("drive_c/users/steamuser/Documents");
        std::fs::remove_dir_all(&docs).expect("rm");
        std::os::unix::fs::symlink(&secret, &docs).expect("link");

        let j = jail(&pfx, "drive_c/users/steamuser/Documents").expect("jail");

        assert!(j
            .resolve(&pr("drive_c/users/steamuser/Documents/id_rsa"), false)
            .is_err());
    }

    /// Every shipped rule that names the prefix stays inside the allowlist, so
    /// a bad rule fails here and not on a player's machine.
    #[test]
    fn every_shipped_prefix_location_is_allowlisted() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/app");
        let mut seen = 0;

        for entry in std::fs::read_dir(root).expect("plugins/app").flatten() {
            let Ok(text) = std::fs::read_to_string(entry.path().join("config.json")) else {
                continue;
            };

            let v: serde_json::Value = serde_json::from_str(&text).expect("json");

            for loc in v["config"]["locations"].as_array().into_iter().flatten() {
                if loc["root"] == "prefix" {
                    let p = loc["path"].as_str().expect("path");

                    assert!(
                        crate::plugins::jail::prefix_allowed(p),
                        "{}: {p}",
                        entry.path().display()
                    );
                    seen += 1;
                }
            }
        }

        assert!(seen >= 2, "expected the shipped prefix rules, saw {seen}");
    }
}
