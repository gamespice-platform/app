//! Pasted text to a typed [`ExternalRef`]. Pure: no network, no disk.

use url::Url;

use super::{ExternalRef, ProviderId};

/// Archive-like extensions a bare link may end in and still be a direct download.
pub const DIRECT_EXTENSIONS: [&str; 9] = [
    "zip", "jar", "7z", "rar", "tar.gz", "mrpack", "gma", "cs", "dll",
];

/// Why the text is not a link this module can use. The message is user-facing.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    #[error("nothing to add: the text is empty")]
    Empty,
    #[error("that is not a link")]
    NotALink,
    #[error("plain http links are refused; use the https version of this link")]
    Insecure,
    #[error("unsupported link: {0}")]
    Unsupported(String),
    #[error("the {0} link is incomplete: {1}")]
    Incomplete(&'static str, &'static str),
}

/// Take the first usable line of pasted text: skips blanks and `#` comments,
/// strips quotes and angle brackets, and reads the `URL=` line of a `.url` file.
fn first_candidate(text: &str) -> Option<String> {
    for line in text.lines() {
        let l = line.trim().trim_start_matches('\u{feff}');
        if l.is_empty() || l.starts_with('#') || l.starts_with('[') {
            continue;
        }
        let l = l
            .strip_prefix("URL=")
            .or_else(|| l.strip_prefix("url="))
            .unwrap_or(l);
        let l = l
            .trim()
            .trim_matches(|c| matches!(c, '"' | '\'' | '<' | '>'));
        if !l.is_empty() {
            return Some(l.to_string());
        }
    }
    None
}

fn seg(url: &Url) -> Vec<String> {
    url.path_segments()
        .map(|s| s.filter(|x| !x.is_empty()).map(str::to_string).collect())
        .unwrap_or_default()
}

fn query(url: &Url, key: &str) -> Option<String> {
    url.query_pairs()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, v)| v.into_owned())
        .filter(|v| !v.is_empty())
}

fn host_is(url: &Url, domain: &str) -> bool {
    url.host_str().is_some_and(|h| {
        let h = h.to_ascii_lowercase();
        h == domain || h.ends_with(&format!(".{domain}"))
    })
}

fn has_direct_ext(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    DIRECT_EXTENSIONS
        .iter()
        .any(|e| p.ends_with(&format!(".{e}")))
}

/// Parse pasted text.
pub fn parse_input(text: &str) -> Result<ExternalRef, ParseError> {
    let raw = first_candidate(text).ok_or(ParseError::Empty)?;
    let url = Url::parse(&raw).map_err(|_| ParseError::NotALink)?;
    match url.scheme() {
        "http" => Err(ParseError::Insecure),
        "https" => parse_web(url),
        "modrinth" => parse_modrinth_scheme(&url),
        "ror2mm" => parse_ror2mm(&url),
        "curseforge" => parse_curseforge_scheme(&url),
        "nxm" => parse_nxm(&url),
        "prismlauncher" => parse_prism(&url),
        "steam" => parse_steam_scheme(&url),
        other => Err(ParseError::Unsupported(format!("the {other}:// scheme"))),
    }
}

/// The schemes this module takes from the OS, for deep-link routing.
pub const EXTERNAL_SCHEMES: [&str; 6] = [
    "nxm",
    "curseforge",
    "modrinth",
    "ror2mm",
    "prismlauncher",
    "steam",
];

