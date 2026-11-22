//! The install-manifest checks of `npm run manifests:check`, in Rust.
//!
//! A port of `readJson`, `lintManifest`, `verifySignature` and `lintFixtures`
//! from `scripts/manifests/check.ts` and `sign.ts`. Both sides run the same
//! fixtures (`schemas/fixtures/lint/`): every `ok/*.json` passes and every
//! `bad/<code>[--<case>].json` fails with exactly `<code>`. The problem codes
//! are the contract; messages are for people and may differ in wording.
//!
//! The order is that of the TypeScript check: a text that is not JSON, has
//! no canonical form or is not stored pretty stops there; a document the
//! schema refuses stops there; only then do the rules below run.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use base64::Engine;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde_json::{Map, Value};

use super::canonical::{self, ParseError};
use super::placeholders::{classify, tokens, Placeholder, Position};
use super::schema;

/// One problem, with its stable code and a JSON pointer into the document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub code: String,
    pub at: String,
    pub message: String,
}

impl Problem {
    fn new(code: &str, at: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            at: at.into(),
            message: message.into(),
        }
    }
}

/// One entry of `exec-keys.json`: a key allowed to sign manifests that run code.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecKey {
    pub key_id: String,
    /// 32 bytes of base64.
    pub ed25519: String,
}

/// What a manifest is checked against beyond itself.
#[derive(Debug, Clone, Copy)]
pub struct LintContext<'a> {
    /// The ids of the manifests that exist for this app (for `requires`).
    pub known_ids: &'a HashSet<String>,
    /// The parsed `.sig`, if there is one.
    pub sig: Option<&'a Value>,
    pub keys: &'a [ExecKey],
}

#[derive(Debug, Clone)]
enum Seg {
    Key(String),
    Index(usize),
}

impl Seg {
    fn is(&self, key: &str) -> bool {
        matches!(self, Seg::Key(k) if k == key)
    }
}

fn ptr(at: &[Seg]) -> String {
    at.iter()
        .map(|s| match s {
            Seg::Key(k) => format!("/{}", k.replace('~', "~0").replace('/', "~1")),
            Seg::Index(i) => format!("/{i}"),
        })
        .collect()
}

/// Reads a manifest's text: the codes `json`, `canonical` and `not-pretty`.
/// The document is returned whenever it parsed, as `readJson` does.
pub fn read_manifest(text: &str) -> (Option<Value>, Vec<Problem>) {
    let doc = match canonical::parse(text) {
        Ok(doc) => doc,
        Err(ParseError::Json(m)) => return (None, vec![Problem::new("json", "", m)]),
        Err(ParseError::Canonical(m)) => return (None, vec![Problem::new("canonical", "", m)]),
    };
    let problems = match canonical::canonical_string(&doc).and_then(|_| canonical::pretty_string(&doc)) {
        Err(e) => vec![Problem::new("canonical", "", e.0)],
        Ok(pretty) if pretty != text => vec![Problem::new(
            "not-pretty",
            "",
            "not the pretty form of its canonical form ($schema first, then keys sorted by code point, 4-space indent, trailing newline)",
        )],
        Ok(_) => Vec::new(),
    };
    (Some(doc), problems)
}

/// The schema's problems, as code `schema`.
pub fn schema_problems(doc: &Value) -> Vec<Problem> {
    schema::validate_manifest(doc)
        .into_iter()
        .map(|e| Problem::new("schema", e.at, e.message))
        .collect()
}

fn host_allowed(host: &str, hosts: &[&str]) -> bool {
    hosts.iter().any(|h| match h.strip_prefix('*') {
        Some(suffix) if h.starts_with("*.") => host.ends_with(suffix) && host.len() > suffix.len(),
        _ => host == *h,
    })
}

