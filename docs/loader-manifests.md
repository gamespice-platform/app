An install manifest describes, as data, how to install one game version or one loader build into a layer folder. This document covers the JSON Schema, the steps it adds to the plugin step vocabulary, the jail roots they run in, the security model, the trust tiers, where manifests live in this repository, how they move to and from the AdminCP, and the seed manifests. **Status: designed, not built.** It is part of [loaders-and-versions.md](loaders-and-versions.md); the build units are in [loaders-work-split.md](loaders-work-split.md).

## What it is, and how it differs from a plugin

A plugin is data and never code (see [plugins.md](plugins.md)). An install manifest is also data, and most install manifests stay that way: Fabric, Quilt and vanilla only download files and write JSON. Forge and NeoForge, however, can only be installed by running their installer, because its processors patch the game jar at install time. A design that refused to run the installer could not install Forge, and CurseForge-level parity requires installing Forge.

The installer therefore gets exactly one door, the `runInstaller` step. Three rules hold that door:

- `runInstaller` is valid **only** in an install manifest. The plugin loader rejects it at parse time ("runInstaller is not allowed in plugins"), so plugins stay data.
- A manifest containing it is an **exec manifest**. An exec manifest is trusted only if it is compiled into the app or signed offline by a TMC exec key (see "Trust").
- The protection is **provenance, not confinement**, and the UI says so. The installer runs as the user, with network and filesystem access. What makes running it acceptable is the following chain:
  - the exact bytes are pinned by sha256;
  - their URL is fixed by a signed template;
  - the runtime is pinned;
  - the argument vector is closed;
  - the working directory and environment are controlled;
  - the user saw the command line and agreed.
- OS-level confinement, such as running under `bwrap` on Linux or in an AppContainer on Windows, is listed in the gaps.

## Files

| Path | What |
| --- | --- |
| `schemas/loader-manifest.v1.schema.json` | the JSON Schema below, verbatim |
| `schemas/game-catalog.v1.schema.json` | the schema of `catalog.snapshot.json` (the wire `catalog` response plus loader version lists and builds) |
| `manifests/<appSlug>/version.manifest.json` | the game's version template (`<appSlug>/version`) |
| `manifests/<appSlug>/loaders/<loaderKey>.manifest.json` | one template per loader (`<appSlug>/loader/<loaderKey>`) |
| `manifests/<appSlug>/*.sig` | the detached signature next to each exec manifest (`forge.manifest.json.sig`) |
| `manifests/<appSlug>/catalog.snapshot.json` | the exported catalog: versions, loaders, loader versions, builds with bindings and artifacts, for the last-N window. It is compiled in as the offline fallback |
| `manifests/runtimes/<kind>.snapshot.json` | the exported runtimes, compiled in |
| `manifests/keys/exec-keys.json` | the public exec keys: `[{ "keyId": "tmc-exec-2026", "ed25519": "<base64>" }]`, compiled in |
| `scripts/manifests/check.ts` | `npm run manifests:check`: the schema, the lint rules below, and that each signature matches |
| `scripts/manifests/sign.ts` | `npm run manifests:sign -- <file>`: signs with the key at `$TMC_EXEC_SIGNING_KEY` (an owner-held file, never in a repository or on a server) |
| `scripts/manifests/canonical.ts` | the canonical form, shared by check and sign |

`manifests/` is outside `plugins/` on purpose. Install manifests are not plugins, are not loaded by the plugin registry, and have their own schema version.

## Canonical form

Signatures and idempotency keys are computed over the **canonical bytes**:

- UTF-8 JSON;
- object keys sorted by code point, at every depth;
- no insignificant whitespace;
- numbers as integers (the schema has no floats);
- strings escaped minimally (RFC 8785 JCS);
- the `$schema` key removed.

Repository files are pretty-printed (4-space indent, a trailing newline), and `manifests:check` fails if a file is not the pretty form of its own canonical form. Rust (`tmc_core::canon`, which already exists) and TypeScript (`scripts/manifests/canonical.ts`, `website-city/src/lib/game-catalog/canonical.ts`) must produce identical bytes. A shared fixture, `schemas/fixtures/canonical/*.json` with its `.canonical` file, is tested in all three.

## Placeholders

Strings may contain `{name}`. Unlike plugin steps, an unknown name is a **parse error**, never a literal: the closed set is the whole point.