fn parse_web(url: Url) -> Result<ExternalRef, ParseError> {
    let s = seg(&url);
    if host_is(&url, "modrinth.com") {
        // /mod/<slug>[/version/<id>], /plugin/, /datapack/, /resourcepack/, /shader/, /modpack/
        if s.len() >= 2
            && matches!(
                s[0].as_str(),
                "mod" | "plugin" | "datapack" | "resourcepack" | "shader" | "modpack"
            )
        {
            let mut r = ExternalRef::new(ProviderId::Modrinth, url.as_str());
            r.project = Some(s[1].clone());
            r.class = Some(s[0].clone());
            if s.len() >= 4 && s[2] == "version" {
                r.version = Some(s[3].clone());
            }
            return Ok(r);
        }
        return Err(ParseError::Incomplete(
            "Modrinth",
            "expected modrinth.com/mod/<name>",
        ));
    }
    if host_is(&url, "thunderstore.io") {
        // /package/<ns>/<name>[/<version>/], /c/<community>/p/<ns>/<name>/
        let (game, rest): (Option<String>, &[String]) = match s.iter().position(|x| x == "p") {
            Some(i) if s.first().map(String::as_str) == Some("c") && s.len() > i + 2 => {
                (s.get(1).cloned(), &s[i + 1..])
            }
            _ if s.first().map(String::as_str) == Some("package") => (None, &s[1..]),
            _ => (None, &[][..]),
        };
        if rest.len() >= 2 && rest[0] != "download" {
            let mut r = ExternalRef::new(ProviderId::Thunderstore, url.as_str());
            r.project = Some(format!("{}/{}", rest[0], rest[1]));
            r.version = rest.get(2).cloned();
            r.game = game;
            return Ok(r);
        }
        // /package/download/<ns>/<name>/<ver>/
        if s.len() >= 5 && s[0] == "package" && s[1] == "download" {
            let mut r = ExternalRef::new(ProviderId::Thunderstore, url.as_str());
            r.project = Some(format!("{}/{}", s[2], s[3]));
            r.version = Some(s[4].clone());
            return Ok(r);
        }
        return Err(ParseError::Incomplete(
            "Thunderstore",
            "expected thunderstore.io/package/<owner>/<name>",
        ));
    }
    if host_is(&url, "github.com") && s.len() >= 2 {
        // A release asset (`/releases/download/<tag>/<file>`) is a direct file;
        // `/releases/tag/<tag>` and a bare repo are a release lookup.
        if s.len() >= 6 && s[2] == "releases" && s[3] == "download" {
            let mut r = ExternalRef::new(ProviderId::Github, url.as_str());
            r.project = Some(format!("{}/{}", s[0], s[1]));
            r.version = Some(s[4].clone());
            r.extras.insert("asset".into(), s[5].clone());
            return Ok(r);
        }
        let mut r = ExternalRef::new(ProviderId::Github, url.as_str());
        r.project = Some(format!("{}/{}", s[0], s[1].trim_end_matches(".git")));
        if s.len() >= 5 && s[2] == "releases" && s[3] == "tag" {
            r.version = Some(s[4].clone());
        }
        return Ok(r);
    }
    if host_is(&url, "curseforge.com") {
        // /minecraft/mc-mods/<slug>[/files/<id>]
        if s.len() >= 3 {
            let mut r = ExternalRef::new(ProviderId::Curseforge, url.as_str());
            r.game = Some(s[0].clone());
            r.class = Some(s[1].clone());
            r.project = Some(s[2].clone());
            if s.len() >= 5 && s[3] == "files" {
                r.version = Some(s[4].clone());
            }
            return Ok(r);
        }
        return Err(ParseError::Incomplete(
            "CurseForge",
            "expected curseforge.com/<game>/<class>/<project>",
        ));
    }
    if host_is(&url, "nexusmods.com") {
        // /<game>/mods/<id>
        if s.len() >= 3 && s[1] == "mods" {
            let mut r = ExternalRef::new(ProviderId::Nexus, url.as_str());
            r.game = Some(s[0].clone());
            r.project = Some(s[2].clone());
            if let Some(f) = query(&url, "file_id") {
                r.version = Some(f);
            }
            return Ok(r);
        }
        return Err(ParseError::Incomplete(
            "Nexus",
            "expected nexusmods.com/<game>/mods/<id>",
        ));
    }
    if host_is(&url, "steamcommunity.com") {
        if let Some(id) = query(&url, "id").filter(|i| i.bytes().all(|b| b.is_ascii_digit())) {
            let mut r = ExternalRef::new(ProviderId::Steam, url.as_str());
            r.project = Some(id);
            return Ok(r);
        }
        return Err(ParseError::Incomplete(
            "Steam Workshop",
            "expected ?id=<number>",
        ));
    }
    if host_is(&url, "mod.io") {
        // /g/<game>/m/<mod>
        if s.len() >= 4 && s[0] == "g" && s[2] == "m" {
            let mut r = ExternalRef::new(ProviderId::Modio, url.as_str());
            r.game = Some(s[1].clone());
            r.project = Some(s[3].clone());
            return Ok(r);
        }
        return Err(ParseError::Incomplete(
            "mod.io",
            "expected mod.io/g/<game>/m/<mod>",
        ));
    }
    if has_direct_ext(url.path()) {
        let mut r = ExternalRef::new(ProviderId::Direct, url.as_str());
        r.extras
            .insert("file".into(), s.last().cloned().unwrap_or_default());
        return Ok(r);
    }
    Err(ParseError::Unsupported(format!(
        "{} is not a mod site this app knows, and the link does not end in a file such as .zip or .jar",
        url.host_str().unwrap_or("that site")
    )))
}

/// `modrinth://mod/<slug>[/<version>]`, also `modrinth://project/<slug>`.
fn parse_modrinth_scheme(url: &Url) -> Result<ExternalRef, ParseError> {
    let mut parts: Vec<String> = Vec::new();
    if let Some(h) = url.host_str() {
        parts.push(h.to_string());
    }
    parts.extend(seg(url));
    if parts.len() >= 2 {
        let mut r = ExternalRef::new(ProviderId::Modrinth, url.as_str());
        r.class = Some(parts[0].clone());
        r.project = Some(parts[1].clone());
        r.version = parts.get(2).cloned();
        return Ok(r);
    }
    Err(ParseError::Incomplete(
        "Modrinth",
        "expected modrinth://mod/<name>",
    ))
}