/// The lint rules the schema cannot express. Run on a schema-valid manifest.
pub fn lint_manifest(doc: &Value, ctx: LintContext<'_>) -> Vec<Problem> {
    let empty = Map::new();
    let mut out = Vec::new();
    let app = doc.get("app").and_then(Value::as_str).unwrap_or_default();
    let is_loader = doc.get("kind").and_then(Value::as_str) == Some("loader");
    let artifacts = doc
        .get("artifacts")
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    let declared = |name: &str| artifacts.get(name).is_some_and(Value::is_object);
    let runtimes: HashSet<&str> = doc
        .pointer("/requirements/runtimes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|r| r.get("kind").and_then(Value::as_str))
        .collect();
    let steps: &[Value] = doc
        .get("steps")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let hosts: Vec<&str> = doc
        .get("downloadHosts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    // Artifact name -> where it is executed (the last place, as a JS Map.set).
    let mut executed: BTreeMap<String, Vec<Seg>> = BTreeMap::new();

    // Placeholders: the closed set, each in its allowed positions.
    let mut scan = |s: &str, at: &[Seg], out: &mut Vec<Problem>| {
        let installer_field = match (at.first(), at.get(1)) {
            (Some(first), Some(Seg::Index(i))) if first.is("steps") => steps
                .get(*i)
                .filter(|s| s.get("action").and_then(Value::as_str) == Some("runInstaller"))
                .map(|_| match at.get(2) {
                    Some(Seg::Key(k)) => k.clone(),
                    Some(Seg::Index(i)) => i.to_string(),
                    None => "undefined".to_string(),
                }),
            _ => None,
        };
        let field = installer_field.as_deref();
        let in_patch = at.first().is_some_and(|s| s.is("deployPatches"))
            && !at.get(2).is_some_and(|s| s.is("path") || s.is("when"));
        for (_, name) in tokens(s) {
            let position = |where_: &str| {
                Problem::new(
                    "placeholder-position",
                    ptr(at),
                    format!("{{{name}}} is only allowed in {where_}"),
                )
            };
            let Some(p) = classify(name) else {
                out.push(Problem::new(
                    "placeholder-unknown",
                    ptr(at),
                    format!("{{{name}}} is not in the closed set"),
                ));
                continue;
            };
            match (p.position(), p) {
                (Position::Anywhere, _) => {}
                (Position::LoaderManifests, _) => {
                    if !is_loader {
                        out.push(position("loader manifests"))
                    }
                }
                (Position::InstallerProgramAndArgs, Placeholder::Artifact(rest)) => {
                    if field != Some("args") && field != Some("program") {
                        out.push(position("runInstaller program and args"))
                    } else if !declared(rest) {
                        out.push(Problem::new(
                            "artifact-undeclared",
                            ptr(at),
                            format!("{{{name}}}: no artifact \"{rest}\" is declared"),
                        ));
                    } else {
                        executed.insert(rest.to_string(), at.to_vec());
                    }
                }
                (Position::InstallerProgram, Placeholder::Runtime(rest)) => {
                    if field != Some("program") {
                        out.push(position("runInstaller program"))
                    } else if !runtimes.contains(rest) {
                        out.push(Problem::new(
                            "runtime-undeclared",
                            ptr(at),
                            format!("{{{name}}}: \"{rest}\" is not in requirements.runtimes"),
                        ));
                    }
                }
                (Position::InstallerArgsAndCwd, _) => {
                    if field != Some("args") && field != Some("cwd") {
                        out.push(position("runInstaller args and cwd"))
                    }
                }
                (Position::DeployPatches, _) => {
                    if !in_patch {
                        out.push(position("deployPatches"))
                    }
                }
                (Position::InstallerProgramAndArgs | Position::InstallerProgram, _) => {
                    unreachable!("artifact and runtime placeholders carry their name")
                }
            }
        }
    };
    fn walk(v: &Value, at: &mut Vec<Seg>, f: &mut dyn FnMut(&str, &[Seg])) {
        match v {
            Value::String(s) => f(s, at),
            Value::Array(items) => {
                for (i, x) in items.iter().enumerate() {
                    at.push(Seg::Index(i));
                    walk(x, at, f);
                    at.pop();
                }
            }
            Value::Object(map) => {
                for (k, x) in map {
                    if k == "x-tmc-placeholder" || (at.is_empty() && k == canonical::SCHEMA_KEY) {
                        continue;
                    }
                    at.push(Seg::Key(k.clone()));
                    walk(x, at, f);
                    at.pop();
                }
            }
            _ => {}
        }
    }
    walk(doc, &mut Vec::new(), &mut |s, at| scan(s, at, &mut out));

    // Artifact references, and the hosts of every fixed URL.
    let check_host = |url: Option<&Value>, at: Vec<Seg>, out: &mut Vec<Problem>| {
        let Some(url) = url.and_then(Value::as_str) else {
            return;
        };
        let authority = url
            .strip_prefix("https://")
            .map(|r| r.split(['/', '?', '#']).next().unwrap_or_default());
        let host = authority.unwrap_or_default().to_lowercase();
        if host.is_empty()
            || !host
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'.' || c == b'-')
        {
            out.push(Problem::new(
                "host-not-allowed",
                ptr(&at),
                format!("the host of {url} must be a literal host name"),
            ));
        } else if !host_allowed(&host, &hosts) {
            out.push(Problem::new(
                "host-not-allowed",
                ptr(&at),
                format!("{host} is not in downloadHosts"),
            ));
        }
    };
    for (key, decl) in artifacts {
        if decl.is_object() {
            let at = vec![
                Seg::Key("artifacts".into()),
                Seg::Key(key.clone()),
                Seg::Key("urlTemplate".into()),
            ];
            check_host(decl.get("urlTemplate"), at, &mut out);
        }
    }
    let step_at = |i: usize, rest: &[Seg]| {
        let mut at = vec![Seg::Key("steps".into()), Seg::Index(i)];
        at.extend_from_slice(rest);
        at
    };
    for (i, step) in steps.iter().enumerate() {
        if !step.is_object() {
            continue;
        }
        if let Some(name) = step.get("artifact").and_then(Value::as_str) {
            if !declared(name) {
                out.push(Problem::new(
                    "artifact-undeclared",
                    ptr(&step_at(i, &[Seg::Key("artifact".into())])),
                    format!("no artifact \"{name}\" is declared"),
                ));
            }
        }
        check_host(
            step.get("url"),
            step_at(i, &[Seg::Key("url".into())]),
            &mut out,
        );
        for (j, m) in step
            .get("mirrors")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .enumerate()
        {
            check_host(
                Some(m),
                step_at(i, &[Seg::Key("mirrors".into()), Seg::Index(j)]),
                &mut out,
            );
        }
    }

    // An executed artifact's URL is fixed by the (signed) template.
    let exec = steps
        .iter()
        .any(|s| s.get("action").and_then(Value::as_str) == Some("runInstaller"));
    for (key, at) in &executed {
        let decl = &artifacts[key.as_str()];
        if exec && decl.get("urlTemplate").and_then(Value::as_str).is_none() {
            out.push(Problem::new(
                "exec-artifact-no-url-template",
                ptr(at),
                format!(
                    "artifact \"{key}\" is executed, so it must declare its URL as urlTemplate"
                ),
            ));
        }
    }

    if is_loader {
        let loader = doc
            .get("loader")
            .and_then(Value::as_str)
            .unwrap_or("undefined");
        let want = format!("{app}/loader/{loader}");
        if doc.get("id").and_then(Value::as_str) != Some(want.as_str()) {
            out.push(Problem::new(
                "loader-id-mismatch",
                "/id",
                format!("a loader's id must be {want}"),
            ));
        }
    }

    for (i, r) in doc
        .get("requires")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
    {
        let Some(r) = r.as_str() else { continue };
        if !r.starts_with(&format!("{app}/")) {
            out.push(Problem::new(
                "requires-other-app",
                format!("/requires/{i}"),
                format!("{r} belongs to another app than {app}"),
            ));
        } else if !ctx.known_ids.contains(r) {
            out.push(Problem::new(
                "requires-unknown",
                format!("/requires/{i}"),
                format!("no manifest {r} exists"),
            ));
        }
    }

    if exec || ctx.sig.is_some() {
        match ctx.sig {
            None => out.push(Problem::new(
                "signature-missing",
                "",
                "a manifest with runInstaller needs a .sig next to it",
            )),
            Some(sig) => out.extend(verify_signature(doc, sig, ctx.keys)),
        }
    }
    out
}

/// Exactly `n` bytes of strict base64 that re-encodes to the same text.
fn b64_bytes<const N: usize>(s: Option<&Value>) -> Option<[u8; N]> {
    let s = s?.as_str()?;
    let body = s.trim_end_matches('=');
    let shape = !body.is_empty()
        && s.len() - body.len() <= 2
        && body
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'+' || c == b'/');
    if !shape {
        return None;
    }
    let engine = base64::engine::general_purpose::STANDARD;
    let bytes = engine.decode(s).ok()?;
    (engine.encode(&bytes) == s).then_some(())?;
    bytes.try_into().ok()
}

