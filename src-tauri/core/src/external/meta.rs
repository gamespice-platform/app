//! Read the metadata file other tools put in a mod, folder or archive.
//!
//! Field names follow each format's own documentation (checked against the
//! reference managers under `~/inspiration/game/mod/manager/`). Every reader is
//! a pure function over bytes; [`inspect`] finds the file and picks the reader.
//! No metadata at all is not an error: it is [`Inspection::NeedsSelection`].

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::tree::{self, FileTree, Source};
use crate::error::AppResult;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MetaFormat {
    ModrinthIndex,
    Fabric,
    Quilt,
    ForgeToml,
    NeoforgeToml,
    McmodInfo,
    Thunderstore,
    CurseforgeManifest,
    Prism,
    Plugin,
}

/// What every format boils down to.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalMeta {
    pub format: Option<MetaFormat>,
    pub id: Option<String>,
    pub name: Option<String>,
    pub version: Option<String>,
    pub authors: Vec<String>,
    pub description: Option<String>,
    /// `minecraft`, `rust` ... when the format says.
    pub game: Option<String>,
    pub loader: Option<String>,
    pub game_version: Option<String>,
    /// Dependency ids as the format names them.
    pub deps: Vec<String>,
    /// True for a modpack or instance: it lists mods rather than being one.
    pub is_pack: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum Inspection {
    Found {
        meta: ExternalMeta,
        /// Where the metadata file was, relative to the root.
        file: String,
    },
    /// Nothing recognisable. The picker shows the tree.
    NeedsSelection { tree: FileTree },
}

fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k)
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|x| !x.is_empty())
}

fn person(v: &Value) -> Option<String> {
    v.as_str().map(str::to_string).or_else(|| s(v, "name"))
}

pub fn read_modrinth_index(b: &[u8]) -> Option<ExternalMeta> {
    let v: Value = serde_json::from_slice(b).ok()?;
    v.get("formatVersion")?; // distinguishes it from any other index.json
    let deps = v.get("dependencies").and_then(Value::as_object);
    let loader = deps.and_then(|d| {
        ["fabric-loader", "quilt-loader", "forge", "neoforge"]
            .into_iter()
            .find(|k| d.contains_key(*k))
            .map(|k| k.trim_end_matches("-loader").to_string())
    });
    Some(ExternalMeta {
        format: Some(MetaFormat::ModrinthIndex),
        name: s(&v, "name"),
        version: s(&v, "versionId"),
        description: s(&v, "summary"),
        game: s(&v, "game"),
        loader,
        game_version: deps
            .and_then(|d| d.get("minecraft"))
            .and_then(Value::as_str)
            .map(str::to_string),
        is_pack: true,
        ..Default::default()
    })
}

pub fn read_fabric(b: &[u8]) -> Option<ExternalMeta> {
    let v: Value = serde_json::from_slice(b).ok()?;
    let id = s(&v, "id")?;
    let deps = v.get("depends").and_then(Value::as_object);
    Some(ExternalMeta {
        format: Some(MetaFormat::Fabric),
        name: s(&v, "name").or_else(|| Some(id.clone())),
        id: Some(id),
        version: s(&v, "version"),
        authors: v
            .get("authors")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(person).collect())
            .unwrap_or_default(),
        description: s(&v, "description"),
        game: Some("minecraft".into()),
        loader: Some("fabric".into()),
        game_version: deps
            .and_then(|d| d.get("minecraft"))
            .and_then(Value::as_str)
            .map(str::to_string),
        deps: deps
            .map(|d| d.keys().cloned().collect())
            .unwrap_or_default(),
        is_pack: false,
    })
}

pub fn read_quilt(b: &[u8]) -> Option<ExternalMeta> {
    let v: Value = serde_json::from_slice(b).ok()?;
    let l = v.get("quilt_loader")?;
    let id = s(l, "id")?;
    let md = l.get("metadata");
    Some(ExternalMeta {
        format: Some(MetaFormat::Quilt),
        name: md.and_then(|m| s(m, "name")).or_else(|| Some(id.clone())),
        id: Some(id),
        version: s(l, "version"),
        authors: md
            .and_then(|m| m.get("contributors"))
            .and_then(Value::as_object)
            .map(|o| o.keys().cloned().collect())
            .unwrap_or_default(),
        description: md.and_then(|m| s(m, "description")),
        game: Some("minecraft".into()),
        loader: Some("quilt".into()),
        deps: l
            .get("depends")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|d| d.as_str().map(str::to_string).or_else(|| s(d, "id")))
                    .collect()
            })
            .unwrap_or_default(),
        ..Default::default()
    })
}