/// `ror2mm://v1/install/thunderstore.io/<ns>/<name>/<version>/`.
fn parse_ror2mm(url: &Url) -> Result<ExternalRef, ParseError> {
    let mut parts: Vec<String> = Vec::new();
    if let Some(h) = url.host_str() {
        parts.push(h.to_string());
    }
    parts.extend(seg(url));
    let i = parts
        .iter()
        .position(|p| p == "install")
        .ok_or(ParseError::Incomplete(
            "ror2mm",
            "expected .../install/thunderstore.io/<owner>/<name>/<version>",
        ))?;
    let rest = &parts[i + 1..];
    // Optional leading domain.
    let rest = if rest.first().is_some_and(|d| d.contains('.')) {
        &rest[1..]
    } else {
        rest
    };
    if rest.len() < 2 {
        return Err(ParseError::Incomplete(
            "ror2mm",
            "owner and package name are missing",
        ));
    }
    let mut r = ExternalRef::new(ProviderId::Thunderstore, url.as_str());
    r.project = Some(format!("{}/{}", rest[0], rest[1]));
    r.version = rest.get(2).cloned();
    Ok(r)
}

/// `curseforge://install?addonId=<id>&fileId=<id>`.
fn parse_curseforge_scheme(url: &Url) -> Result<ExternalRef, ParseError> {
    let addon =
        query(url, "addonId").ok_or(ParseError::Incomplete("CurseForge", "addonId is missing"))?;
    let mut r = ExternalRef::new(ProviderId::Curseforge, url.as_str());
    r.project = Some(addon);
    r.version = query(url, "fileId");
    Ok(r)
}

/// `nxm://<game>/mods/<id>/files/<fid>?key=&expires=&user_id=`.
fn parse_nxm(url: &Url) -> Result<ExternalRef, ParseError> {
    let game = url
        .host_str()
        .ok_or(ParseError::Incomplete("Nexus", "the game is missing"))?
        .to_string();
    let s = seg(url);
    if s.len() < 2 || s[0] != "mods" {
        return Err(ParseError::Incomplete(
            "Nexus",
            "expected nxm://<game>/mods/<id>/files/<id>",
        ));
    }
    let mut r = ExternalRef::new(ProviderId::Nexus, url.as_str());
    r.game = Some(game);
    r.project = Some(s[1].clone());
    if s.len() >= 4 && s[2] == "files" {
        r.version = Some(s[3].clone());
    }
    for k in ["key", "expires", "user_id"] {
        if let Some(v) = query(url, k) {
            r.extras.insert(k.into(), v);
        }
    }
    Ok(r)
}

/// `prismlauncher://install?url=<https link>` or the CurseForge form of it.
fn parse_prism(url: &Url) -> Result<ExternalRef, ParseError> {
    if let Some(inner) = query(url, "url") {
        let mut r = parse_input(&inner)?;
        r.extras.insert("via".into(), "prismlauncher".into());
        return Ok(r);
    }
    if query(url, "addonId").is_some() {
        let mut r = parse_curseforge_scheme(url)?;
        r.extras.insert("via".into(), "prismlauncher".into());
        return Ok(r);
    }
    Err(ParseError::Incomplete(
        "Prism",
        "expected ?url=<link> or ?addonId=<id>",
    ))
}

