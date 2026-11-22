//! The closed set of install-manifest placeholders, and where each may appear.
//!
//! The rules are those of `docs/loader-manifests.md` ("Placeholders") and of
//! `lintManifest` in `scripts/manifests/check.ts`: anything that looks like a
//! name in braces is a placeholder and must be known. Expansion is a single
//! pass over the closed set, so a value that itself contains `{...}` is never
//! expanded again.

use std::collections::HashMap;

/// One known placeholder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placeholder<'a> {
    GameVersion,
    Platform,
    Side,
    LaunchVersionId,
    Var(&'a str),
    Loader,
    LoaderVersion,
    Artifact(&'a str),
    Runtime(&'a str),
    LayerFiles,
    LayerWork,
    SandboxId,
    SandboxName,
    TargetDir,
}

/// Where a placeholder is allowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Position {
    /// Every string of the manifest.
    Anywhere,
    /// Anywhere in a loader manifest, nowhere in a version manifest.
    LoaderManifests,
    /// `runInstaller.program` and `runInstaller.args`.
    InstallerProgramAndArgs,
    /// `runInstaller.program` only.
    InstallerProgram,
    /// `runInstaller.args` and `runInstaller.cwd`.
    InstallerArgsAndCwd,
    /// The patch side of `deployPatches` (not its `path` or `when`).
    DeployPatches,
}

impl Placeholder<'_> {
    pub fn position(&self) -> Position {
        match self {
            Self::GameVersion
            | Self::Platform
            | Self::Side
            | Self::LaunchVersionId
            | Self::Var(_) => Position::Anywhere,
            Self::Loader | Self::LoaderVersion => Position::LoaderManifests,
            Self::Artifact(_) => Position::InstallerProgramAndArgs,
            Self::Runtime(_) => Position::InstallerProgram,
            Self::LayerFiles | Self::LayerWork => Position::InstallerArgsAndCwd,
            Self::SandboxId | Self::SandboxName | Self::TargetDir => Position::DeployPatches,
        }
    }
}

/// `^[A-Za-z][A-Za-z0-9]{0,31}$`
pub fn is_ident(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b.len() <= 32
        && b[0].is_ascii_alphabetic()
        && b.iter().all(u8::is_ascii_alphanumeric)
}

/// `^[a-z0-9-]{1,32}$`
pub fn is_key(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 32
        && s.bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}

/// Classifies the name inside a token; `None` is "not in the closed set".
pub fn classify(name: &str) -> Option<Placeholder<'_>> {
    let mut parts = name.split('.');
    let head = parts.next().unwrap_or_default();
    let rest = parts.next();
    if parts.next().is_some() {
        return None;
    }
    Some(match (head, rest) {
        ("gameVersion", None) => Placeholder::GameVersion,
        ("platform", None) => Placeholder::Platform,
        ("side", None) => Placeholder::Side,
        ("launchVersionId", None) => Placeholder::LaunchVersionId,
        ("var", Some(r)) if is_ident(r) => Placeholder::Var(r),
        ("loader", None) => Placeholder::Loader,
        ("loaderVersion", None) => Placeholder::LoaderVersion,
        ("artifact", Some(r)) if is_ident(r) => Placeholder::Artifact(r),
        ("runtime", Some(r)) if is_key(r) => Placeholder::Runtime(r),
        ("layerFiles", None) => Placeholder::LayerFiles,
        ("layerWork", None) => Placeholder::LayerWork,
        ("sandboxId", None) => Placeholder::SandboxId,
        ("sandboxName", None) => Placeholder::SandboxName,
        ("targetDir", None) => Placeholder::TargetDir,
        _ => return None,
    })
}