/// A quoted TOML string value, `key = "value"` or `key='value'`, or a `'''` / `"""` block's first line.
fn toml_str(line: &str, key: &str) -> Option<String> {
    let rest = line
        .trim()
        .strip_prefix(key)?
        .trim_start()
        .strip_prefix('=')?
        .trim();
    let q = rest.chars().next().filter(|c| *c == '"' || *c == '\'')?;
    let inner = rest.trim_start_matches(q);
    let end = inner.find(q).unwrap_or(inner.len());
    Some(inner[..end].to_string()).filter(|x| !x.is_empty())
}

/// The subset of TOML a `mods.toml` uses: top-level keys and the first `[[mods]]`.
pub fn read_mods_toml(text: &str, neoforge: bool) -> Option<ExternalMeta> {
    let mut m = ExternalMeta {
        format: Some(if neoforge {
            MetaFormat::NeoforgeToml
        } else {
            MetaFormat::ForgeToml
        }),
        game: Some("minecraft".into()),
        loader: Some(if neoforge { "neoforge" } else { "forge" }.into()),
        ..Default::default()
    };
    let (mut in_first, mut seen) = (false, false);
    let mut in_deps = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('#') || t.is_empty() {
            continue;
        }
        if t.starts_with("[[mods]]") {
            in_first = !seen;
            seen = true;
            in_deps = false;
            continue;
        }
        if t.starts_with("[[dependencies.") {
            in_first = false;
            in_deps = true;
            continue;
        }
        if t.starts_with('[') {
            in_first = false;
            in_deps = false;
            continue;
        }
        if in_first {
            m.id = m.id.or_else(|| toml_str(t, "modId"));
            m.name = m.name.or_else(|| toml_str(t, "displayName"));
            m.version = m.version.or_else(|| toml_str(t, "version"));
            m.description = m.description.or_else(|| toml_str(t, "description"));
            if let Some(a) = toml_str(t, "authors") {
                m.authors = a.split(',').map(|x| x.trim().to_string()).collect();
            }
        } else if in_deps {
            if let Some(d) = toml_str(t, "modId") {
                if d != "minecraft" && d != "forge" && d != "neoforge" {
                    m.deps.push(d);
                }
            }
        }
    }
    m.id.as_ref()?;
    m.name = m.name.or_else(|| m.id.clone());
    Some(m)
}

pub fn read_mcmod_info(b: &[u8]) -> Option<ExternalMeta> {
    let v: Value = serde_json::from_slice(b).ok()?;
    let first = match &v {
        Value::Array(a) => a.first()?.clone(),
        Value::Object(o) => o.get("modList")?.as_array()?.first()?.clone(),
        _ => return None,
    };
    let id = s(&first, "modid")?;
    Some(ExternalMeta {
        format: Some(MetaFormat::McmodInfo),
        name: s(&first, "name").or_else(|| Some(id.clone())),
        id: Some(id),
        version: s(&first, "version"),
        authors: first
            .get("authorList")
            .or_else(|| first.get("authors"))
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
        description: s(&first, "description"),
        game: Some("minecraft".into()),
        loader: Some("forge".into()),
        game_version: s(&first, "mcversion"),
        ..Default::default()
    })
}

/// Thunderstore (`version_number`) or CurseForge (`manifestType`) `manifest.json`.
pub fn read_manifest_json(b: &[u8]) -> Option<ExternalMeta> {
    let v: Value = serde_json::from_slice(b).ok()?;
    if v.get("manifestType").is_some() {
        let mc = v.get("minecraft");
        let loader = mc
            .and_then(|m| m.get("modLoaders"))
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .and_then(|l| s(l, "id"))
            .map(|id| id.split('-').next().unwrap_or(&id).to_string());
        return Some(ExternalMeta {
            format: Some(MetaFormat::CurseforgeManifest),
            name: s(&v, "name"),
            version: s(&v, "version"),
            authors: s(&v, "author").into_iter().collect(),
            game: Some("minecraft".into()),
            loader,
            game_version: mc.and_then(|m| s(m, "version")),
            is_pack: v.get("manifestType").and_then(Value::as_str) == Some("minecraftModpack"),
            ..Default::default()
        });
    }
    let version = s(&v, "version_number")?;
    Some(ExternalMeta {
        format: Some(MetaFormat::Thunderstore),
        name: s(&v, "name"),
        version: Some(version),
        description: s(&v, "description"),
        deps: v
            .get("dependencies")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
        ..Default::default()
    })
}

