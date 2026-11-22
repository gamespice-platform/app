//! The canonical JSON form of an install manifest: the bytes a `.sig` signs.
//!
//! This is a port of `scripts/manifests/canonical.ts` and must stay
//! byte-identical to it (the test below runs both directions over
//! `schemas/fixtures/canonical/`):
//!
//! - object keys sorted by their UTF-8 bytes, no whitespace;
//! - integers only (a safe JS integer; `1e3` and `-0` are integers, `1.5` is not);
//! - strings escape `"`, `\`, `\b \t \n \f \r` by name and every other
//!   character below U+0020 as `\u00xx`; everything else is written raw;
//! - the top-level `$schema` is not part of the canonical form.
//!
//! [`pretty_string`] is the form a manifest is stored in: `$schema` first,
//! then sorted keys, a 4-space indent and a trailing newline.

use serde_json::{Map, Value};

/// A document that has no canonical form (a float, a lone surrogate).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct CanonicalError(pub String);

/// Why [`parse`] refused a text: the problem codes `json` and `canonical` of
/// `npm run manifests:check`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    #[error("{0}")]
    Json(String),
    #[error("{0}")]
    Canonical(String),
}

pub const SCHEMA_KEY: &str = "$schema";

/// The largest integer a JS number holds exactly (`Number.MAX_SAFE_INTEGER`).
const MAX_SAFE: u64 = (1 << 53) - 1;

/// Parses JSON text. `JSON.parse` accepts a lone surrogate escape and the
/// canonical writer then refuses it; serde_json refuses it while parsing, so
/// that error is reported as `canonical`, as the TypeScript check does.
pub fn parse(text: &str) -> Result<Value, ParseError> {
    serde_json::from_str(text).map_err(|e| {
        let message = e.to_string();
        if message.contains("surrogate") || message.contains("end of hex escape") {
            ParseError::Canonical(format!("lone surrogate: {message}"))
        } else {
            ParseError::Json(message)
        }
    })
}

/// The canonical form, without the top-level `$schema`.
pub fn canonical_string(doc: &Value) -> Result<String, CanonicalError> {
    let mut out = String::new();
    match doc {
        Value::Object(map) if map.contains_key(SCHEMA_KEY) => {
            let mut rest = map.clone();
            rest.remove(SCHEMA_KEY);
            write(&Value::Object(rest), "", &mut out)?;
        }
        _ => write(doc, "", &mut out)?,
    }
    Ok(out)
}

/// The bytes a signature covers.
pub fn canonical_bytes(doc: &Value) -> Result<Vec<u8>, CanonicalError> {
    canonical_string(doc).map(String::into_bytes)
}

/// The stored form: `$schema` first at the top, keys sorted, 4-space indent,
/// `": "` between key and value, empty containers as `[]`/`{}`, a final newline.
pub fn pretty_string(doc: &Value) -> Result<String, CanonicalError> {
    let mut out = String::new();
    pretty(doc, "", "", true, &mut out)?;
    out.push('\n');
    Ok(out)
}

fn pointer_token(k: &str) -> String {
    k.replace('~', "~0").replace('/', "~1")
}

/// Keys in UTF-8 byte order. serde_json's map (without `preserve_order`) is
/// a BTreeMap over `String`, whose order is already byte order; sorting here
/// keeps that true if the feature is ever turned on.
fn sorted_keys(map: &Map<String, Value>) -> Vec<&String> {
    let mut keys: Vec<&String> = map.keys().collect();
    keys.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
    keys
}

fn number(n: &serde_json::Number, at: &str) -> Result<String, CanonicalError> {
    let fail = || {
        CanonicalError(format!(
            "{}: {n} is not an integer (the canonical form has no floats)",
            if at.is_empty() { "/" } else { at }
        ))
    };
    if let Some(u) = n.as_u64() {
        return if u <= MAX_SAFE { Ok(u.to_string()) } else { Err(fail()) };
    }
    if let Some(i) = n.as_i64() {
        return if i.unsigned_abs() <= MAX_SAFE { Ok(i.to_string()) } else { Err(fail()) };
    }
    match n.as_f64() {
        // JS has one number type: 1e3 and -0 are integers there.
        Some(f) if f.is_finite() && f.fract() == 0.0 && f.abs() <= MAX_SAFE as f64 => {
            Ok(format!("{}", f as i64))
        }
        _ => Err(fail()),
    }
}

