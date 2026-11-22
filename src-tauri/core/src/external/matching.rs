//! Which installed game, and which sandbox, a mod belongs in. Pure.
//!
//! The provider names a game its own way (Modrinth: Minecraft; Thunderstore:
//! a community; Nexus: a domain; a plugin header: Rust). This maps that to the
//! games the app has detected, then to the sandboxes of that game, and says
//! what does not line up rather than silently dropping a candidate: a Fabric
//! mod offered for a Forge sandbox is a warning, not an absence.

use serde::{Deserialize, Serialize};

use super::meta::ExternalMeta;
use super::GameHint;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameRef {
    pub app_id: i64,
    pub slug: Option<String>,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxRef {
    pub id: i64,
    pub app_id: i64,
    pub name: String,
    pub loader: Option<String>,
    pub game_version: Option<String>,
    pub is_default: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    pub app_id: i64,
    pub game_name: String,
    /// `None` means "the game, no sandbox picked".
    pub sandbox_id: Option<i64>,
    pub sandbox_name: Option<String>,
    /// Higher is a better fit.
    pub score: i32,
    pub warnings: Vec<String>,
}

/// Names a provider, a plugin header or a metadata file might use for a game,
/// mapped to the substring of the app's slug or title that identifies it.
const ALIASES: [(&str, &[&str]); 8] = [
    ("minecraft", &["minecraft"]),
    ("rust", &["rust", "oxide", "carbon"]),
    ("gmod", &["garrysmod", "garrys-mod", "gmod", "garry"]),
    ("garrysmod", &["garrysmod", "garrys-mod", "gmod", "garry"]),
    ("riskofrain2", &["riskofrain2", "risk-of-rain-2", "ror2"]),
    ("valheim", &["valheim"]),
    ("skyrimspecialedition", &["skyrim"]),
    ("cs2", &["counter-strike-2", "cs2"]),
];

fn norm(s: &str) -> String {
    s.to_ascii_lowercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect()
}

fn game_matches(want: &str, g: &GameRef) -> bool {
    let want = norm(want);
    let slug = norm(g.slug.as_deref().unwrap_or_default());
    let name = norm(&g.name);
    let squash = |s: &str| s.replace('-', "");
    let keys: Vec<String> = ALIASES
        .iter()
        .find(|(k, _)| *k == squash(&want))
        .map(|(_, v)| v.iter().map(|x| squash(x)).collect())
        .unwrap_or_else(|| vec![squash(&want)]);
    keys.iter()
        .any(|k| !k.is_empty() && (squash(&slug).contains(k) || squash(&name).contains(k)))
}

/// Rank the places a mod could go. `hint` comes from a provider, `meta` from a
/// file, both optional; with neither, every installed game is a bare candidate.
pub fn candidates(
    hint: &GameHint,
    meta: Option<&ExternalMeta>,
    games: &[GameRef],
    sandboxes: &[SandboxRef],
) -> Vec<Candidate> {
    let want_game = hint
        .game
        .clone()
        .or_else(|| meta.and_then(|m| m.game.clone()));
    let loaders: Vec<String> = if hint.loaders.is_empty() {
        meta.and_then(|m| m.loader.clone()).into_iter().collect()
    } else {
        hint.loaders.clone()
    };
    let versions: Vec<String> = if hint.game_versions.is_empty() {
        meta.and_then(|m| m.game_version.clone())
            .into_iter()
            .collect()
    } else {
        hint.game_versions.clone()
    };

    let mut out = Vec::new();
    for g in games {
        let game_fit = match &want_game {
            Some(w) if game_matches(w, g) => 100,
            Some(_) => continue,
            None => 10,
        };
        let boxes: Vec<&SandboxRef> = sandboxes.iter().filter(|s| s.app_id == g.app_id).collect();
        if boxes.is_empty() {
            out.push(Candidate {
                app_id: g.app_id,
                game_name: g.name.clone(),
                sandbox_id: None,
                sandbox_name: None,
                score: game_fit,
                warnings: vec![],
            });
            continue;
        }
        for s in boxes {
            let mut score = game_fit + i32::from(s.is_default);
            let mut warnings = Vec::new();
            if !loaders.is_empty() {
                match &s.loader {
                    Some(l) if loaders.iter().any(|x| x.eq_ignore_ascii_case(l)) => score += 50,
                    Some(l) => warnings.push(format!(
                        "this sandbox uses {l}; the mod is for {}",
                        loaders.join(" or ")
                    )),
                    None => warnings.push("this sandbox has no mod loader set".to_string()),
                }
            }
            if !versions.is_empty() {
                match &s.game_version {
                    Some(v)
                        if versions
                            .iter()
                            .any(|x| x == v || x.trim_start_matches(['~', '^', '>', '=']) == v) =>
                    {
                        score += 30
                    }
                    Some(v) => warnings.push(format!(
                        "this sandbox is on {v}; the mod lists {}",
                        versions.join(", ")
                    )),
                    None => {}
                }
            }
            out.push(Candidate {
                app_id: g.app_id,
                game_name: g.name.clone(),
                sandbox_id: Some(s.id),
                sandbox_name: Some(s.name.clone()),
                score,
                warnings,
            });
        }
    }
    out.sort_by(|a, b| b.score.cmp(&a.score).then(a.sandbox_id.cmp(&b.sandbox_id)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn games() -> Vec<GameRef> {
        vec![
            GameRef {
                app_id: 1,
                slug: Some("minecraft-java".into()),
                name: "Minecraft".into(),
            },
            GameRef {
                app_id: 2,
                slug: Some("rust".into()),
                name: "Rust".into(),
            },
            GameRef {
                app_id: 3,
                slug: Some("garrys-mod".into()),
                name: "Garry's Mod".into(),
            },
        ]
    }
    fn sb(id: i64, app: i64, loader: Option<&str>, ver: Option<&str>) -> SandboxRef {
        SandboxRef {
            id,
            app_id: app,
            name: format!("sb{id}"),
            loader: loader.map(Into::into),
            game_version: ver.map(Into::into),
            is_default: id == 10,
        }
    }

    #[test]
    fn minecraft_loader_and_version_rank_sandboxes() {
        let hint = GameHint {
            game: Some("minecraft".into()),
            loaders: vec!["fabric".into()],
            game_versions: vec!["1.20.1".into()],
        };
        let sbs = [
            sb(10, 1, Some("forge"), Some("1.20.1")),
            sb(11, 1, Some("fabric"), Some("1.20.1")),
            sb(12, 1, Some("fabric"), Some("1.19")),
        ];
        let c = candidates(&hint, None, &games(), &sbs);
        assert_eq!(c.len(), 3, "only Minecraft matches");
        assert_eq!(c[0].sandbox_id, Some(11));
        assert!(c[0].warnings.is_empty());
        let forge = c.iter().find(|x| x.sandbox_id == Some(10)).unwrap();
        assert!(forge.warnings[0].contains("forge"));
    }

    #[test]
    fn rust_plugin_and_gmod_alias() {
        let meta = ExternalMeta {
            game: Some("rust".into()),
            ..Default::default()
        };
        let c = candidates(&GameHint::default(), Some(&meta), &games(), &[]);
        assert_eq!((c.len(), c[0].app_id, c[0].sandbox_id), (1, 2, None));
        let hint = GameHint {
            game: Some("gmod".into()),
            ..Default::default()
        };
        assert_eq!(candidates(&hint, None, &games(), &[])[0].app_id, 3);
    }

    #[test]
    fn no_game_hint_offers_every_game_and_unknown_game_none() {
        assert_eq!(
            candidates(&GameHint::default(), None, &games(), &[]).len(),
            3
        );
        let hint = GameHint {
            game: Some("stardew".into()),
            ..Default::default()
        };
        assert!(candidates(&hint, None, &games(), &[]).is_empty());
    }
}
