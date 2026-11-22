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

`external_parse`, `external_resolve`, `external_install`, `external_remove`, `external_inspect_path` (a vault token, never a path), `external_scheme_status`, `external_scheme_set`, `external_install_path` (install a chosen subset of an inspected token: `selected` paths, minimal cover, the core re-checks each against the tree), and the credential commands below. Typed in `src/lib/external/`.

## Credentials

Two kinds, never mixed:

- **Our keys (CurseForge, mod.io)** live in the TMC AdminCP and are used by the site. They are never in the app. The app asks the site for metadata and a download URL; the file itself is fetched straight from the provider CDN.
- **The user's own keys (Nexus Mods API key)** are stored in the OS keychain (`keyring`, feature `os-keyring`). With no usable keychain (a Linux box with no Secret Service) they go to an encrypted file in the data dir (XChaCha20-Poly1305, key from a random key file mixed with the machine id) and the app is told it is in fallback mode. A value is read back after every write, so a keychain that accepts and forgets is caught. Keys are never in settings JSON, logs or an IPC answer. Bring-your-own CurseForge and mod.io keys are refused for now.

IPC (none returns a secret; each answers with the same report):

- `external_credential_status` returns `{backend: keychain|encryptedFile|memory, fallback, providers: [{provider, label, configured, source: user|site|none, acceptsUserKey, note}]}`.
- `external_credential_set(provider, value)` stores the user's key.
- `external_credential_clear(provider)` forgets it.

Nexus: the key goes in the `apikey` header. A premium account gets a download link directly. A free account is only given one with the `key` and `expires` from an `nxm://` link ("Download with manager"); without them the answer is "open the page". Those two values and `user_id` are used once and are stripped from the link remembered for the mod.

## Site endpoint contract (for website-city)

Both under `/api/app/v1`, with the user's TMC bearer token (the same auth as every other app call). A signed-out user, a 404 (not deployed yet) or any error means "no keyed provider is on"; the app never guesses. The answer is cached for 5 minutes (30 seconds after a failure).

`GET /external/providers`

```json
{"providers": {"curseforge": true, "modio": false}}
```

A provider is on only when the AdminCP has a key for it and the admin switched it on. Absent means off. The app ignores unknown fields.

`GET /external/resolve?provider=curseforge&project=<id or slug>&version=<file id>&game=<slug>&class=<kind>`

`version`, `game` and `class` are optional; without `version` the site picks the newest file. The answer is the app's `Resolved` object, camelCase, already mapped by the site:

```json
{"provider": "curseforge", "projectId": "238222", "versionId": "4455",
 "title": "Jade", "version": "1.2", "author": "someone",
 "game": {"game": "minecraft", "loaders": ["forge"], "gameVersions": ["1.20.1"]},
 "files": [{"url": "https://edge.forgecdn.net/...", "name": "jade-1.2.jar",
            "size": 123456, "hashes": {"sha1": "..."}, "primary": true}],
 "deps": [], "pageUrl": "https://www.curseforge.com/...",
 "manualOnly": null, "licence": null}
```

Rules the app enforces on the answer: `provider` matches the request; every file URL is `https`; there is at least one file unless `manualOnly` is set. Set `manualOnly` to the project page when the author has turned third-party distribution off. The file URL must not carry our API key (it is fetched from the CDN on the user's machine). Errors use the normal app envelope; a 4xx message is shown to the user.

## Known limits

- Nothing deployed yet answers the two endpoints above, so CurseForge and mod.io report "not enabled" until website-city ships them. All app code is tested against a mock site.
- `external_max_download_mb = 0` is unlimited. The ceiling is carried per request (`tmc.max_bytes`); a request that carries none keeps the 32 GiB default.
- macOS cannot read or reassign link handlers from here; scheme status is `unknown` there. Linux needs the desktop file to declare the scheme MIME type.
- Install progress is indeterminate (a step label, no byte counter); the core reports no per-download progress to this flow yet.
- The drop host handles only the first file of a batch.
- Linux: the app does not add `x-scheme-handler/*` MIME entries to its desktop file. Declaring them at install would make us the default handler where none exists, which contradicts "never register by default"; the Setup dialog registers a scheme only on request.
- The encrypted-file backend has a passphrase mode in core only; no screen asks for one yet.

## Screens (mount points)

Self-contained pieces in `src/components/external/` (barrel `index.ts`) over the logic in `src/lib/external/`. Nothing is mounted by default; the Library > Mods page owner wires them:

- `ExternalIntakeHost` (once, inside the router or above it): window paste of a link, `tmc://external-link` deep links (confirm dialog, explicit Install), and with `handleDrops` an own drop listener. Leave `handleDrops` off while `DropImport` is mounted, or both would handle one drop.
- `PasteExternalLink` and `useExternalIntake`: a labelled field for user-initiated installs, with a progress bar and Undo.
- `SetupExternalDialog` (first run, mode `setup`) and `EditExternalDialog`: schemes nxm, curseforge, modrinth, ror2mm, prismlauncher. Only a free scheme is pre-ticked; replacing another app's handler needs a second explicit tick and a warning.
- `ProviderCredentials`: configured state, backend and fallback warning, set/replace/clear in a password field, never shows a stored value; providers the site has not enabled say "Not enabled".
- `FileSelectionPicker`: for needs-selection installs; tree with tri-state folders, sizes, search, expand/collapse all, presets, column choice, keyboard navigation (arrows, Space, Home/End); sort and columns kept under the `tmc.external.picker.layout` local-storage key.

prismlauncher is now a managed scheme (was recognised only). Tests are pure-logic plus static-markup; the repo has no DOM test environment.