| Name | Bound from | Allowed in |
| --- | --- | --- |
| `{gameVersion}` | the sandbox's game version | everywhere |
| `{loader}`, `{loaderVersion}` | the sandbox | loader manifests |
| `{platform}`, `{side}` | the app | everywhere |
| `{var.<name>}` | bindings `vars` (game version vars, then build vars, then the manifest's own `vars`) | everywhere |
| `{launchVersionId}` | shorthand for `{var.launchVersionId}` | everywhere |
| `{artifact.<key>}` | the absolute path of the downloaded artifact inside `downloads` | `runInstaller.args` only |
| `{layerFiles}`, `{layerWork}` | absolute paths of the layer's `files/` and `work/` | `runInstaller.args` and `cwd` only |
| `{runtime.<kind>}` | the absolute path of the runtime's entry executable | `runInstaller.program` only |
| `{sandboxId}`, `{sandboxName}` | the sandbox; the name is JSON-string-escaped and limited to 64 characters | `deployPatches` values only |
| `{targetDir}` | the deploy target (the instance folder) | `deployPatches` values only |

Placeholders are expanded once, left to right, and their values are never re-scanned. A value containing `{` therefore cannot inject another placeholder.

## Steps

An install manifest's `steps` may use the existing plugin steps, with these roots: `download` (into `downloads`), `extract`, `copy`, `move`, `mkdir`, `remove`, `writeText` and `patchJson`.

Two changes apply to the existing steps:

- **Roots.** `PathRef.root` gains `layer` (the layer folder being built: `files/` is what gets deployed, and `work/` is scratch space that is never deployed) and `runtime` (read-only). In an install manifest, `gameDir` and `pluginData` are **not** valid. Steps never touch the game folder. Only `deployPatches` do, and through their own path.
- **`download.artifact`.** As an alternative to `url`, a download may name an artifact key. The app fills in the url, mirrors, sha256 and size from the bindings. A download with a `url` must still carry `sha256`; in install manifests, `sha256` is required on every download.

New steps (`plugins::manifest::Step` variants):

| `action` | Fields | Does |
| --- | --- | --- |
| `runInstaller` | `program`, `args[]`, `cwd`, `timeoutSecs`, `expectFiles[]`, `network` | runs one program (see "runInstaller rules") |
| `downloadList` | `list: PathRef`, `pointer`, `urlField`, `pathField`, `sha1Field?`, `sha256Field?`, `sizeField?`, `to: PathRef`, `max` | reads an array from a JSON file inside the jail and downloads each entry into `to`/`path` |
| `mergeJson` | `base: PathRef`, `overlay: PathRef`, `to: PathRef`, `rules[]` | merges two JSON documents with per-pointer rules: `{pointer, mode: "replace" \| "append" \| "prepend" \| "union"}`. The default is a deep merge where the overlay wins. This is the generic form of a version-JSON `inheritsFrom` merge |
| `requireFile` | `path: PathRef`, `sha256?` | fails if the file is missing or differs. Used after `runInstaller` and to assert an upstream shape |

Rules for `downloadList`:

- **The list must be pinned.** The `list` file must have been written by an earlier `download` with a pinned sha256 in the same layer, and the executor tracks that provenance. A list produced by `runInstaller` or `writeText` is refused.
- **Hashes.** Entries may carry sha1 only, since that is what Mojang and maven publish, because the list itself is sha256-pinned. An entry with no hash at all is refused.
- **Paths.** Each `path` is jail-checked under `to`.
- **Hosts.** Hosts must be in the manifest's `downloadHosts`.
- **Count.** `max` bounds the entry count. `0` means unlimited, and the default is 10000.

`writeText` gains `ifMissing: bool` (default false), so a file such as a stub `launcher_profiles.json` can be created without overwriting an existing one.

Every step may carry `when: { side?: "client"|"server", platforms?: [..], consent?: "<id>" }`. A step whose `when` does not match is **reported as skipped**, never dropped from the plan the user sees.

## Deploy patches

`deployPatches` is a top-level array, separate from `steps`, because it runs at deploy time against the game folder rather than at layer-build time.

```json
{ "path": "launcher_profiles.json", "ensure": { "profiles": {} }, "pointer": "/profiles/tmc-{sandboxId}", "value": { "name": "TMC: {sandboxName}", "type": "custom", "lastVersionId": "{launchVersionId}", "gameDir": "{targetDir}", "icon": "Furnace" } }
```

- `path` is relative to the deploy target and jail-checked.
- `ensure` is the document written if the file does not exist. Creating it is recorded in the ledger as a file we placed.
- `pointer` names the value to set, and `value` is the JSON to set there. The ledger records the previous value (see [loaders-and-versions.md](loaders-and-versions.md) "Deploy consumption", step 6).
- Patches never delete keys and never touch a file that is not JSON. They fail if `path` exists but is not valid JSON; the file is not overwritten.

## runInstaller rules

```json
{ "action": "runInstaller", "program": "{runtime.java}", "args": ["-jar", "{artifact.installer}", "--installClient", "{layerFiles}"], "cwd": "{layerWork}", "timeoutSecs": 900, "network": true, "expectFiles": [{ "root": "layer", "path": "files/versions/{launchVersionId}/{launchVersionId}.json" }] }
```

1. **Program.** `program` is either `{runtime.<kind>}`, a runtime the manifest lists in `requirements.runtimes`, or `{artifact.<key>}`, a native installer artifact for this platform.
   - Nothing else is allowed: no PATH lookup, no `execSearch`, no absolute path.
   - Runtimes and artifacts are both sha256-verified before the run.
2. **Args.** `args` is a vector. Each element is a literal string or a string whose only placeholders are from the closed set. There is no shell, so nothing goes through `sh -c` or `cmd /c`.
3. **Artifact URL.** An exec manifest's artifact must declare its URL in the manifest itself, as `artifacts.<key>.urlTemplate`. A binding may only supply `sha256` and `size`, plus `mirrors`, which must have the same hash. The database therefore cannot redirect an installer: it can only fail to match.
4. **Working directory.** `cwd` must be `{layerWork}`. Before the run, the layer's `files/` and `work/` exist and are empty except for what earlier steps wrote.
5. **Environment.** The environment is cleared, then given:
   - `PATH`, set to the runtime's `bin` only;
   - `HOME`, `USERPROFILE`, `APPDATA` and `LOCALAPPDATA`, set to `{layerWork}/home`;
   - `TMP`, `TEMP` and `TMPDIR`, set to `{layerWork}/tmp`;
   - `JAVA_HOME`, set when the program is a java runtime;
   - the app's proxy variables, if the user configured a proxy.
   `JAVA_TOOL_OPTIONS`, `_JAVA_OPTIONS` and `CLASSPATH` are never passed. A Java program also gets `-Djava.io.tmpdir` and `-Duser.home` prepended, pointing at the same folders.
6. **Process.** The process starts through `src-tauri/src/spawn.rs`, still the only place a process starts.
   - It has no console window on Windows, and stdin is closed.
   - stdout and stderr go to `{layerWork}/installer.log` and are streamed to the progress UI.
   - On timeout the whole process tree is killed (the job object on Windows, the process group elsewhere). `timeoutSecs` defaults to 900; `0` means no timeout, which the preview shows.
7. **Result.** A non-zero exit fails the step. After a zero exit, every `expectFiles` entry must exist under `layer`.
8. **Network.** `network: true` is shown in the preview ("This installer downloads more files itself"). The Forge and NeoForge installers fetch their libraries and the vanilla jar themselves. That traffic is outside TMC's hash checks. The preview says so, and the layer checks record the hashes of what it produced.
9. **Who may use it.** It is allowed only when the plan's trust is `Compiled` or `Official` and the manifest's `exec` signature verifies. A `Local` (user-imported) manifest containing `runInstaller` is refused at resolve time, before anything is shown as installable.

## Layer checks

These run after the steps finish and before the marker is written. They also run on reuse, against the marker.

- **Jail.** The tree under the `.partial` folder is walked without following links.
  - Any symlink, junction, hard link with a link count above 1 that points outside the layer, device file, FIFO or socket fails the build.
  - Any path that would escape `files/` when merged fails the build. This is the zip-slip defence for files an installer wrote, which the Extract step cannot see.
- **Bounds.** Total size and file count are checked against the app settings `layerMaxBytes` (default 8 GiB) and `layerMaxFiles` (default 200000). `0` means unlimited for both. Breaching a bound fails loudly with the measured numbers.
- **Reserved names.** Paths in `files/` must pass the existing mod-target rules: no `..`, no reserved Windows names, no trailing dots or spaces, and no case-colliding duplicates.
- **Recorded hashes.** Every file's sha256 and size go into `.tmc-layer.json`, which makes layer reuse verifiable.

## Security model, in one place

| Threat | Defence |
| --- | --- |
| A compromised website database points an installer at a malicious jar | The installer URL comes from the signed template (`urlTemplate`), and the signature key is offline. The database supplies only a hash, which must match the bytes at that fixed URL |
| A compromised website signs a malicious exec manifest | It cannot: exec signatures are made offline (`manifests:sign`), and the website only verifies them against `manifests/keys/exec-keys.json`, which is synced to `website-city/src/types/game-catalog/exec-keys.json` |
| A malicious non-exec manifest from the website | It is jailed to the layer, hash-pinned, limited to download hosts, and shown before running. Its worst case is the same as a malicious mod from the same website, which the existing approval and scanning already address |
| A user-imported manifest | It gets `Local` trust: no `runInstaller`, no runtime downloads, every download host needs consent, and the consent dialog says "Local file, unsigned" |
| An installer writes outside the layer | It cannot be prevented (it is not sandboxed), only detected for links and escapes inside the layer, and disclosed in the consent dialog. Confinement is in the gaps |
| Zip-slip in archives | The existing Extract defences, plus the layer checks for installer output |
| Placeholder injection | The closed set, single-pass expansion, values never re-scanned, and the custom version charset |
| Anti-cheat | Loaders carry `onlineNote`, which is shown in consent. TMC injects into no process: usvfs hooks only the game it launches, as today. Installers never target a game executable. `runInstaller` cannot write to `gameDir` because the root is not available |
| Downgrade to an older, vulnerable installer | Builds are withdrawable. The app refuses withdrawn builds at resolve. Consent is per `plan_hash`, so the user is asked again |
| Replay of an old signed manifest revision | The app keeps the highest revision seen per `manifestKey` and refuses a lower one unless the sandbox pinned it before |

## Trust

| Tier | Source | May |
| --- | --- | --- |
| `Compiled` | `manifests/` at build time, verified by `the_compiled_in_manifests_match_the_repository` | everything; exec manifests are still signature-checked, so a repository edit without the key fails CI |
| `Official` | the public API: a PUBLISHED revision, from a site whose origin is the app's configured API base | everything; exec requires a valid signature by a key in `exec-keys.json` |
| `Local` | a file the user imports in Settings → Advanced → "Install manifest from file" | non-exec steps only; every download host is shown and consented |

Community submissions (a user proposing a loader for a game) are **not** a tier at launch. In the future they would go through the site's approval queue (`ApprovalHold*`) and become `Official` only when an admin publishes them, which needs the exec signature for exec. Until then the AdminCP is admin-only, and that is the whole community story. See the gaps.

## Import and export

The repository is the source of truth for **templates** (`*.manifest.json`). The AdminCP is the source of truth for the **catalog** (versions, loaders, builds and artifacts), which is exported to `catalog.snapshot.json` for the offline fallback.

**Idempotency.** A manifest is identified by `id` and `revision`. Its content key is the sha256 of the canonical bytes. Import against the database works as follows:

| Database state | Import result |
| --- | --- |
| no row for `id` | create a DRAFT at `revision` |
| a row with the same sha256 | no-op: return the existing row and its status. Importing the same file twice changes nothing |
| a different sha256 at the same `revision` | refuse with `revision_conflict`, showing a diff. A changed file must bump `revision` |
| `revision` lower than the highest stored | refuse with `stale_revision` |
| `revision` higher | create a DRAFT. Publishing is a separate, MFA-gated step, unless the website CLI was run with `--publish` |

**Paths in.**
- AdminCP: Mods → Loaders → Manifests → "Import JSON". It takes a file and an optional `.sig`.
- website-city CLI: `npm run catalog:import -- --app minecraft --dir ../tmc-app/manifests/minecraft [--publish]`. This is `scripts/catalog-import.ts`, which runs on the site host as the CLI user and writes audit rows with `actorId = null` and `detail.cli = true`.

**Paths out.**
- AdminCP: "Export JSON" for one revision, or "Export catalog snapshot" for an app.
- website-city CLI: `npm run catalog:export -- --app minecraft --out ../tmc-app/manifests/minecraft --for-repo`. It writes:
  - each PUBLISHED template, in pretty canonical form;
  - its `.sig`;
  - `catalog.snapshot.json`.
- `--for-repo` refuses non-prod CDN origins and any unpublished row.

**Round trip.** Export, then `manifests:check`, then import is a no-op. That is the acceptance test.

**Snapshot import.** `catalog.importSnapshot` exists for seeding a fresh site, such as a dev site or a new environment.
- Rows are looked up by natural key: app slug plus versionId, loader key, version, and the build pair.
- Each is then created or updated. `sortKeyManual` rows keep their `sortKey`.
- Artifacts match on owner, key, platform and sha256.
- Nothing is deleted.

## Seed manifests

These are the real step logic for Minecraft in official-launcher mode. The bindings in the wire example of [loaders-and-versions.md](loaders-and-versions.md) show what the catalog supplies.

- Anything not yet verified against the real upstream tool is marked `x-tmc-placeholder` with the reason.
- `manifests:check` allows the marker, but `manifests.publish` and the release CI (`manifests:check --release`) refuse it.
- The seeds are therefore real, and honest about what was not yet run.

`manifests/minecraft/version.manifest.json`

```json
{
    "$schema": "../../schemas/loader-manifest.v1.schema.json",
    "schemaVersion": 1,
    "id": "minecraft/version",
    "revision": 1,
    "kind": "version",
    "app": "minecraft",
    "title": "Minecraft",
    "downloadHosts": ["piston-meta.mojang.com", "piston-data.mojang.com"],
    "artifacts": {
        "versionJson": { "platform": "any", "redistributable": false, "description": "The version JSON Mojang publishes for this version" },
        "serverJar": { "platform": "any", "redistributable": false, "sides": ["server"], "description": "The dedicated server jar" }
    },
    "consents": [
        { "id": "eula", "sides": ["server"], "text": "Running a Minecraft server requires accepting the Minecraft EULA.", "url": "https://aka.ms/MinecraftEULA" }
    ],
    "vars": { "launchVersionId": "{gameVersion}" },
    "requirements": { "runtimes": [{ "kind": "java", "major": "{var.javaMajor}", "for": "launch" }] },
    "steps": [
        { "action": "download", "artifact": "versionJson", "to": { "root": "layer", "path": "files/versions/{gameVersion}/{gameVersion}.json" }, "when": { "side": "client" } },
        { "action": "download", "artifact": "serverJar", "to": { "root": "layer", "path": "files/server.jar" }, "when": { "side": "server" } },
        { "action": "writeText", "path": { "root": "layer", "path": "files/eula.txt" }, "content": "eula=true\n", "when": { "side": "server", "consent": "eula" } }
    ],
    "deployPatches": [
        { "path": "launcher_profiles.json", "ensure": { "profiles": {}, "version": 3 }, "pointer": "/profiles/tmc-{sandboxId}", "value": { "name": "TMC: {sandboxName}", "type": "custom", "lastVersionId": "{launchVersionId}", "gameDir": "{targetDir}", "icon": "Furnace" }, "when": { "side": "client" } }
    ],
    "launch": { "vars": ["launchVersionId"] }
}
```

`manifests/minecraft/loaders/fabric.manifest.json`

```json
{
    "$schema": "../../../schemas/loader-manifest.v1.schema.json",
    "schemaVersion": 1,
    "id": "minecraft/loader/fabric",
    "revision": 1,
    "kind": "loader",
    "app": "minecraft",
    "loader": "fabric",
    "title": "Fabric",
    "requires": ["minecraft/version"],
    "downloadHosts": [],
    "artifacts": {
        "profileJson": { "platform": "any", "sides": ["client"], "sourceTemplate": "https://meta.fabricmc.net/v2/versions/loader/{gameVersion}/{loaderVersion}/profile/json", "description": "Launcher profile JSON, mirrored by the importer" },
        "serverLauncher": { "platform": "any", "sides": ["server"], "sourceTemplate": "https://meta.fabricmc.net/v2/versions/loader/{gameVersion}/{loaderVersion}/{var.installerVersion}/server/jar", "description": "Fabric server launcher jar, mirrored by the importer" }
    },
    "vars": { "launchVersionId": "fabric-loader-{loaderVersion}-{gameVersion}", "serverJar": "fabric-server-launch.jar" },
    "steps": [
        { "action": "download", "artifact": "profileJson", "to": { "root": "layer", "path": "files/versions/{launchVersionId}/{launchVersionId}.json" }, "when": { "side": "client" } },
        { "action": "download", "artifact": "serverLauncher", "to": { "root": "layer", "path": "files/fabric-server-launch.jar" }, "when": { "side": "server" } }
    ],
    "launch": { "vars": ["launchVersionId", "serverJar"] }
}
```

`manifests/minecraft/loaders/quilt.manifest.json` is the same shape, with these differences:
- `id` is `minecraft/loader/quilt`, and `loader` is `quilt`;
- `profileJson.sourceTemplate` is `https://meta.quiltmc.org/v3/versions/loader/{gameVersion}/{loaderVersion}/profile/json`;
- `launchVersionId` is `quilt-loader-{loaderVersion}-{gameVersion}`;
- the server side is a placeholder (`"x-tmc-placeholder": "Quilt servers are installed by quilt-installer 'install server'; needs an exec step and a signed template"`), with no server steps until that is written.

`manifests/minecraft/loaders/forge.manifest.json` (exec; ships with `forge.manifest.json.sig`)

```json
{
    "$schema": "../../../schemas/loader-manifest.v1.schema.json",
    "schemaVersion": 1,
    "id": "minecraft/loader/forge",
    "revision": 1,
    "kind": "loader",
    "app": "minecraft",
    "loader": "forge",
    "title": "Forge",
    "requires": ["minecraft/version"],
    "downloadHosts": ["maven.minecraftforge.net"],
    "artifacts": {
        "installer": { "platform": "any", "urlTemplate": "https://maven.minecraftforge.net/net/minecraftforge/forge/{gameVersion}-{loaderVersion}/forge-{gameVersion}-{loaderVersion}-installer.jar", "description": "The Forge installer" }
    },
    "vars": { "launchVersionId": "{gameVersion}-forge-{loaderVersion}" },
    "requirements": { "runtimes": [{ "kind": "java", "major": "{var.javaMajor}", "for": "install" }], "minGameVersionSortKey": "x-tmc-placeholder: legacy Forge before 1.13 uses another installer layout; the importer only creates builds for 1.13 and later" },
    "steps": [
        { "action": "download", "artifact": "installer", "to": { "root": "downloads", "path": "forge-{gameVersion}-{loaderVersion}-installer.jar" } },
        { "action": "writeText", "path": { "root": "layer", "path": "files/launcher_profiles.json" }, "content": "{\"profiles\":{}}", "ifMissing": true, "when": { "side": "client" } },
        { "action": "runInstaller", "program": "{runtime.java}", "args": ["-jar", "{artifact.installer}", "--installClient", "{layerFiles}"], "cwd": "{layerWork}", "timeoutSecs": 900, "network": true, "expectFiles": [{ "root": "layer", "path": "files/versions/{launchVersionId}/{launchVersionId}.json" }], "when": { "side": "client" } },
        { "action": "remove", "path": { "root": "layer", "path": "files/launcher_profiles.json" }, "when": { "side": "client" } },
        { "action": "runInstaller", "program": "{runtime.java}", "args": ["-jar", "{artifact.installer}", "--installServer", "{layerFiles}"], "cwd": "{layerWork}", "timeoutSecs": 1800, "network": true, "expectFiles": [{ "root": "layer", "path": "files/run.sh" }], "when": { "side": "server" }, "x-tmc-placeholder": "verify expectFiles across 1.17-1.21 server installs; older ones write a forge-*.jar instead of run.sh" }
    ],
    "launch": { "vars": ["launchVersionId"] }
}
```

The `remove` step is there because the stub `launcher_profiles.json` exists only to satisfy the installer's check. The real profile is a deploy patch. The `minGameVersionSortKey` line above is a placeholder in prose: the schema field is `requirements.minGameVersion` (a version id resolved to a sort key by the catalog). The seed must set it to `1.13` once the importer's cut-off is confirmed.

`manifests/minecraft/loaders/neoforge.manifest.json` is the Forge shape, with these differences:
- `id` is `minecraft/loader/neoforge`, and `loader` is `neoforge`;
- `downloadHosts` is `["maven.neoforged.net"]`;
- `installer.urlTemplate` is `https://maven.neoforged.net/releases/net/neoforged/neoforge/{loaderVersion}/neoforge-{loaderVersion}-installer.jar`;
- `launchVersionId` is `neoforge-{loaderVersion}`;
- the `--installClient` and `--installServer` arguments are marked `"x-tmc-placeholder": "confirm the flag names with 'java -jar <installer> --help' for the 20.4, 21.1 and 21.4 lines"`;
- builds are created only for `net/neoforged/neoforge`. The 1.20.1 artefacts live at `net/neoforged/forge` with another URL, which is a gap.

The Java version comes from the game version's `javaMajor` var. The importer reads it from the Mojang version JSON's `javaVersion.majorVersion` and never guesses it. Forge and NeoForge installers run on that runtime. Fabric and Quilt need no runtime at install time.

## The schema

`schemas/loader-manifest.v1.schema.json`, verbatim. Draft 2020-12. Every object is closed (`additionalProperties: false`), except that `x-tmc-placeholder` is allowed on the root, on steps, on artifacts and on requirements.

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://tmc.app/schemas/loader-manifest.v1.schema.json",
  "title": "TMC install manifest v1",
  "type": "object",
  "additionalProperties": false,
  "required": ["schemaVersion", "id", "revision", "kind", "app", "title", "steps"],
  "properties": {
    "$schema": { "type": "string" },
    "schemaVersion": { "const": 1 },
    "id": { "type": "string", "pattern": "^[a-z0-9-]{1,48}/(version|loader/[a-z0-9-]{1,32})$" },
    "revision": { "type": "integer", "minimum": 1 },
    "kind": { "enum": ["version", "loader"] },
    "app": { "$ref": "#/$defs/slug" },
    "loader": { "$ref": "#/$defs/key" },
    "title": { "type": "string", "minLength": 1, "maxLength": 80 },
    "description": { "type": "string", "maxLength": 2000 },
    "requires": { "type": "array", "items": { "type": "string" }, "maxItems": 8, "uniqueItems": true },
    "downloadHosts": { "type": "array", "items": { "$ref": "#/$defs/host" }, "maxItems": 32, "uniqueItems": true },
    "artifacts": { "type": "object", "propertyNames": { "$ref": "#/$defs/ident" }, "additionalProperties": { "$ref": "#/$defs/artifactDecl" }, "maxProperties": 32 },
    "vars": { "type": "object", "propertyNames": { "$ref": "#/$defs/ident" }, "additionalProperties": { "type": "string", "maxLength": 256 }, "maxProperties": 32 },
    "requirements": { "$ref": "#/$defs/requirements" },
    "consents": { "type": "array", "items": { "$ref": "#/$defs/consent" }, "maxItems": 8 },
    "steps": { "type": "array", "items": { "$ref": "#/$defs/step" }, "maxItems": 256 },
    "deployPatches": { "type": "array", "items": { "$ref": "#/$defs/deployPatch" }, "maxItems": 32 },
    "launch": { "type": "object", "additionalProperties": false, "properties": { "vars": { "type": "array", "items": { "$ref": "#/$defs/ident" }, "maxItems": 16, "uniqueItems": true } } },
    "x-tmc-placeholder": { "type": "string" }
  },
  "allOf": [
    { "if": { "properties": { "kind": { "const": "loader" } } }, "then": { "required": ["loader"] } },
    { "if": { "properties": { "kind": { "const": "version" } } }, "then": { "not": { "required": ["loader"] } } }
  ],
  "$defs": {
    "slug": { "type": "string", "pattern": "^[a-z0-9-]{1,48}$" },
    "key": { "type": "string", "pattern": "^[a-z0-9-]{1,32}$" },
    "ident": { "type": "string", "pattern": "^[A-Za-z][A-Za-z0-9]{0,31}$" },
    "host": { "type": "string", "pattern": "^(\\*\\.)?[a-z0-9-]+(\\.[a-z0-9-]+)+$" },
    "platform": { "enum": ["any", "windows-x64", "windows-arm64", "linux-x64", "linux-arm64", "macos-x64", "macos-arm64"] },
    "side": { "enum": ["client", "server"] },
    "relPath": { "type": "string", "minLength": 1, "maxLength": 512, "not": { "pattern": "(^/|^[A-Za-z]:|\\\\|(^|/)\\.\\.(/|$))" } },
    "httpsUrl": { "type": "string", "pattern": "^https://", "maxLength": 2048 },
    "pathRef": {
      "type": "object", "additionalProperties": false, "required": ["root", "path"],
      "properties": { "root": { "enum": ["layer", "downloads", "runtime"] }, "path": { "$ref": "#/$defs/relPath" } }
    },
    "when": {
      "type": "object", "additionalProperties": false,
      "properties": {
        "side": { "$ref": "#/$defs/side" },
        "platforms": { "type": "array", "items": { "$ref": "#/$defs/platform" }, "minItems": 1, "uniqueItems": true },
        "consent": { "$ref": "#/$defs/ident" }
      }
    },
    "artifactDecl": {
      "type": "object", "additionalProperties": false,
      "properties": {
        "platform": { "$ref": "#/$defs/platform" },
        "sides": { "type": "array", "items": { "$ref": "#/$defs/side" }, "minItems": 1, "uniqueItems": true },
        "urlTemplate": { "$ref": "#/$defs/httpsUrl" },
        "sourceTemplate": { "$ref": "#/$defs/httpsUrl" },
        "redistributable": { "type": "boolean" },
        "linkHashPolicy": { "enum": ["pinned", "fetch"] },
        "description": { "type": "string", "maxLength": 200 },
        "x-tmc-placeholder": { "type": "string" }
      }
    },
    "requirements": {
      "type": "object", "additionalProperties": false,
      "properties": {
        "runtimes": { "type": "array", "maxItems": 4, "items": {
          "type": "object", "additionalProperties": false, "required": ["kind", "major", "for"],
          "properties": { "kind": { "$ref": "#/$defs/key" }, "major": { "type": "string", "maxLength": 32 }, "for": { "enum": ["install", "launch", "both"] } } } },
        "minGameVersion": { "type": "string", "maxLength": 64 },
        "maxGameVersion": { "type": "string", "maxLength": 64 },
        "platforms": { "type": "array", "items": { "$ref": "#/$defs/platform" }, "uniqueItems": true },
        "x-tmc-placeholder": { "type": "string" }
      }
    },
    "consent": {
      "type": "object", "additionalProperties": false, "required": ["id", "text"],
      "properties": { "id": { "$ref": "#/$defs/ident" }, "text": { "type": "string", "maxLength": 500 }, "url": { "$ref": "#/$defs/httpsUrl" }, "sides": { "type": "array", "items": { "$ref": "#/$defs/side" }, "uniqueItems": true } }
    },
    "deployPatch": {
      "type": "object", "additionalProperties": false, "required": ["path", "pointer", "value"],
      "properties": {
        "path": { "$ref": "#/$defs/relPath" },
        "ensure": { "type": "object" },
        "pointer": { "type": "string", "pattern": "^(/[^/]+)+$", "maxLength": 256 },
        "value": {},
        "when": { "$ref": "#/$defs/when" }
      }
    },
    "stepCommon": {
      "properties": { "when": { "$ref": "#/$defs/when" }, "label": { "type": "string", "maxLength": 120 }, "x-tmc-placeholder": { "type": "string" } }
    },
    "step": {
      "type": "object", "required": ["action"],
      "oneOf": [
        { "$ref": "#/$defs/stepDownload" }, { "$ref": "#/$defs/stepExtract" }, { "$ref": "#/$defs/stepCopy" },
        { "$ref": "#/$defs/stepMove" }, { "$ref": "#/$defs/stepMkdir" }, { "$ref": "#/$defs/stepRemove" },
        { "$ref": "#/$defs/stepWriteText" }, { "$ref": "#/$defs/stepPatchJson" }, { "$ref": "#/$defs/stepMergeJson" },
        { "$ref": "#/$defs/stepDownloadList" }, { "$ref": "#/$defs/stepRequireFile" }, { "$ref": "#/$defs/stepRunInstaller" }
      ]
    },
    "stepDownload": {
      "type": "object", "unevaluatedProperties": false, "$ref": "#/$defs/stepCommon", "required": ["action", "to"],
      "properties": {
        "action": { "const": "download" },
        "artifact": { "$ref": "#/$defs/ident" },
        "url": { "$ref": "#/$defs/httpsUrl" },
        "sha256": { "type": "string", "pattern": "^[0-9a-f]{64}$" },
        "maxBytes": { "type": "integer", "minimum": 0 },
        "mirrors": { "type": "array", "items": { "$ref": "#/$defs/httpsUrl" }, "maxItems": 8 },
        "to": { "$ref": "#/$defs/pathRef" }
      },
      "oneOf": [ { "required": ["artifact"], "not": { "required": ["url"] } }, { "required": ["url", "sha256"], "not": { "required": ["artifact"] } } ]
    },
    "stepExtract": {
      "type": "object", "unevaluatedProperties": false, "$ref": "#/$defs/stepCommon", "required": ["action", "from", "to"],
      "properties": { "action": { "const": "extract" }, "from": { "$ref": "#/$defs/pathRef" }, "to": { "$ref": "#/$defs/pathRef" }, "strip": { "type": "integer", "minimum": 0, "maximum": 8 }, "stripSingleRoot": { "type": "boolean" }, "include": { "type": "array", "items": { "type": "string", "maxLength": 256 }, "maxItems": 64 } }
    },
    "stepCopy": {
      "type": "object", "unevaluatedProperties": false, "$ref": "#/$defs/stepCommon", "required": ["action", "from", "to"],
      "properties": { "action": { "const": "copy" }, "from": { "$ref": "#/$defs/pathRef" }, "to": { "$ref": "#/$defs/pathRef" } }
    },
    "stepMove": {
      "type": "object", "unevaluatedProperties": false, "$ref": "#/$defs/stepCommon", "required": ["action", "from", "to"],
      "properties": { "action": { "const": "move" }, "from": { "$ref": "#/$defs/pathRef" }, "to": { "$ref": "#/$defs/pathRef" } }
    },
    "stepMkdir": {
      "type": "object", "unevaluatedProperties": false, "$ref": "#/$defs/stepCommon", "required": ["action", "path"],
      "properties": { "action": { "const": "mkdir" }, "path": { "$ref": "#/$defs/pathRef" } }
    },
    "stepRemove": {
      "type": "object", "unevaluatedProperties": false, "$ref": "#/$defs/stepCommon", "required": ["action", "path"],
      "properties": { "action": { "const": "remove" }, "path": { "$ref": "#/$defs/pathRef" } }
    },
    "stepWriteText": {
      "type": "object", "unevaluatedProperties": false, "$ref": "#/$defs/stepCommon", "required": ["action", "path", "content"],
      "properties": { "action": { "const": "writeText" }, "path": { "$ref": "#/$defs/pathRef" }, "content": { "type": "string", "maxLength": 65536 }, "ifMissing": { "type": "boolean" } }
    },
    "stepPatchJson": {
      "type": "object", "unevaluatedProperties": false, "$ref": "#/$defs/stepCommon", "required": ["action", "path", "pointer", "value"],
      "properties": { "action": { "const": "patchJson" }, "path": { "$ref": "#/$defs/pathRef" }, "pointer": { "type": "string", "maxLength": 256 }, "value": {} }
    },
    "stepMergeJson": {
      "type": "object", "unevaluatedProperties": false, "$ref": "#/$defs/stepCommon", "required": ["action", "base", "overlay", "to"],
      "properties": {
        "action": { "const": "mergeJson" }, "base": { "$ref": "#/$defs/pathRef" }, "overlay": { "$ref": "#/$defs/pathRef" }, "to": { "$ref": "#/$defs/pathRef" },
        "rules": { "type": "array", "maxItems": 32, "items": { "type": "object", "additionalProperties": false, "required": ["pointer", "mode"], "properties": { "pointer": { "type": "string", "maxLength": 256 }, "mode": { "enum": ["replace", "append", "prepend", "union"] } } } }
      }
    },
    "stepDownloadList": {
      "type": "object", "unevaluatedProperties": false, "$ref": "#/$defs/stepCommon", "required": ["action", "list", "pointer", "urlField", "pathField", "to"],
      "properties": {
        "action": { "const": "downloadList" }, "list": { "$ref": "#/$defs/pathRef" }, "pointer": { "type": "string", "maxLength": 256 },
        "urlField": { "type": "string", "maxLength": 128 }, "pathField": { "type": "string", "maxLength": 128 },
        "sha1Field": { "type": "string", "maxLength": 128 }, "sha256Field": { "type": "string", "maxLength": 128 }, "sizeField": { "type": "string", "maxLength": 128 },
        "to": { "$ref": "#/$defs/pathRef" }, "max": { "type": "integer", "minimum": 0 }
      },
      "anyOf": [ { "required": ["sha1Field"] }, { "required": ["sha256Field"] } ]
    },
    "stepRequireFile": {
      "type": "object", "unevaluatedProperties": false, "$ref": "#/$defs/stepCommon", "required": ["action", "path"],
      "properties": { "action": { "const": "requireFile" }, "path": { "$ref": "#/$defs/pathRef" }, "sha256": { "type": "string", "pattern": "^[0-9a-f]{64}$" } }
    },
    "stepRunInstaller": {
      "type": "object", "unevaluatedProperties": false, "$ref": "#/$defs/stepCommon", "required": ["action", "program", "args", "cwd"],
      "properties": {
        "action": { "const": "runInstaller" },
        "program": { "type": "string", "pattern": "^\\{(runtime|artifact)\\.[A-Za-z][A-Za-z0-9]{0,31}\\}$" },
        "args": { "type": "array", "items": { "type": "string", "maxLength": 512 }, "maxItems": 32 },
        "cwd": { "const": "{layerWork}" },
        "timeoutSecs": { "type": "integer", "minimum": 0 },
        "network": { "type": "boolean" },
        "expectFiles": { "type": "array", "items": { "$ref": "#/$defs/pathRef" }, "maxItems": 16 }
      }
    }
  }
}
```

The schema alone does not express some rules. `manifests:check`, the website's `src/lib/game-catalog/manifest-lint.ts` and Rust's `layers::lint` all enforce the following, and share one fixture set (`schemas/fixtures/lint/{ok,bad}/*.json`):

- every placeholder is in the closed set and allowed in its position;
- every `artifact` reference and every `{artifact.x}` is declared;
- every `{runtime.x}` is in `requirements.runtimes`;
- every host of a `url` or `urlTemplate` is in `downloadHosts`;
- a manifest containing `runInstaller` has every artifact it executes declared with `urlTemplate`;
- `kind: "loader"` manifests have `id` equal to `<app>/loader/<loader>`;
- `requires` names existing manifests of the same app;
- the `.sig` verifies when `runInstaller` is present.