/// `None` when `sig` is a valid signature of `doc`'s canonical bytes by a key
/// in `keys`; otherwise `signature-malformed`, `-unknown-key` or `-invalid`.
pub fn verify_signature(doc: &Value, sig: &Value, keys: &[ExecKey]) -> Option<Problem> {
    let malformed = || {
        Some(Problem::new(
            "signature-malformed",
            "",
            "the .sig must be { \"alg\": \"ed25519\", \"keyId\": \"...\", \"signature\": \"<64 bytes of base64>\" }",
        ))
    };
    let Some(s) = sig.as_object() else {
        return Some(Problem::new(
            "signature-malformed",
            "",
            "the .sig is not a JSON object",
        ));
    };
    let extra = s
        .keys()
        .any(|k| !["alg", "keyId", "signature", "x-tmc-placeholder"].contains(&k.as_str()));
    let bytes = b64_bytes::<64>(s.get("signature"));
    let key_id = s.get("keyId").and_then(Value::as_str);
    let (Some(bytes), Some(key_id), false, Some("ed25519")) =
        (bytes, key_id, extra, s.get("alg").and_then(Value::as_str))
    else {
        return malformed();
    };
    let Some(entry) = keys.iter().find(|k| k.key_id == key_id) else {
        return Some(Problem::new(
            "signature-unknown-key",
            "",
            format!("signed by {key_id}, which is not in exec-keys.json"),
        ));
    };
    let ok = (|| {
        let raw = b64_bytes::<32>(Some(&Value::String(entry.ed25519.clone())))?;
        let key = VerifyingKey::from_bytes(&raw).ok()?;
        let message = canonical::canonical_bytes(doc).ok()?;
        key.verify(&message, &Signature::from_bytes(&bytes)).ok()
    })()
    .is_some();
    (!ok).then(|| {
        Problem::new(
            "signature-invalid",
            "",
            format!("the signature by {key_id} does not match this manifest's canonical bytes"),
        )
    })
}