/// Prism: `mmc-pack.json` components give the loader and Minecraft version,
/// `instance.cfg` (INI) gives the name.
pub fn read_prism(pack: Option<&[u8]>, cfg: Option<&str>) -> Option<ExternalMeta> {
    let mut m = ExternalMeta {
        format: Some(MetaFormat::Prism),
        is_pack: true,
        game: Some("minecraft".into()),
        ..Default::default()
    };
    if let Some(p) = pack.and_then(|b| serde_json::from_slice::<Value>(b).ok()) {
        for c in p
            .get("components")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let (uid, ver) = (s(c, "uid").unwrap_or_default(), s(c, "version"));
            match uid.as_str() {
                "net.minecraft" => m.game_version = ver,
                "net.fabricmc.fabric-loader" => m.loader = Some("fabric".into()),
                "org.quiltmc.quilt-loader" => m.loader = Some("quilt".into()),
                "net.neoforged" => m.loader = Some("neoforge".into()),
                "net.minecraftforge" => m.loader = Some("forge".into()),
                _ => {}
            }
        }
    }
    if let Some(c) = cfg {
        for line in c.lines() {
            if let Some(n) = line.strip_prefix("name=") {
                m.name = Some(n.trim().to_string()).filter(|x| !x.is_empty());
            }
        }
    }
    (m.name.is_some() || m.loader.is_some() || m.game_version.is_some()).then_some(m)
}

