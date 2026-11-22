# External mods

Mods from sites other than TMC: Modrinth, Thunderstore, GitHub releases, direct `.zip`/`.jar` links, and (once enabled) CurseForge, Nexus and mod.io. Phase 1 is the Rust core and its IPC; there is no screen yet. Code: `src-tauri/core/src/external/`.

## Rules

1. A link that arrives from the OS (`nxm://`, `curseforge://`, `modrinth://`, `ror2mm://`, `prismlauncher://`) never installs. `external::pending::route` turns it into a `Pending` and `lib.rs` emits `tmc://external-link` so a screen can ask. A paste or drop is the user's own act and installs directly.
2. This app never replaces another app's link handler. `external::scheme::set` refuses when something else owns the scheme unless `override_existing` is true, and nothing registers a scheme except an explicit `external_scheme_set`.
3. No keys in the source. CurseForge, Nexus and mod.io report `NotConfigured` until the AdminCP flag exists.
4. No download cap by default; `externalMaxDownloadMb` in settings sets one, 0 is unlimited. Over the cap fails with a message naming the setting.

## Flow

`parse_input` (pure) -> `ModProvider::resolve` (network via `Fetch`) -> `install::download` (DownloadManager, then size and hash check) -> `install::install_downloaded` (local-mod store, `Origin::External`, row in `external_mod`) -> optionally `sandbox_add_local`. Deploy, the ledger and the journal are the existing local-mod ones, so removal and sandbox purge need nothing new.

| Provider | Link forms | Download | Verified by |
| --- | --- | --- | --- |
| Modrinth | project, version, `modrinth://` | yes | sha512 |
| Thunderstore | package, `ror2mm://` | yes | size (no hash published) |
| GitHub | repo, release, asset | yes | sha256 `digest` when present |
| Direct | `.zip`, `.jar`, `.rar`... | yes | `#sha256=` fragment if given |
| CurseForge, Nexus, mod.io | web and protocol links | not until enabled | n/a |
| Steam Workshop | item link | hand-off: open the page | n/a |

sha1 and md5 are not computed; a file with only those is verified by size.

## Entry point for `tmc mods add <url>`

`external::install::add(text, fetch, registry, avail, ctx, dm, work_root, max_mb, env, target)` returns `AddOutcome::{Installed, OpenPage, Unavailable}`. The CLI lives in another repo; it calls this through the core or the app IPC `external_install`.

## IPC

`external_parse`, `external_resolve`, `external_install`, `external_remove`, `external_inspect_path` (a vault token, never a path), `external_scheme_status`, `external_scheme_set`. Typed in `src/lib/external/`.

## Known limits

- `ProviderAvailability` is a stub (`TODO(site)`): the site has no endpoint yet. `from_json` documents the expected shape.
- `DownloadManager` has its own hard 32 GiB per-file ceiling, so "unlimited" means up to that.
- macOS cannot read or reassign link handlers from here; scheme status is `unknown` there. Linux needs the desktop file to declare the scheme MIME type.
- Needs-selection installs (no metadata found) return the file tree but the selected-files install is not wired yet.
