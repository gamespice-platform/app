//! Deep-link policy: a link that arrives from the OS never installs anything.
//!
//! Anyone can put an `nxm://` or `curseforge://` link on a web page, so the
//! app only ever turns one into a [`Pending`] for the user to confirm. A paste
//! or drop is the user's own act and goes straight to `install::add`.

use serde::{Deserialize, Serialize};

use super::parse::{parse_input, EXTERNAL_SCHEMES};
use super::{ExternalRef, ProviderId};

/// A link waiting for the user's yes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum Pending {
    /// Understood; show the confirm screen with these details.
    Confirm {
        provider: ProviderId,
        label: String,
        url: String,
        reference: ExternalRef,
    },
    /// An external-scheme link that could not be understood. Shown, not acted on.
    Rejected { url: String, reason: String },
}

/// Is this one of the schemes handled here?
pub fn is_external_link(url: &str) -> bool {
    url.split_once(':')
        .is_some_and(|(s, _)| EXTERNAL_SCHEMES.contains(&s.to_ascii_lowercase().as_str()))
}

/// Route an OS-delivered link. `None` means it is not ours (an account link,
/// an app link) and the caller carries on. Never installs, never fetches.
pub fn route(url: &str) -> Option<Pending> {
    if !is_external_link(url) {
        return None;
    }
    Some(match parse_input(url) {
        Ok(r) => Pending::Confirm {
            provider: r.provider,
            label: r.provider.label().to_string(),
            url: url.to_string(),
            reference: r,
        },
        Err(e) => Pending::Rejected {
            url: url.to_string(),
            reason: e.to_string(),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_external_schemes_are_taken() {
        assert!(route("tmc://auth?code=1").is_none());
        assert!(route("https://modrinth.com/mod/x").is_none());
        assert!(route("").is_none());
    }

    #[test]
    fn external_links_become_pending_never_installs() {
        match route("nxm://skyrimspecialedition/mods/1234/files/5678").unwrap() {
            Pending::Confirm { provider, .. } => assert_eq!(provider, ProviderId::Nexus),
            other => panic!("{other:?}"),
        }
        match route("CURSEFORGE://install?addonId=1&fileId=2").unwrap() {
            Pending::Confirm { provider, .. } => assert_eq!(provider, ProviderId::Curseforge),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_broken_external_link_is_rejected_visibly() {
        assert!(matches!(route("nxm://").unwrap(), Pending::Rejected { .. }));
    }
}