/// `steam://url/CommunityFilePage/<id>` and `steam://workshop/...`.
fn parse_steam_scheme(url: &Url) -> Result<ExternalRef, ParseError> {
    let id = seg(url)
        .into_iter()
        .rev()
        .find(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
        .ok_or(ParseError::Incomplete("Steam Workshop", "no item id"))?;
    let mut r = ExternalRef::new(ProviderId::Steam, url.as_str());
    r.project = Some(id);
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(t: &str) -> ExternalRef {
        parse_input(t).unwrap_or_else(|e| panic!("{t}: {e}"))
    }

    #[test]
    fn modrinth_urls_and_scheme() {
        let r = ok("https://modrinth.com/mod/sodium");
        assert_eq!(
            (r.provider, r.project.as_deref()),
            (ProviderId::Modrinth, Some("sodium"))
        );
        assert_eq!(r.class.as_deref(), Some("mod"));
        let r = ok("https://modrinth.com/mod/sodium/version/abc123");
        assert_eq!(r.version.as_deref(), Some("abc123"));
        let r = ok("modrinth://mod/sodium");
        assert_eq!(r.project.as_deref(), Some("sodium"));
        assert!(parse_input("https://modrinth.com/").is_err());
    }

    #[test]
    fn thunderstore_urls_and_ror2mm() {
        let r = ok("https://thunderstore.io/package/bbepis/BepInExPack/5.4.2100/");
        assert_eq!(r.project.as_deref(), Some("bbepis/BepInExPack"));
        assert_eq!(r.version.as_deref(), Some("5.4.2100"));
        let r = ok("https://thunderstore.io/c/riskofrain2/p/Owner/Pkg/");
        assert_eq!(
            (r.game.as_deref(), r.project.as_deref()),
            (Some("riskofrain2"), Some("Owner/Pkg"))
        );
        let r = ok("ror2mm://v1/install/thunderstore.io/Owner/Pkg/1.2.3/");
        assert_eq!(r.project.as_deref(), Some("Owner/Pkg"));
        assert_eq!(r.version.as_deref(), Some("1.2.3"));
        assert!(parse_input("ror2mm://v1/install/thunderstore.io/Owner").is_err());
    }

    #[test]
    fn github_release_and_asset() {
        let r = ok("https://github.com/a/b/releases/tag/v1.0");
        assert_eq!(
            (r.project.as_deref(), r.version.as_deref()),
            (Some("a/b"), Some("v1.0"))
        );
        let r = ok("https://github.com/a/b/releases/download/v1.0/mod.zip");
        assert_eq!(r.extras.get("asset").map(String::as_str), Some("mod.zip"));
        let r = ok("https://github.com/a/b");
        assert_eq!(r.version, None);
    }

    #[test]
    fn keyed_providers_parse() {
        let r = ok("https://www.curseforge.com/minecraft/mc-mods/jei/files/123");
        assert_eq!(
            (r.provider, r.project.as_deref(), r.version.as_deref()),
            (ProviderId::Curseforge, Some("jei"), Some("123"))
        );
        let r = ok("curseforge://install?addonId=238222&fileId=4567");
        assert_eq!(
            (r.project.as_deref(), r.version.as_deref()),
            (Some("238222"), Some("4567"))
        );
        let r = ok("https://www.nexusmods.com/skyrimspecialedition/mods/266");
        assert_eq!(
            (r.game.as_deref(), r.project.as_deref()),
            (Some("skyrimspecialedition"), Some("266"))
        );
        let r = ok("nxm://skyrimspecialedition/mods/266/files/1234?key=K&expires=99&user_id=7");
        assert_eq!(r.version.as_deref(), Some("1234"));
        assert_eq!(r.extras.get("key").map(String::as_str), Some("K"));
        let r = ok("https://mod.io/g/snowrunner/m/some-mod");
        assert_eq!(
            (r.provider, r.game.as_deref()),
            (ProviderId::Modio, Some("snowrunner"))
        );
        let r = ok("https://steamcommunity.com/sharedfiles/filedetails/?id=1234567");
        assert_eq!(
            (r.provider, r.project.as_deref()),
            (ProviderId::Steam, Some("1234567"))
        );
        let r = ok("steam://url/CommunityFilePage/1234567");
        assert_eq!(r.project.as_deref(), Some("1234567"));
    }

    #[test]
    fn prism_wraps_other_links() {
        let r = ok("prismlauncher://install?url=https%3A%2F%2Fmodrinth.com%2Fmod%2Fsodium");
        assert_eq!(r.provider, ProviderId::Modrinth);
        assert_eq!(
            r.extras.get("via").map(String::as_str),
            Some("prismlauncher")
        );
        let r = ok("prismlauncher://install?platform=curseforge&addonId=1&fileId=2");
        assert_eq!(r.provider, ProviderId::Curseforge);
    }

    #[test]
    fn direct_files_and_fallbacks() {
        let r = ok("https://example.com/files/pack.ZIP");
        assert_eq!(r.provider, ProviderId::Direct);
        assert_eq!(r.extras.get("file").map(String::as_str), Some("pack.ZIP"));
        assert!(matches!(
            parse_input("https://example.com/page"),
            Err(ParseError::Unsupported(_))
        ));
    }

    #[test]
    fn refuses_and_cleans() {
        assert_eq!(
            parse_input("http://example.com/a.zip"),
            Err(ParseError::Insecure)
        );
        assert_eq!(parse_input("   \n# c\n"), Err(ParseError::Empty));
        assert_eq!(parse_input("not a link"), Err(ParseError::NotALink));
        assert!(matches!(
            parse_input("ftp://x/y"),
            Err(ParseError::Unsupported(_))
        ));
        let r = ok("# note\n\n  \"<https://modrinth.com/mod/sodium>\"  \nsecond");
        assert_eq!(r.project.as_deref(), Some("sodium"));
        let r = ok("[InternetShortcut]\nURL=https://modrinth.com/mod/lithium\n");
        assert_eq!(r.project.as_deref(), Some("lithium"));
    }
}
