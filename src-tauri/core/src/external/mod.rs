//! External mods: a mod that lives on somebody else's site, added by pasting or
//! dropping its link (or a folder or archive from another launcher).
//!
//! | Module | Owns | Network | Disk |
//! | --- | --- | --- | --- |
//! | [`parse`] | pasted text to a typed [`ExternalRef`] | no | no |
//! | [`provider`] | the [`ModProvider`] trait, the registry, credential state | via [`provider::Fetch`] | no |
//! | [`providers`] | Modrinth, Thunderstore, GitHub, direct URL, and the keyed providers | via `Fetch` | no |
//! | [`meta`] | other launchers' metadata files to [`meta::ExternalMeta`] | no | reads |
//! | [`tree`] | the file-tree model behind the picker | no | reads |
//! | [`matching`] | provider game to detected game and sandbox | no | no |
//! | [`install`] | download, verify, unpack, register, remove | download queue | writes |
//! | [`store`] | the `external_mod` table: where each row came from | no | SQLite |
//! | [`scheme`] | who owns `nxm://`, `curseforge://` ... on this machine | no | OS tools |
//! | [`pending`] | an incoming deep link becomes a confirm request | no | no |
//!
//! THE TWO RULES
//! -------------
//! 1. **A link that arrives from outside never installs.** [`pending::route`]
//!    returns a [`pending::Pending`] for the UI to confirm; only a paste or a
//!    drop the user performed in the window goes straight to [`install`].
//!    This is `deeplink.rs`'s rule, extended to these schemes.
//! 2. **Nothing here holds a key.** A provider that needs one reports
//!    [`CredentialState::NotConfigured`] until the TMC AdminCP says it is on
//!    ([`provider::ProviderAvailability`]).
//!
//! The entry point for `tmc mods add <url>` is [`install::add`]; see
//! `docs/external-mods.md`.

pub mod install;
pub mod matching;
pub mod meta;
pub mod parse;
pub mod pending;
pub mod provider;
pub mod providers;
pub mod scheme;
pub mod store;
pub mod tree;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub use parse::{parse_input, ParseError};
pub use provider::{ModProvider, ProviderAvailability, Registry};

/// The `User-Agent` every request to a provider carries. Modrinth requires an
/// identifying one; the others are owed the same courtesy.
pub fn user_agent() -> String {
    format!(
        "GameSpiceApp/{} (+https://modcommunity.net)",
        env!("CARGO_PKG_VERSION")
    )
}

/// Which service a link belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProviderId {
    Modrinth,
    Thunderstore,
    Github,
    Direct,
    Curseforge,
    Nexus,
    Modio,
    Steam,
}

impl ProviderId {
    pub const ALL: [ProviderId; 8] = [
        Self::Modrinth,
        Self::Thunderstore,
        Self::Github,
        Self::Direct,
        Self::Curseforge,
        Self::Nexus,
        Self::Modio,
        Self::Steam,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Modrinth => "modrinth",
            Self::Thunderstore => "thunderstore",
            Self::Github => "github",
            Self::Direct => "direct",
            Self::Curseforge => "curseforge",
            Self::Nexus => "nexus",
            Self::Modio => "modio",
            Self::Steam => "steam",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.as_str() == raw)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Modrinth => "Modrinth",
            Self::Thunderstore => "Thunderstore",
            Self::Github => "GitHub",
            Self::Direct => "Direct download",
            Self::Curseforge => "CurseForge",
            Self::Nexus => "Nexus Mods",
            Self::Modio => "mod.io",
            Self::Steam => "Steam Workshop",
        }
    }
}

/// A link, understood but not yet looked up.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalRef {
    pub provider: ProviderId,
    /// Slug or id. Thunderstore `namespace/name`, GitHub `owner/repo`, Nexus
    /// `mods/<id>`, CurseForge the addon id or the slug.
    pub project: Option<String>,
    /// A specific version, release tag or file id. `None` means "the latest".
    pub version: Option<String>,
    /// The provider's own game or community: a Thunderstore community, a Nexus
    /// game domain, a CurseForge game slug.
    pub game: Option<String>,
    /// Modrinth project type (`mod`, `shader` ...) or CurseForge class.
    pub class: Option<String>,
    /// The normalised link. Shown to the user and stored on the row.
    pub url: String,
    /// Anything else the link carried (`nxm` key and expiry, a file name).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extras: BTreeMap<String, String>,
}

impl ExternalRef {
    pub fn new(provider: ProviderId, url: impl Into<String>) -> Self {
        Self {
            provider,
            project: None,
            version: None,
            game: None,
            class: None,
            url: url.into(),
            extras: BTreeMap::new(),
        }
    }
}

/// Digests a provider published for a file. Compared strongest first.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hashes {
    pub sha512: Option<String>,
    pub sha256: Option<String>,
    pub sha1: Option<String>,
    pub md5: Option<String>,
}

impl Hashes {
    pub fn is_empty(&self) -> bool {
        self.sha512.is_none() && self.sha256.is_none() && self.sha1.is_none() && self.md5.is_none()
    }
}

/// One downloadable file of a resolved mod.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteFile {
    pub url: String,
    pub name: String,
    pub size: Option<u64>,
    pub hashes: Hashes,
    pub primary: bool,
}

/// What a game needs to know to pick this mod's files.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameHint {
    /// `minecraft`, a Thunderstore community, a Nexus domain ...
    pub game: Option<String>,
    pub loaders: Vec<String>,
    pub game_versions: Vec<String>,
}

/// A dependency, as the provider names it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DepRef {
    pub project: String,
    pub version: Option<String>,
    pub kind: DepKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DepKind {
    Required,
    Optional,
    Incompatible,
    Embedded,
}

/// A link looked up: everything needed to show a confirm screen and install.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Resolved {
    pub provider: ProviderId,
    pub project_id: String,
    pub version_id: String,
    pub title: String,
    pub version: String,
    pub author: Option<String>,
    pub game: GameHint,
    pub files: Vec<RemoteFile>,
    pub deps: Vec<DepRef>,
    /// Where the author's page is, for "open the page".
    pub page_url: Option<String>,
    /// Set when the app may not or cannot download this itself (CurseForge
    /// with distribution off, a Steam Workshop item). The UI opens it instead.
    pub manual_only: Option<String>,
    pub licence: Option<String>,
}

impl Resolved {
    /// The file to install: the one marked primary, else the first.
    pub fn primary_file(&self) -> Option<&RemoteFile> {
        self.files.iter().find(|f| f.primary).or(self.files.first())
    }
}

/// Where an installed external mod came from. Stored on its row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalSource {
    pub provider: ProviderId,
    pub url: String,
    pub project_id: String,
    pub version_id: String,
    pub version: String,
}

/// Whether a provider can be used right now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "state")]
pub enum CredentialState {
    /// Works with no key.
    NotRequired,
    /// A key is present and the provider is on.
    Configured,
    /// Needs a key the TMC AdminCP has not supplied. The UI hides or disables
    /// the provider and shows `reason`.
    NotConfigured { reason: String },
}

impl CredentialState {
    pub fn usable(&self) -> bool {
        !matches!(self, Self::NotConfigured { .. })
    }
}
