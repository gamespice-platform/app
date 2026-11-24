//! **What a game download row says it is.**
//!
//! A pack game has no build of its own: the installed build is its ENGINE's
//! (app "Godot"), shared by every game that runs on it. A row reading just
//! "Godot 1.2026…" hides that, so the queue row carries a role, an action and
//! who uses it, as plain `meta` strings the Downloads screen renders.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::library::db::InstalledGame;

/// Why an install started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    /// The user pressed Install or Update.
    Manual,
    /// The sync pass kept a game current (`updates::run`).
    Automatic,
}

impl Trigger {
    fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Automatic => "automatic",
        }
    }
}

/// The engine app id a `/apps` row names, `None` for an engine's own row.
pub fn engine_id(app: &Value) -> Option<i64> {
    let own = app.get("id").and_then(Value::as_i64);

    app.get("engine")
        .and_then(|e| e.get("id"))
        .and_then(Value::as_i64)
        .filter(|id| Some(*id) != own)
}

/// Installed games (other than `app_id`) whose catalogue row names `app_id` as
/// their engine, from a `/apps?ids=` answer, in the order given.
pub fn users_of(answer: &Value, app_id: i64, installed: &[InstalledGame]) -> Vec<String> {
    let Some(apps) = answer.get("apps").and_then(Value::as_array) else {
        return Vec::new();
    };

    installed
        .iter()
        .filter(|g| g.app_id != app_id)
        .filter(|g| {
            apps.iter()
                .find(|a| a.get("id").and_then(Value::as_i64) == Some(g.app_id))
                .and_then(engine_id)
                == Some(app_id)
        })
        .map(|g| g.name.clone())
        .collect()
}

/// The queue row's label and meta for one install or update.
///
/// `used_by` non-empty makes this an engine runtime; otherwise it is a game.
pub fn describe(
    app_id: i64,
    name: &str,
    version: &str,
    platform: &str,
    previous: Option<&str>,
    used_by: &[String],
    trigger: Trigger,
) -> (String, BTreeMap<String, String>) {
    let engine = !used_by.is_empty();

    let label = if engine {
        format!("{name} engine {version}")
    } else {
        format!("{name} {version}")
    };

    let mut meta = BTreeMap::from([
        ("kind".to_string(), "game".to_string()),
        ("appId".to_string(), app_id.to_string()),
        (
            "role".to_string(),
            if engine { "engine" } else { "game" }.to_string(),
        ),
        (
            "action".to_string(),
            if previous.is_some() {
                "update"
            } else {
                "install"
            }
            .to_string(),
        ),
        ("version".to_string(), version.to_string()),
        ("platform".to_string(), platform.to_string()),
        ("reason".to_string(), trigger.as_str().to_string()),
    ]);

    if let Some(old) = previous {
        meta.insert("fromVersion".to_string(), old.to_string());
    }

    if engine {
        meta.insert("usedBy".to_string(), used_by.join(", "));
    }

    (label, meta)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn game(app_id: i64, name: &str) -> InstalledGame {
        InstalledGame {
            app_id,
            slug: None,
            name: name.into(),
            platform: "linux-x64".into(),
            version: "1".into(),
            dir: String::new(),
            entry: None,
            args: Vec::new(),
            size_bytes: 0,
            installed_ms: 0,
            updated_ms: 0,
            auto_update: true,
        }
    }

    #[test]
    fn engine_update_names_its_users_and_old_version() {
        let (label, meta) = describe(
            84,
            "Godot",
            "2",
            "linux-x64",
            Some("1"),
            &["Fatal Contact".into(), "Airdrift".into()],
            Trigger::Automatic,
        );

        assert_eq!(label, "Godot engine 2");
        assert_eq!(meta["role"], "engine");
        assert_eq!(meta["action"], "update");
        assert_eq!(meta["fromVersion"], "1");
        assert_eq!(meta["usedBy"], "Fatal Contact, Airdrift");
        assert_eq!(meta["reason"], "automatic");
        assert_eq!(meta["kind"], "game");
        assert_eq!(meta["appId"], "84");
    }

    #[test]
    fn fresh_standalone_game_is_a_game_install() {
        let (label, meta) = describe(7, "Solo", "1", "linux-x64", None, &[], Trigger::Manual);

        assert_eq!(label, "Solo 1");
        assert_eq!(meta["role"], "game");
        assert_eq!(meta["action"], "install");
        assert_eq!(meta["reason"], "manual");
        assert!(!meta.contains_key("fromVersion"));
        assert!(!meta.contains_key("usedBy"));
    }

    #[test]
    fn users_are_the_installed_games_naming_this_engine() {
        let answer = json!({"apps": [
            {"id": 114, "engine": {"id": 84}},
            {"id": 115, "engine": {"id": 99}},
            {"id": 116, "engine": {"id": 84}},
            {"id": 84, "engine": {"id": 84}},
        ]});
        let installed = [
            game(84, "Godot"),
            game(114, "Fatal Contact"),
            game(115, "Other"),
            game(116, "Airdrift"),
        ];

        assert_eq!(
            users_of(&answer, 84, &installed),
            vec!["Fatal Contact", "Airdrift"]
        );
        assert!(users_of(&json!({}), 84, &installed).is_empty());
    }
}