fn escape(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{08}' => out.push_str("\\b"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\u{0c}' => out.push_str("\\f"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

fn write(v: &Value, at: &str, out: &mut String) -> Result<(), CanonicalError> {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => out.push_str(&number(n, at)?),
        Value::String(s) => escape(s, out),
        Value::Array(items) => {
            out.push('[');
            for (i, x) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write(x, &format!("{at}/{i}"), out)?;
            }
            out.push(']');
        }
        Value::Object(map) => {
            out.push('{');
            for (i, k) in sorted_keys(map).into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                escape(k, out);
                out.push(':');
                write(&map[k], &format!("{at}/{}", pointer_token(k)), out)?;
            }
            out.push('}');
        }
    }
    Ok(())
}

fn pretty(v: &Value, indent: &str, at: &str, top: bool, out: &mut String) -> Result<(), CanonicalError> {
    let inner = format!("{indent}    ");
    match v {
        Value::Array(items) if items.is_empty() => out.push_str("[]"),
        Value::Array(items) => {
            out.push_str("[\n");
            for (i, x) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(",\n");
                }
                out.push_str(&inner);
                pretty(x, &inner, &format!("{at}/{i}"), false, out)?;
            }
            out.push('\n');
            out.push_str(indent);
            out.push(']');
        }
        Value::Object(map) if map.is_empty() => out.push_str("{}"),
        Value::Object(map) => {
            let mut keys = sorted_keys(map);
            if top {
                if let Some(i) = keys.iter().position(|k| k.as_str() == SCHEMA_KEY) {
                    let k = keys.remove(i);
                    keys.insert(0, k);
                }
            }
            out.push_str("{\n");
            for (i, k) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push_str(",\n");
                }
                out.push_str(&inner);
                escape(k, out);
                out.push_str(": ");
                pretty(&map[k], &inner, &format!("{at}/{}", pointer_token(k)), false, out)?;
            }
            out.push('\n');
            out.push_str(indent);
            out.push('}');
        }
        scalar => write(scalar, at, out)?,
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixtures() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas/fixtures/canonical")
    }

    #[test]
    fn byte_identical_to_the_typescript_canonicaliser() {
        let mut checked = 0;
        let mut entries: Vec<_> = std::fs::read_dir(fixtures()).unwrap().flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            let expected = std::fs::read(path.with_extension("canonical")).unwrap();
            let doc = parse(&text).unwrap();
            let got = canonical_bytes(&doc).unwrap();
            assert_eq!(
                String::from_utf8_lossy(&got),
                String::from_utf8_lossy(&expected),
                "{}",
                path.display()
            );
            assert_eq!(got, expected, "{}", path.display());
            checked += 1;
        }
        assert_eq!(checked, 6, "every canonical fixture pair is checked");
    }

    #[test]
    fn floats_and_unsafe_integers_have_no_canonical_form() {
        for text in ["{\"a\":1.5}", "[9007199254740992]", "[-9007199254740992]", "[1e300]"] {
            let doc = parse(text).unwrap();
            assert!(canonical_string(&doc).is_err(), "{text}");
        }
        assert_eq!(canonical_string(&parse("[-0, 1e3, 2.0]").unwrap()).unwrap(), "[0,1000,2]");
        let err = canonical_string(&parse("{\"a\":[1.5]}").unwrap()).unwrap_err();
        assert!(err.0.starts_with("/a/0: 1.5 is not an integer"), "{err}");
    }

    #[test]
    fn a_lone_surrogate_is_a_canonical_problem_not_a_json_one() {
        for text in ["\"a \\ud800\"", "\"\\udc00\"", "\"\\ud800\\u0041\""] {
            assert!(matches!(parse(text), Err(ParseError::Canonical(_))), "{text}");
        }
        assert!(matches!(parse("{"), Err(ParseError::Json(_))));
        assert_eq!(parse("\"\\ud83d\\ude00\"").unwrap(), Value::String("\u{1F600}".into()));
    }

    #[test]
    fn pretty_puts_schema_first_only_at_the_top() {
        let doc = parse(r#"{"b":{"$schema":1,"a":[]},"$schema":"x","a":{}}"#).unwrap();
        assert_eq!(
            pretty_string(&doc).unwrap(),
            "{\n    \"$schema\": \"x\",\n    \"a\": {},\n    \"b\": {\n        \"$schema\": 1,\n        \"a\": []\n    }\n}\n"
        );
    }
}