/// Oxide / Carbon plugin header: `[Info("Title", "Author", "1.0.0")]`.
pub fn read_plugin_header(text: &str) -> Option<ExternalMeta> {
    let mut m = ExternalMeta {
        format: Some(MetaFormat::Plugin),
        game: Some("rust".into()),
        ..Default::default()
    };
    for line in text.lines().take(400) {
        let t = line.trim();
        if let Some(args) = t.strip_prefix("[Info(").and_then(|r| r.split(")]").next()) {
            let parts: Vec<String> = args
                .split('"')
                .enumerate()
                .filter(|(i, _)| i % 2 == 1)
                .map(|(_, p)| p.to_string())
                .collect();
            if parts.len() >= 3 {
                m.name = Some(parts[0].clone());
                m.authors = vec![parts[1].clone()];
                m.version = Some(parts[2].clone());
            }
        } else if let Some(d) = t.strip_prefix("[Description(\"") {
            m.description = d.split('"').next().map(str::to_string);
        }
    }
    m.name.is_some().then_some(m)
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

/// Look in the root, then inside a single wrapping folder (what most archives are).
pub fn inspect(source: &Source, mod_extensions: &[String]) -> AppResult<Inspection> {
    let entries = source.entries()?;
    let tops: std::collections::BTreeSet<&str> = entries
        .iter()
        .map(|(p, _, _)| p.split('/').next().unwrap_or(""))
        .collect();
    let wrapper = (tops.len() == 1)
        .then(|| tops.iter().next().copied())
        .flatten()
        .filter(|t| {
            entries
                .iter()
                .any(|(p, _, _)| p.starts_with(&format!("{t}/")))
        });
    let prefixes: Vec<String> = match wrapper {
        Some(w) => vec![String::new(), format!("{w}/")],
        None => vec![String::new()],
    };
    let has = |p: &str| entries.iter().any(|(e, d, _)| !*d && e == p);
    for pre in &prefixes {
        let at = |n: &str| format!("{pre}{n}");
        type Reader = fn(&[u8]) -> Option<ExternalMeta>;
        let json_readers: [(&str, Reader); 5] = [
            ("modrinth.index.json", read_modrinth_index),
            ("fabric.mod.json", read_fabric),
            ("quilt.mod.json", read_quilt),
            ("mcmod.info", read_mcmod_info),
            ("manifest.json", read_manifest_json),
        ];
        for (name, reader) in json_readers {
            let p = at(name);
            if has(&p) {
                if let Some(meta) = source.read(&p)?.and_then(|b| reader(&b)) {
                    return Ok(Inspection::Found { meta, file: p });
                }
            }
        }
        for (name, neo) in [
            ("META-INF/neoforge.mods.toml", true),
            ("META-INF/mods.toml", false),
            ("neoforge.mods.toml", true),
        ] {
            let p = at(name);
            if has(&p) {
                if let Some(meta) = source
                    .read(&p)?
                    .and_then(|b| read_mods_toml(&text(&b), neo))
                {
                    return Ok(Inspection::Found { meta, file: p });
                }
            }
        }
        let (pack, cfg) = (at("mmc-pack.json"), at("instance.cfg"));
        if has(&pack) || has(&cfg) {
            let pb = if has(&pack) {
                source.read(&pack)?
            } else {
                None
            };
            let cb = if has(&cfg) { source.read(&cfg)? } else { None };
            if let Some(meta) = read_prism(pb.as_deref(), cb.as_deref().map(text).as_deref()) {
                return Ok(Inspection::Found {
                    meta,
                    file: if has(&pack) { pack } else { cfg },
                });
            }
        }
    }
    // A plugin source file: the first `.cs` or `.lua`-less header that parses.
    for (p, d, _) in entries
        .iter()
        .filter(|(p, d, _)| !*d && p.ends_with(".cs"))
        .take(20)
    {
        let _ = d;
        if let Some(meta) = source.read(p)?.and_then(|b| read_plugin_header(&text(&b))) {
            return Ok(Inspection::Found {
                meta,
                file: p.clone(),
            });
        }
    }
    Ok(Inspection::NeedsSelection {
        tree: tree::build(source, mod_extensions)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::external::tree::testutil::write_zip;

    #[test]
    fn modrinth_index() {
        let m = read_modrinth_index(
            br#"{"formatVersion":1,"game":"minecraft","name":"P","versionId":"1.0","summary":"s",
            "dependencies":{"minecraft":"1.20.1","fabric-loader":"0.15"}}"#,
        )
        .unwrap();
        assert_eq!(
            (m.loader.as_deref(), m.game_version.as_deref(), m.is_pack),
            (Some("fabric"), Some("1.20.1"), true)
        );
        assert!(read_modrinth_index(b"{}").is_none());
    }

    #[test]
    fn fabric_and_quilt() {
        let f = read_fabric(br#"{"id":"sodium","name":"Sodium","version":"1","authors":["a",{"name":"b"}],"depends":{"minecraft":"~1.20"}}"#).unwrap();
        assert_eq!(
            (f.authors, f.game_version.as_deref()),
            (vec!["a".to_string(), "b".into()], Some("~1.20"))
        );
        let q = read_quilt(br#"{"quilt_loader":{"id":"q","version":"2","metadata":{"name":"Q","contributors":{"Ann":"Owner"}},"depends":[{"id":"quilt_loader"},"x"]}}"#).unwrap();
        assert_eq!(
            (q.name.as_deref(), q.authors, q.deps),
            (
                Some("Q"),
                vec!["Ann".to_string()],
                vec!["quilt_loader".to_string(), "x".into()]
            )
        );
    }

    #[test]
    fn forge_and_neoforge_toml() {
        let t = "modLoader=\"javafml\"\nloaderVersion=\"[47,)\"\n[[mods]]\nmodId=\"jei\"\nversion=\"1.2\"\ndisplayName=\"JEI\"\nauthors=\"A, B\"\ndescription='''Items'''\n[[dependencies.jei]]\nmodId=\"forge\"\n[[dependencies.jei]]\nmodId=\"lib\"\n[[mods]]\nmodId=\"second\"\n";
        let m = read_mods_toml(t, false).unwrap();
        assert_eq!(
            (m.id.as_deref(), m.name.as_deref(), m.version.as_deref()),
            (Some("jei"), Some("JEI"), Some("1.2"))
        );
        assert_eq!(
            (m.authors, m.deps, m.loader.as_deref()),
            (
                vec!["A".to_string(), "B".into()],
                vec!["lib".to_string()],
                Some("forge")
            )
        );
        assert_eq!(
            read_mods_toml(t, true).unwrap().loader.as_deref(),
            Some("neoforge")
        );
        assert!(read_mods_toml("[[mods]]\n", false).is_none());
    }

    #[test]
    fn mcmod_info_both_shapes() {
        let a = read_mcmod_info(
            br#"[{"modid":"m","name":"M","version":"1","mcversion":"1.12.2","authorList":["x"]}]"#,
        )
        .unwrap();
        assert_eq!(a.game_version.as_deref(), Some("1.12.2"));
        let b = read_mcmod_info(br#"{"modList":[{"modid":"n"}]}"#).unwrap();
        assert_eq!(b.name.as_deref(), Some("n"));
    }

    #[test]
    fn thunderstore_and_curseforge_manifests() {
        let t = read_manifest_json(br#"{"name":"Pkg","version_number":"1.0.0","description":"d","dependencies":["a-b-1.0.0"]}"#).unwrap();
        assert_eq!(
            (t.format, t.deps.len()),
            (Some(MetaFormat::Thunderstore), 1)
        );
        let c = read_manifest_json(
            br#"{"manifestType":"minecraftModpack","name":"Pack","version":"2","author":"me",
            "minecraft":{"version":"1.20.1","modLoaders":[{"id":"forge-47.1.0","primary":true}]}}"#,
        )
        .unwrap();
        assert_eq!(
            (c.loader.as_deref(), c.is_pack, c.format),
            (Some("forge"), true, Some(MetaFormat::CurseforgeManifest))
        );
        assert!(read_manifest_json(br#"{"name":"x"}"#).is_none());
    }

    #[test]
    fn prism_components_and_cfg() {
        let m = read_prism(
            Some(br#"{"components":[{"uid":"net.minecraft","version":"1.20.1"},{"uid":"net.fabricmc.fabric-loader","version":"0.15"}]}"#),
            Some("InstanceType=OneSix\nname=My Pack\n"),
        )
        .unwrap();
        assert_eq!(
            (
                m.name.as_deref(),
                m.loader.as_deref(),
                m.game_version.as_deref()
            ),
            (Some("My Pack"), Some("fabric"), Some("1.20.1"))
        );
        assert!(read_prism(None, None).is_none());
    }

    #[test]
    fn plugin_header() {
        let m = read_plugin_header("using Oxide;\nnamespace X {\n[Info(\"Kits\", \"Ann\", \"1.4.2\")]\n[Description(\"Gives kits\")]\nclass Kits {}}").unwrap();
        assert_eq!(
            (m.name.as_deref(), m.version.as_deref(), m.game.as_deref()),
            (Some("Kits"), Some("1.4.2"), Some("rust"))
        );
        assert_eq!(m.description.as_deref(), Some("Gives kits"));
        assert!(read_plugin_header("class A {}").is_none());
    }

    #[test]
    fn inspect_finds_metadata_inside_a_wrapper_folder() {
        let d = tempfile::tempdir().unwrap();
        let z = d.path().join("m.zip");
        write_zip(
            &z,
            &[
                ("wrap/fabric.mod.json", r#"{"id":"x","version":"1"}"#),
                ("wrap/a.class", ""),
            ],
        );
        let src = Source::open(&z).unwrap();
        match inspect(&src, &[]).unwrap() {
            Inspection::Found { meta, file } => {
                assert_eq!(meta.id.as_deref(), Some("x"));
                assert_eq!(file, "wrap/fabric.mod.json");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn inspect_a_folder_with_a_plugin_and_without_anything() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("Kits.cs"), "[Info(\"Kits\", \"A\", \"1\")]").unwrap();
        assert!(matches!(
            inspect(&Source::open(d.path()).unwrap(), &[]).unwrap(),
            Inspection::Found { .. }
        ));
        let e = tempfile::tempdir().unwrap();
        std::fs::write(e.path().join("blob.bin"), "1").unwrap();
        match inspect(&Source::open(e.path()).unwrap(), &[]).unwrap() {
            Inspection::NeedsSelection { tree } => assert_eq!(tree.suggested, vec!["blob.bin"]),
            other => panic!("{other:?}"),
        }
    }
}