/// The whole check of one manifest text, in the TypeScript order.
pub fn check_manifest(text: &str, ctx: LintContext<'_>) -> Vec<Problem> {
    let (doc, mut problems) = read_manifest(text);
    let Some(doc) = doc.filter(|_| problems.is_empty()) else {
        return problems;
    };
    problems = schema_problems(&doc);
    if problems.is_empty() && doc.is_object() {
        problems = lint_manifest(&doc, ctx);
    }
    problems
}

/// Where the shared lint fixtures live, relative to the repository root.
pub const FIXTURES: &str = "schemas/fixtures/lint";

/// One fixture's outcome.
#[derive(Debug, Clone)]
pub struct FixtureResult {
    pub file: PathBuf,
    /// The code a bad fixture must produce (its file name up to `--`); `None` for ok/.
    pub expected: Option<String>,
    pub problems: Vec<Problem>,
}

impl FixtureResult {
    /// Whether the fixture behaved: ok/ has no problems, bad/ has exactly its code.
    pub fn passed(&self) -> bool {
        let codes: HashSet<&str> = self.problems.iter().map(|p| p.code.as_str()).collect();
        match &self.expected {
            None => codes.is_empty(),
            Some(code) => codes.len() == 1 && codes.contains(code.as_str()),
        }
    }
}