/// Every token in `s`: `\{([A-Za-z][A-Za-z0-9_.-]*)\}`, left to right, not
/// overlapping. Yields the byte range of the whole token and the name.
pub fn tokens(s: &str) -> impl Iterator<Item = (std::ops::Range<usize>, &str)> {
    let b = s.as_bytes();
    let mut i = 0;
    std::iter::from_fn(move || {
        while i < b.len() {
            if b[i] == b'{' && i + 1 < b.len() && b[i + 1].is_ascii_alphabetic() {
                let mut j = i + 2;
                while j < b.len()
                    && (b[j].is_ascii_alphanumeric() || matches!(b[j], b'_' | b'.' | b'-'))
                {
                    j += 1;
                }
                if j < b.len() && b[j] == b'}' {
                    let start = i;
                    i = j + 1;
                    return Some((start..j + 1, &s[start + 1..j]));
                }
            }
            i += 1;
        }
        None
    })
}

/// A placeholder that has no value here: unknown, or not available at this
/// point (an `{artifact.x}` that was never downloaded, a loader placeholder
/// in a version install).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{{{0}}} has no value here")]
pub struct Unresolved(pub String);

/// The values of the placeholders available to one install, by token name
/// (`gameVersion`, `var.javaMajor`, `artifact.installer`, ...).
#[derive(Debug, Clone, Default)]
pub struct Values(pub HashMap<String, String>);

impl Values {
    pub fn set(&mut self, name: impl Into<String>, value: impl Into<String>) {
        self.0.insert(name.into(), value.into());
    }

    /// Expands every token of `s` in one pass. A token with no value is an
    /// error rather than being left in place, so a typo never reaches a path.
    pub fn expand(&self, s: &str) -> Result<String, Unresolved> {
        let mut out = String::with_capacity(s.len());
        let mut last = 0;
        for (range, name) in tokens(s) {
            let value = self
                .0
                .get(name)
                .ok_or_else(|| Unresolved(name.to_string()))?;
            out.push_str(&s[last..range.start]);
            out.push_str(value);
            last = range.end;
        }
        out.push_str(&s[last..]);
        Ok(out)
    }

    /// Expands every string inside a JSON value (keys are left alone).
    pub fn expand_json(&self, v: &serde_json::Value) -> Result<serde_json::Value, Unresolved> {
        use serde_json::Value;
        Ok(match v {
            Value::String(s) => Value::String(self.expand(s)?),
            Value::Array(items) => Value::Array(
                items
                    .iter()
                    .map(|x| self.expand_json(x))
                    .collect::<Result<_, _>>()?,
            ),
            Value::Object(map) => Value::Object(
                map.iter()
                    .map(|(k, x)| Ok((k.clone(), self.expand_json(x)?)))
                    .collect::<Result<_, Unresolved>>()?,
            ),
            other => other.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_match_the_typescript_regex() {
        let found: Vec<_> = tokens("{{a} x{b.c-d_e} {1} {} {f")
            .map(|(_, n)| n)
            .collect();
        assert_eq!(found, ["a", "b.c-d_e"]);
    }

    #[test]
    fn the_closed_set() {
        assert_eq!(classify("gameVersion"), Some(Placeholder::GameVersion));
        assert_eq!(
            classify("var.javaMajor"),
            Some(Placeholder::Var("javaMajor"))
        );
        assert_eq!(classify("runtime.java"), Some(Placeholder::Runtime("java")));
        assert_eq!(
            classify("artifact.installer"),
            Some(Placeholder::Artifact("installer"))
        );
        assert_eq!(classify("var.a.b"), None);
        assert_eq!(classify("var"), None);
        assert_eq!(classify("runtime.Java"), None);
        assert_eq!(classify("gameDir"), None);
        assert_eq!(classify("side.x"), None);
    }

    #[test]
    fn expansion_is_one_pass_and_refuses_unknown_tokens() {
        let mut v = Values::default();
        v.set("gameVersion", "{loader}");
        v.set("loader", "fabric");
        assert_eq!(
            v.expand("mc-{gameVersion}/{loader}").unwrap(),
            "mc-{loader}/fabric"
        );
        assert_eq!(v.expand("{nope}"), Err(Unresolved("nope".into())));
        assert_eq!(v.expand("{} {1}").unwrap(), "{} {1}");
    }
}
