//! The providers. Each is a unit struct over a pure JSON-to-[`Resolved`] mapper,
//! so the mapping is tested with captured responses.

use std::sync::Arc;

use super::provider::ModProvider;
use super::{parse_input, ExternalRef, ExternalSource, ProviderId, Services};

pub mod direct;
pub mod github;
pub mod modrinth;
pub mod nexus;
pub mod proxied;
pub mod steam;
pub mod thunderstore;

pub fn standard(services: &Services) -> Vec<Arc<dyn ModProvider>> {
    vec![
        Arc::new(modrinth::Modrinth),
        Arc::new(thunderstore::Thunderstore),
        Arc::new(github::Github),
        Arc::new(direct::Direct),
        Arc::new(proxied::Proxied::new(
            ProviderId::Curseforge,
            services.site.clone(),
        )),
        Arc::new(nexus::Nexus::new(services.credentials.clone())),
        Arc::new(proxied::Proxied::new(
            ProviderId::Modio,
            services.site.clone(),
        )),
        Arc::new(steam::Steam),
    ]
}

/// The reference for an installed mod, rebuilt from its stored link so the
/// game / class the link carried survive an update check. No version: the
/// caller wants the newest.
pub(crate) fn ref_from_installed(installed: &ExternalSource) -> ExternalRef {
    let mut r = parse_input(&installed.url)
        .ok()
        .filter(|r| r.provider == installed.provider)
        .unwrap_or_else(|| ExternalRef::new(installed.provider, installed.url.clone()));
    if r.project.is_none() {
        r.project = Some(installed.project_id.clone());
    }
    r.version = None;
    r
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
