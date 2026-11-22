//! The providers. Each is a unit struct over a pure JSON-to-[`Resolved`] mapper,
//! so the mapping is tested with captured responses.

use std::sync::Arc;

use super::provider::ModProvider;

pub mod direct;
pub mod github;
pub mod keyed;
pub mod modrinth;
pub mod thunderstore;

pub fn standard() -> Vec<Arc<dyn ModProvider>> {
    vec![
        Arc::new(modrinth::Modrinth),
        Arc::new(thunderstore::Thunderstore),
        Arc::new(github::Github),
        Arc::new(direct::Direct),
        Arc::new(keyed::Keyed::curseforge()),
        Arc::new(keyed::Keyed::nexus()),
        Arc::new(keyed::Keyed::modio()),
        Arc::new(keyed::Keyed::steam()),
    ]
}

/// A JSON string field, or `None`.
pub(crate) fn s(v: &serde_json::Value, key: &str) -> Option<String> {
    v.get(key).and_then(|x| x.as_str()).map(str::to_string)
}

/// A JSON array of strings.
pub(crate) fn strs(v: &serde_json::Value, key: &str) -> Vec<String> {
    v.get(key)
        .and_then(|x| x.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}
