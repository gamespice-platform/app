//! Validates an install manifest against `schemas/loader-manifest.v1.schema.json`.
//!
//! The schema file is the one source of truth (the TypeScript check runs it
//! through ajv); this is a small interpreter for exactly the JSON Schema
//! 2020-12 keywords that file uses, so the two cannot drift apart. A keyword
//! the interpreter does not know fails the [`tests::every_keyword_is_known`]
//! test rather than being silently ignored.
//!
//! Like `schemaProblems` in check.ts, a step that matches no `oneOf` branch
//! reports the errors of the branch its `action` names, not of all twelve.

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};

use regex::Regex;
use serde_json::Value;

/// The manifest schema, as shipped in the repository.
pub const MANIFEST_SCHEMA: &str =
    include_str!("../../../../schemas/loader-manifest.v1.schema.json");

/// One schema error: where in the document, and what.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaError {
    pub at: String,
    pub message: String,
}

/// The keywords [`validate`] understands; annotations it may ignore. The
/// schema test holds the shipped schema to these.
#[cfg(test)]
const KEYWORDS: &[&str] = &[
    "$ref",
    "type",
    "const",
    "enum",
    "pattern",
    "minLength",
    "maxLength",
    "minimum",
    "maximum",
    "items",
    "minItems",
    "maxItems",
    "uniqueItems",
    "required",
    "properties",
    "additionalProperties",
    "propertyNames",
    "maxProperties",
    "unevaluatedProperties",
    "allOf",
    "anyOf",
    "oneOf",
    "not",
    "if",
    "then",
];
#[cfg(test)]
const ANNOTATIONS: &[&str] = &["$schema", "$id", "$defs", "title", "description"];

fn manifest_schema() -> &'static Value {
    static SCHEMA: OnceLock<Value> = OnceLock::new();
    SCHEMA
        .get_or_init(|| serde_json::from_str(MANIFEST_SCHEMA).expect("the manifest schema is JSON"))
}

/// Validates `doc` as an install manifest. Empty means valid.
pub fn validate_manifest(doc: &Value) -> Vec<SchemaError> {
    validate(manifest_schema(), doc)
}

/// Validates `doc` against `schema` (whose `$ref`s resolve inside itself).
pub fn validate(schema: &Value, doc: &Value) -> Vec<SchemaError> {
    let v = Validator { root: schema };
    let mut errors = Vec::new();
    v.check(schema, doc, "", &mut errors);
    let mut seen = HashSet::new();
    errors.retain(|e| seen.insert((e.at.clone(), e.message.clone())));
    errors
}

fn regex(pattern: &str) -> Option<Regex> {
    static CACHE: OnceLock<Mutex<HashMap<String, Option<Regex>>>> = OnceLock::new();
    let mut cache = CACHE
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    cache
        .entry(pattern.to_string())
        .or_insert_with(|| Regex::new(pattern).ok())
        .clone()
}

fn pointer_token(k: &str) -> String {
    k.replace('~', "~0").replace('/', "~1")
}

fn json_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(a, b)| json_eq(a, b))
        }
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len()
                && x.iter()
                    .all(|(k, a)| y.get(k).is_some_and(|b| json_eq(a, b)))
        }
        _ => a == b,
    }
}

fn is_integer(v: &Value) -> bool {
    match v {
        Value::Number(n) => {
            n.is_i64()
                || n.is_u64()
                || n.as_f64()
                    .is_some_and(|f| f.is_finite() && f.fract() == 0.0)
        }
        _ => false,
    }
}

fn type_ok(name: &str, v: &Value) -> bool {
    match name {
        "object" => v.is_object(),
        "array" => v.is_array(),
        "string" => v.is_string(),
        "boolean" => v.is_boolean(),
        "null" => v.is_null(),
        "number" => v.is_number(),
        "integer" => is_integer(v),
        _ => false,
    }
}

struct Validator<'a> {
    root: &'a Value,
}