/// Lints every fixture under `<root>/schemas/fixtures/lint/`, as `lintFixtures`.
pub fn lint_fixtures(root: &Path) -> std::io::Result<Vec<FixtureResult>> {
    let dir = root.join(FIXTURES);
    let keys: Vec<ExecKey> =
        serde_json::from_str(&std::fs::read_to_string(dir.join("keys/exec-keys.json"))?)
            .map_err(std::io::Error::other)?;
    let list = |sub: &str| -> std::io::Result<Vec<PathBuf>> {
        let mut files: Vec<PathBuf> = std::fs::read_dir(dir.join(sub))?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "json"))
            .collect();
        files.sort();
        Ok(files)
    };
    let ok = list("ok")?;
    let bad = list("bad")?;
    let mut known_ids = HashSet::new();
    for f in &ok {
        if let Ok(doc) = canonical::parse(&std::fs::read_to_string(f)?) {
            if let Some(id) = doc.get("id").and_then(Value::as_str) {
                known_ids.insert(id.to_string());
            }
        }
    }
    let mut results = Vec::new();
    for file in ok.iter().chain(&bad) {
        let text = std::fs::read_to_string(file)?;
        let sig_path = PathBuf::from(format!("{}.sig", file.display()));
        let sig = match std::fs::read_to_string(&sig_path) {
            // An unreadable .sig is a malformed one, not a missing one.
            Ok(t) => Some(canonical::parse(&t).unwrap_or(Value::Null)),
            Err(_) => None,
        };
        let problems = check_manifest(
            &text,
            LintContext {
                known_ids: &known_ids,
                sig: sig.as_ref(),
                keys: &keys,
            },
        );
        let expected = bad.contains(file).then(|| {
            let stem = file
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default();
            stem.split("--").next().unwrap_or_default().to_string()
        });
        results.push(FixtureResult {
            file: file.clone(),
            expected,
            problems,
        });
    }
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    #[test]
    fn every_lint_fixture_behaves_as_in_the_typescript_check() {
        let results = lint_fixtures(&root()).unwrap();
        let failed: Vec<String> = results
            .iter()
            .filter(|r| !r.passed())
            .map(|r| {
                format!(
                    "{} expected {:?}, got {:?}",
                    r.file.display(),
                    r.expected,
                    r.problems
                )
            })
            .collect();
        assert!(failed.is_empty(), "{}", failed.join("\n"));
        assert!(
            results.iter().filter(|r| r.expected.is_none()).count() >= 3,
            "ok/ fixtures ran"
        );
        let bad = results.iter().filter(|r| r.expected.is_some()).count();
        assert!(bad >= 30, "bad/ fixtures ran: {bad}");
    }

    #[test]
    fn wildcard_hosts_match_subdomains_only() {
        assert!(host_allowed("a.example.com", &["*.example.com"]));
        assert!(!host_allowed("example.com", &["*.example.com"]));
        assert!(!host_allowed("badexample.com", &["*.example.com"]));
        assert!(host_allowed("example.com", &["example.com"]));
    }

    #[test]
    fn base64_must_be_strict() {
        let s = |t: &str| Some(Value::String(t.into()));
        assert!(b64_bytes::<3>(s("AAAA").as_ref()).is_some());
        assert!(b64_bytes::<3>(s("AAA A").as_ref()).is_none());
        assert!(
            b64_bytes::<2>(s("AAB=").as_ref()).is_none(),
            "non-zero trailing bits do not re-encode"
        );
        assert!(b64_bytes::<3>(s("").as_ref()).is_none());
    }
}