impl<'a> Validator<'a> {
    fn resolve(&self, reference: &str) -> Option<&'a Value> {
        reference
            .strip_prefix('#')
            .and_then(|p| self.root.pointer(p))
    }

    /// Validates `v` against `s`, appending errors, and returns the names of
    /// the properties `s` evaluated (for `unevaluatedProperties`).
    fn check(
        &self,
        s: &'a Value,
        v: &Value,
        at: &str,
        out: &mut Vec<SchemaError>,
    ) -> HashSet<String> {
        let mut evaluated = HashSet::new();
        let err = |out: &mut Vec<SchemaError>, message: String| {
            out.push(SchemaError {
                at: at.to_string(),
                message,
            })
        };
        let s = match s {
            Value::Bool(true) => return evaluated,
            Value::Bool(false) => {
                err(out, "boolean schema is false".into());
                return evaluated;
            }
            Value::Object(s) => s,
            _ => return evaluated,
        };

        if let Some(r) = s.get("$ref").and_then(Value::as_str) {
            match self.resolve(r) {
                Some(target) => evaluated.extend(self.check(target, v, at, out)),
                None => err(out, format!("can't resolve reference {r}")),
            }
        }
        if let Some(t) = s.get("type").and_then(Value::as_str) {
            if !type_ok(t, v) {
                err(out, format!("must be {t}"));
                return evaluated;
            }
        }
        if let Some(c) = s.get("const") {
            if !json_eq(c, v) {
                err(out, format!("must be equal to constant ({c})"));
            }
        }
        if let Some(Value::Array(options)) = s.get("enum") {
            if !options.iter().any(|o| json_eq(o, v)) {
                err(
                    out,
                    format!(
                        "must be equal to one of the allowed values ({})",
                        Value::Array(options.clone())
                    ),
                );
            }
        }

        if let Value::String(text) = v {
            let len = text.chars().count() as u64;
            if let Some(min) = s.get("minLength").and_then(Value::as_u64) {
                if len < min {
                    err(out, format!("must NOT have fewer than {min} characters"));
                }
            }
            if let Some(max) = s.get("maxLength").and_then(Value::as_u64) {
                if len > max {
                    err(out, format!("must NOT have more than {max} characters"));
                }
            }
            if let Some(p) = s.get("pattern").and_then(Value::as_str) {
                if !regex(p).is_some_and(|re| re.is_match(text)) {
                    err(out, format!("must match pattern \"{p}\""));
                }
            }
        }

        if let Value::Number(n) = v {
            let f = n.as_f64().unwrap_or(f64::NAN);
            if let Some(min) = s.get("minimum").and_then(Value::as_f64) {
                if f < min {
                    err(out, format!("must be >= {min}"));
                }
            }
            if let Some(max) = s.get("maximum").and_then(Value::as_f64) {
                if f > max {
                    err(out, format!("must be <= {max}"));
                }
            }
        }

        if let Value::Array(items) = v {
            if let Some(min) = s.get("minItems").and_then(Value::as_u64) {
                if (items.len() as u64) < min {
                    err(out, format!("must NOT have fewer than {min} items"));
                }
            }
            if let Some(max) = s.get("maxItems").and_then(Value::as_u64) {
                if items.len() as u64 > max {
                    err(out, format!("must NOT have more than {max} items"));
                }
            }
            if s.get("uniqueItems") == Some(&Value::Bool(true)) {
                for (j, b) in items.iter().enumerate() {
                    if let Some(i) = items[..j].iter().position(|a| json_eq(a, b)) {
                        err(out, format!("must NOT have duplicate items (items ## {i} and {j} are identical)"));
                        break;
                    }
                }
            }
            if let Some(item) = s.get("items") {
                for (i, x) in items.iter().enumerate() {
                    self.check(item, x, &format!("{at}/{i}"), out);
                }
            }
        }

        if let Value::Object(map) = v {
            if let Some(Value::Array(required)) = s.get("required") {
                for r in required.iter().filter_map(Value::as_str) {
                    if !map.contains_key(r) {
                        err(out, format!("must have required property '{r}' (\"{r}\")"));
                    }
                }
            }
            if let Some(max) = s.get("maxProperties").and_then(Value::as_u64) {
                if map.len() as u64 > max {
                    err(out, format!("must NOT have more than {max} properties"));
                }
            }
            if let Some(names) = s.get("propertyNames") {
                for k in map.keys() {
                    let mut sub = Vec::new();
                    self.check(names, &Value::String(k.clone()), at, &mut sub);
                    if !sub.is_empty() {
                        err(out, format!("property name '{k}' is invalid"));
                    }
                }
            }
            let props = s.get("properties").and_then(Value::as_object);
            if let Some(props) = props {
                for (k, sub) in props {
                    if let Some(x) = map.get(k) {
                        self.check(sub, x, &format!("{at}/{}", pointer_token(k)), out);
                        evaluated.insert(k.clone());
                    }
                }
            }
            if let Some(additional) = s.get("additionalProperties") {
                for (k, x) in map {
                    if props.is_some_and(|p| p.contains_key(k)) {
                        continue;
                    }
                    if additional == &Value::Bool(false) {
                        err(
                            out,
                            format!("must NOT have additional properties (\"{k}\")"),
                        );
                    } else {
                        self.check(additional, x, &format!("{at}/{}", pointer_token(k)), out);
                    }
                    evaluated.insert(k.clone());
                }
            }
        }

        if let Some(Value::Array(all)) = s.get("allOf") {
            for sub in all {
                evaluated.extend(self.check(sub, v, at, out));
            }
        }
        if let Some(Value::Array(any)) = s.get("anyOf") {
            let mut passed = false;
            for sub in any {
                let mut tmp = Vec::new();
                let props = self.check(sub, v, at, &mut tmp);
                if tmp.is_empty() {
                    passed = true;
                    evaluated.extend(props);
                }
            }
            if !passed {
                err(out, "must match a schema in anyOf".into());
            }
        }
        if let Some(Value::Array(one)) = s.get("oneOf") {
            let mut passing = Vec::new();
            let mut failures = Vec::new();
            for (i, sub) in one.iter().enumerate() {
                let mut tmp = Vec::new();
                let props = self.check(sub, v, at, &mut tmp);
                if tmp.is_empty() {
                    passing.push(i);
                    evaluated.extend(props);
                } else {
                    failures.push((sub, tmp));
                }
            }
            match passing.len() {
                1 => {}
                0 => match self.named_branch(&failures, v) {
                    Some(errors) => out.extend(errors),
                    None => err(out, "must match exactly one schema in oneOf".into()),
                },
                _ => err(
                    out,
                    format!("must match exactly one schema in oneOf (passingSchemas {passing:?})"),
                ),
            }
        }
        if let Some(not) = s.get("not") {
            let mut tmp = Vec::new();
            self.check(not, v, at, &mut tmp);
            if tmp.is_empty() {
                err(out, "must NOT be valid".into());
            }
        }
        if let Some(cond) = s.get("if") {
            let mut tmp = Vec::new();
            let props = self.check(cond, v, at, &mut tmp);
            if tmp.is_empty() {
                evaluated.extend(props);
                if let Some(then) = s.get("then") {
                    evaluated.extend(self.check(then, v, at, out));
                }
            }
        }

        if s.get("unevaluatedProperties") == Some(&Value::Bool(false)) {
            if let Value::Object(map) = v {
                for k in map.keys().filter(|k| !evaluated.contains(*k)) {
                    err(
                        out,
                        format!("must NOT have unevaluated properties (\"{k}\")"),
                    );
                }
                evaluated.extend(map.keys().cloned());
            }
        }
        evaluated
    }

    /// The errors of the `oneOf` branch whose `action` constant is the
    /// document's `action`, when there is exactly one such branch.
    fn named_branch(
        &self,
        failures: &[(&'a Value, Vec<SchemaError>)],
        v: &Value,
    ) -> Option<Vec<SchemaError>> {
        let action = v.get("action")?.as_str()?;
        let mut named = failures.iter().filter(|(sub, _)| {
            let branch = sub
                .get("$ref")
                .and_then(Value::as_str)
                .and_then(|r| self.resolve(r))
                .unwrap_or(sub);
            branch
                .pointer("/properties/action/const")
                .and_then(Value::as_str)
                == Some(action)
        });
        let (_, errors) = named.next()?;
        named.next().is_none().then(|| errors.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn walk_keywords(v: &Value, unknown: &mut Vec<String>) {
        let Value::Object(map) = v else { return };
        for (k, x) in map {
            if !KEYWORDS.contains(&k.as_str()) && !ANNOTATIONS.contains(&k.as_str()) {
                unknown.push(k.clone());
            }
            match k.as_str() {
                "properties" | "$defs" => {
                    for sub in x.as_object().into_iter().flat_map(|m| m.values()) {
                        walk_keywords(sub, unknown);
                    }
                }
                "allOf" | "anyOf" | "oneOf" => {
                    for sub in x.as_array().into_iter().flatten() {
                        walk_keywords(sub, unknown);
                    }
                }
                "const" | "enum" | "required" => {}
                _ => walk_keywords(x, unknown),
            }
        }
    }

    #[test]
    fn every_keyword_is_known() {
        let mut unknown = Vec::new();
        walk_keywords(manifest_schema(), &mut unknown);
        assert!(
            unknown.is_empty(),
            "keywords the interpreter does not implement: {unknown:?}"
        );
    }

    #[test]
    fn every_pattern_compiles() {
        fn patterns(v: &Value, out: &mut Vec<String>) {
            match v {
                Value::Object(m) => {
                    for (k, x) in m {
                        match (k.as_str(), x) {
                            ("pattern", Value::String(p)) => out.push(p.clone()),
                            _ => patterns(x, out),
                        }
                    }
                }
                Value::Array(a) => a.iter().for_each(|x| patterns(x, out)),
                _ => {}
            }
        }
        let mut all = Vec::new();
        patterns(manifest_schema(), &mut all);
        assert!(all.len() >= 10);
        for p in all {
            assert!(regex(&p).is_some(), "{p}");
        }
    }

    #[test]
    fn a_step_reports_only_its_own_branch() {
        let doc = serde_json::json!({
            "schemaVersion": 1, "id": "demo/version", "revision": 1, "kind": "version", "app": "demo",
            "title": "t", "steps": [{ "action": "mkdir", "path": { "root": "gameDir", "path": "x" } }]
        });
        let errors = validate_manifest(&doc);
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert_eq!(errors[0].at, "/steps/0/path/root");
    }
}
