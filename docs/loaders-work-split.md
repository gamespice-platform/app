How the game version and loader feature of [loaders-and-versions.md](loaders-and-versions.md) and [loader-manifests.md](loader-manifests.md) is split into build units, in what order they land, what each one must prove, and what is knowingly left open. **Status: designed, not built.** Each unit is sized for one builder session. Builders write the "what was built" version of the section their unit covers when they land it.

## Units

Two prefixes are used: `A` units are in `tmc-app` and `W` units in `website-city`. "Owns" means that only this unit edits the file while the feature is in flight. A file another unit also touches is listed under "Touches", with the exact symbol.

### A1: schemas, seed manifests, check and sign (tmc-app, no Rust)

- **Owns:**
  - `schemas/loader-manifest.v1.schema.json` and `schemas/game-catalog.v1.schema.json`;
  - `schemas/fixtures/{canonical,lint}/**`;
  - `manifests/**`: the five seeds, `keys/exec-keys.json` and an empty `catalog.snapshot.json` with `"versions":{"items":[]}`;
  - `scripts/manifests/{check,sign,canonical}.ts`, with a node test beside each.
- **Touches:** `package.json`, adding the scripts `manifests:check` and `manifests:sign`.
- **Depends on:** nothing. Start first.
- **Acceptance:**
  - `npm run manifests:check` passes on the seeds, and `--release` fails listing every `x-tmc-placeholder`;
  - every `lint/bad/*.json` fails with its expected message (the message is in the fixture's file name);
  - the canonical fixtures round-trip byte-identically;
  - sign followed by check verifies with a test key, and a one-byte edit fails;
  - the two placeholder holes this design leaves in `forge.manifest.json` are fixed: `minGameVersionSortKey` is replaced by `requirements.minGameVersion: "1.13"`, with a placeholder note on it.
- **Signing:** the real exec key is the owner's to create. A1 commits the `forge` and `neoforge` `.sig` files signed with a **test** key, listed in `exec-keys.json` as `"keyId": "tmc-exec-test"`, and leaves a placeholder marker on each. Release CI refuses the test key id.

### W1: catalog models and domain library (website-city)

- **Owns:**
  - `prisma/models/{game-version,game-loader,game-loader-version,game-loader-build,install-manifest,install-artifact,install-runtime,game-catalog-audit}.prisma` and one migration;
  - `src/types/game-catalog/*`, plus `loader-manifest.v1.schema.json` and `exec-keys.json` (copies);
  - `src/lib/game-catalog/{ordering,resolve,canonical,signature,manifest-lint,artifact,audit,status}.ts` and tests;
  - `scripts/catalog-schema-sync.ts`.
- **Touches:**
  - `prisma/models/app-install.prisma` (`loaderVersion`) and its relation lines on `App`/`FileUpload`;
  - `src/types/content/addressable.ts` (`NOT_ADDRESSABLE`);
  - the site settings registry (`catalogListDefault`, `catalogListMax`, `catalogCacheSeconds`, `catalogRateLimitPerMinute`, `catalogArtifactMaxBytes`, `catalogLinkMaxBytes`, `catalogLinkHostAllow`);
  - `package.json` (`catalog:schema:sync`).
- **Depends on:** A1's schema file (for the sync), which can be stubbed if A1 is still in flight.
- **Acceptance:**
  - the migration applies on a copy of prod;
  - `resolve()` builds the wire example plan from fixture rows byte-for-byte;
  - the zod mirror and the JSON Schema agree on every fixture;
  - the signature check accepts A1's signed fixture and rejects the edited one;
  - ordering tests cover `1.21.10` against `1.21.9`, a snapshot between two releases, and a manual sortKey surviving a re-import;
  - `npm run typecheck` and `lint` are clean.

### W2: admin procedures, import/export, upload flow (website-city)

- **Owns:**
  - `src/server/api/routers/game-catalog.ts` and its tests;
  - `scripts/catalog-import.ts` and `scripts/catalog-export.ts`;
  - `src/lib/game-catalog/{import,export,upload,snapshot}.ts`;
  - the upload job owner for catalog artifacts in `src/lib/file/jobs.ts`. This one is under "Touches": add the job type in `src/types/build/kind.ts` per the file-upload checklist.
- **Touches:** `src/server/api/routers/admin.ts` (mount the router); `package.json` (`catalog:import` and `catalog:export`).
- **Depends on:** W1.
- **Acceptance:**
  - importing the same file twice gives one row and one audit row;
  - a changed file at the same revision is refused with `revision_conflict`;
  - publishing is refused with a placeholder, with an exec manifest that has no signature, and without MFA;
  - export, then `manifests:check`, then import is a no-op;
  - presign, PUT and commit against the dev storez give an `InstallArtifact` with `scan PENDING`, then `CLEAN`;
  - an over-limit size is refused with the limit named, and `0` allows any size;
  - a link to a host outside `catalogLinkHostAllow` is refused;
  - deleting an artifact referenced by a PUBLISHED build is refused.

### W3: AdminCP Mods tab and i18n (website-city)

- **Owns:**
  - `src/app/[locale]/admin/mods/**`;
  - `src/app/_components/lib/admin/mods/**`;
  - the `mods.*` keys in all nine `locales/*/admin.json`.
- **Touches:** `src/app/_components/lib/admin/wrapper.tsx` (the `TABS` entry and the `modsSec` branch); `src/types/i18n/route-namespaces.ts`.
- **Depends on:** W2.
- **Acceptance:**
  - a Playwright admin spec creates a game version, a loader, a loader version, a build, an upload and a link artifact, imports a manifest, publishes it with MFA, sees it in the public API, then withdraws it;
  - a non-admin gets 404 on every page;
  - a screenshot of each subtab is in light and dark;
  - `i18n:bundle:gen` and `i18n:ns:gen` are clean, and the copy is complete in all nine locales.

### W5: public API, contract, wire doc (website-city)

- **Owns:**
  - `src/app/api/app/v1/apps/[id]/catalog/route.ts`, `.../versions/route.ts`, `.../loaders/[key]/versions/route.ts` and `.../install-plan/route.ts`;
  - `src/app/api/app/v1/runtimes/[kind]/[major]/route.ts`;
  - `docs/api/app-catalog.md`;
  - `scripts/verify-game-catalog.ts`;
  - `src/types/game-catalog/fixtures/*.json`, which holds the wire examples verbatim.
- **Touches:**
  - `src/types/app-api/contract.ts`, adding the catalog section;
  - the rate limit bucket registry (`appCatalog`);
  - `docs/api/README` or the index the site uses;
  - the `AppInstall` sync handler (`loaderVersion`).
- **Depends on:** W1. It runs in parallel with W2 and W3, seeding fixtures through the W1 library rather than W2's procedures.
- **Acceptance:**
  - every fixture equals the live response for seeded rows;
  - a 304 on a matching `If-None-Match`;
  - cursor pagination never repeats or skips across an insert at the head;
  - hidden apps 404;
  - withdrawn returns 410 with its note;
  - the rate limit returns 429 past the bucket, and `0` disables it;
  - `verify:run` lists `game-catalog`.

### W6: upstream importers (website-city)

- **Owns:**
  - `src/lib/game-catalog/importers/{registry,diff}.ts`;
  - `src/lib/game-catalog/importers/minecraft/{mojang,fabric,quilt,forge,neoforge}.ts`;
  - their recorded upstream fixtures under `importers/minecraft/fixtures/`.
- **Touches:** W2's router file, only to add the `sync.preview` and `sync.apply` bodies. Coordinate with W2, or land after it.
- **Depends on:** W1, and W2 for the sync procedures.
- **Acceptance:** each importer, run against its recorded fixture, produces drafts with the right `sortKey`, channel and `javaMajor`.
  - Mojang: `release`→STABLE, `snapshot`→EXPERIMENTAL, a pre-release or release candidate (by the id pattern `-pre\d+|-rc\d+`)→PREVIEW, and `old_*`→LEGACY.
  - Fabric and Quilt produce builds only for game versions inside the window (`catalogListDefault` stable plus the newest 10 experimental). They mirror the profile JSON as uploads with `sourceUrl`.
  - Forge and NeoForge produce one build per version from `maven-metadata.xml`, with `recommended` taken from Forge's promotions JSON. Forge before 1.13 is skipped and logged.
  - A re-run is a no-op, and a manual sortKey survives it.
- **Rate:** importers fetch upstream with a 1 request a second pace and a User-Agent naming the site, not a person.

### A2: catalog client, cache, fallback, IPC (tmc-app)

- **Owns:**
  - `src-tauri/core/src/catalog/{mod,types,client,cache,fallback}.rs`;
  - `src-tauri/src/commands/catalog.rs`;
  - `src/lib/api/fixtures/catalog/*` (synced);
  - the `catalog_*` wrappers in a new `src/lib/ipc/catalog.ts`.
- **Touches:**
  - `src-tauri/core/src/library/db.rs`: one migration step adding `catalog_cache`;
  - `src-tauri/core/build.rs`: compiling in `manifests/**/catalog.snapshot.json`;
  - `src-tauri/src/commands/mod.rs` and the handler list;
  - `src/lib/ipc/schemas.ts`: `Catalog*Schema`;
  - the settings struct (`catalogTtlMinutes`, `versionListCount`).
- **Depends on:** A1, and W5's contract and fixtures, which can be developed against the fixtures before W5 is deployed.
- **Acceptance:**
  - Rust tests serve from cache inside the TTL, revalidate after it (a 304 refreshes), fall back to stale on network error with `stale: true`, fall back to compiled with no cache, and error with no snapshot;
  - the webview test parses every fixture through the zod schemas;
  - `cargo test -p tmc-core catalog` and `npm test` pass.
- **Coordination:** `library/db.rs` migrations are numbered. A2 and A4 both add one, so A2 lands first and takes N+1, and A4 takes N+2.

### A3: executor step extensions and jail roots (tmc-app)

- **Owns:**
  - `src-tauri/core/src/plugins/steps_install.rs` (the new steps);
  - `src-tauri/core/src/layers/lint.rs` (the lint rules over the shared fixtures);
  - `src-tauri/core/src/layers/placeholders.rs` (the closed set).
- **Touches:**
  - `src-tauri/core/src/plugins/manifest.rs`: the `Step` variants `RunInstaller`, `DownloadList`, `MergeJson` and `RequireFile`; `when`; `writeText.if_missing`; `download.artifact`; the `FsRoot` variants `Layer` and `Runtime`; and the plugin parser rejecting install-only steps and roots;
  - `src-tauri/core/src/plugins/jail.rs`: the root resolution for the new roots;
  - `src-tauri/core/src/plugins/steps.rs`: the dispatch to `steps_install`;
  - `src-tauri/src/spawn.rs`: a `spawn_installer` helper with the cleared environment, the kill-on-timeout process tree, and log streaming.
- **Depends on:** A1, for the fixtures.
- **Acceptance:**
  - a plugin containing `runInstaller` fails to load, with the message from the design;
  - an install manifest using `gameDir` fails;
  - `downloadList` refuses an unpinned list and an entry with no hash;
  - `mergeJson` passes a fixture of Mojang `inheritsFrom` (libraries appended, `mainClass` replaced);
  - `runInstaller` runs a test program (a tiny Rust test binary built by the test harness, not Java) and the tests prove:
    - the environment is cleared;
    - `JAVA_TOOL_OPTIONS` is absent;
    - the working directory is `work/`;
    - a timeout kills a child that forks a grandchild;
    - `expectFiles` failing fails the step;
  - every lint fixture behaves as in A1;
  - `cargo clippy` is clean.
- **Coordination:** `src-tauri/usvfs` is being changed by another builder. A3 does not touch it, but must rebase onto it before landing.

### A4: plan, layers, runtimes, deploy patches, launch vars, sandbox fields (tmc-app)

- **Owns:**
  - `src-tauri/core/src/layers/{mod,plan,build,checks,runtime,consent}.rs`;
  - `src-tauri/core/src/deploy/json_patch.rs`.
- **Touches:**
  - `src-tauri/core/src/library/deploy.rs`: the resolve, consent, runtimes and layers phases before the merge, and the patches after it;
  - `src-tauri/core/src/deploy/{engine,merge}.rs`: layer sources as lowest-priority pseudo-mods;
  - `src-tauri/core/src/deploy/ledger.rs`: the `JsonPatch` entry kind and its undo;
  - `src-tauri/core/src/library/sandbox.rs`: `loader_version`, `install_state`, consent records, and the migration function for free text;
  - `src-tauri/core/src/library/db.rs`: migration N+2, with the columns and the normalisation;
  - `src-tauri/core/src/library/{sync,share,export}.rs`: `loaderVersion` on the wire, in share codes and in `.mrpack` dependencies;
  - `src-tauri/core/src/launch.rs`: `launch_vars` into the run context;
  - `plugins/games/minecraft/launch.json`: `{launchVersionId}`;
  - `src-tauri/src/commands/{library,sandbox}.rs`: `install_plan_preview`, `install_plan_consent`, and `NeedsConsent` on deploy;
  - the usvfs overlay folder for patches. That is read in `deploy/` only; the usvfs crate is not touched.
- **Depends on:** A3; A2 for the plan source (or the compiled fallback only, to start).
- **Acceptance:**
  - Rust integration tests against a local HTTP fixture server cover:
    - a Fabric plan building a layer;
    - deploying with each strategy;
    - undo leaving the game folder byte-identical, including `launcher_profiles.json` when the user edited another profile in between;
    - undo leaving a patched value alone if the user changed it;
    - a failed layer build not touching the game folder;
    - a second sandbox reusing the layer, which is not rebuilt;
    - a tampered layer file triggering a rebuild;
    - a layer check refusing a symlink that escapes the layer;
    - a withdrawn build refused at resolve;
    - consent being required again after a revision bump;
  - every migration pattern is tested, including unknown values kept;
  - the launch arguments for a Forge sandbox contain `--version 1.21.4-forge-54.1.0`.

### A5: pickers, preview, consent, progress (tmc-app webview)

- **Owns:**
  - `src/components/catalog/{version-picker,loader-picker,loader-version-picker,install-plan-panel,install-consent-dialog,step-label}.tsx`;
  - the `catalog.*` i18n keys.
- **Touches:**
  - `src/routes/sandboxes.tsx`: the two text fields are replaced by the pickers, and the plan disclosure is added;
  - `src/routes/installs.tsx`: the create flow;
  - `src/components/select.tsx`: an optional search box and a custom row, as additive props;
  - `src/components/{pack-panel,sandbox-share}.tsx`: show `loaderVersion`;
  - the deploy progress view, which listens to `deploy://plan-step`;
  - Settings → Storage, for "Clear unused layers";
  - Settings → Advanced, for "Install manifest from file".
- **Depends on:** A2 and A4.
- **Acceptance:**
  - Playwright against the mocked IPC covers:
    - picking 1.21.4, then Forge, then a version, then the preview listing every step with the full argv;
    - the consent dialog showing the exec statement, the EULA checkbox (server) and the trust line;
    - Install, then progress, then done;
    - a failed step being marked, with its log openable;
    - "Custom…" refusing `../x` and accepting `1.21.4-custom`;
    - the offline banner on a stale list;
    - a game with no catalog showing the free-text fields;
  - screenshots in light and dark;
  - `npm run typecheck` and `npm test` pass.
- **Step labels:** `step-label.tsx` is the one table of human labels per step action. W3's manifest panel needs the same text. It reads a JSON copy, `schemas/step-labels.en.json`, owned by A1 and synced by `catalog:schema:sync`, so neither side hand-writes the other's labels.

### A6: end to end, docs, changelog, readme (tmc-app, plus the website wire doc review)

- **Owns:** `e2e/catalog/*`.
- **Touches:**
  - the three design docs, which become "as built";
  - [sandboxes.md](sandboxes.md), for the fields, the closed `.mrpack` gap, layers and the `JsonPatch` ledger entry;
  - [plugins.md](plugins.md), for the steps an install manifest adds and the fact that plugins refuse them;
  - [gaps.md](gaps.md), to move the open items below into it;
  - `CHANGELOG.md`;
  - the `README.md` feature section ("Game versions and loaders").
- **Depends on:** everything else.
- **Acceptance:** the end-to-end plan below runs green on a release build.

## Order and parallelism

```
A1 ──┬─────────────> A3 ──> A4 ──┐
     │                           ├──> A5 ──> A6
W1 ──┼──> W2 ──> W3              │
     │     └──> W6               │
     └──> W5 ──────> A2 ─────────┘
```

| Wave | Units in parallel |
| --- | --- |
| 1 | A1, W1 |
| 2 | W2, W5, A3 |
| 3 | W3, W6, A2, A4 (A4 after A3; A2 after W5's fixtures exist) |
| 4 | A5 |
| 5 | A6 |

At most four builders run at once (wave 3), which fits the fleet's five slots.

## End-to-end plan

These run against the dev site with the importers applied for Minecraft, on Linux. The Windows steps run on a Windows box.

1. Run `npm run catalog:import -- --app minecraft --dir ../tmc-app/manifests/minecraft --publish`. Running it a second time changes nothing.
2. In the AdminCP, Sync from upstream gives a preview and then Apply. The Versions subtab lists 1.21.4 at the top of the stable list.
3. In the app, a new Minecraft sandbox shows 30 versions. Picking 1.21.4 and Fabric shows the newest Fabric loader as preselected, and the preview lists two layers.
4. Deploy shows no consent, because Fabric has no exec. The layer appears under `layers/minecraft/<hash>`, and `versions/fabric-loader-…` appears in the instance. `launcher_profiles.json` gets a `tmc-<id>` profile.
5. Launch starts the official launcher with `--version fabric-loader-…-1.21.4`, and the game reaches the main menu (manual or screenshot).
6. Switching to Forge 54.1.0 brings up consent showing the installer argv and the Java 21 runtime download. Accepting starts the runtime download, then the installer runs with a streamed log, then `versions/1.21.4-forge-54.1.0` appears. The previous Fabric layer files are undeployed by the ledger.
7. A second sandbox on Forge 54.1.0 reuses the layer, with no installer run.
8. Undeploy leaves the instance folder identical to before step 4, apart from user-made files.
9. Withdrawing Forge 54.1.0 in the AdminCP makes the next deploy of the sandbox fail with the withdrawal note, while launching still works.
10. Offline (the network blocked), a new sandbox's picker shows the cached list with the offline banner. With the cache cleared it shows the compiled snapshot, and deploying a not-yet-built layer says it needs a connection.
11. Running a server-side sandbox with Forge requires the EULA checkbox, after which `eula.txt` and `run.sh` exist.
12. Exporting the catalog with `--for-repo` and then running `manifests:check` in tmc-app is clean.

## Gap list

Each item is either closed by a unit, which is named, or **open**, with the reason. A6 moves the open items into [gaps.md](gaps.md).

| Gap | State |
| --- | --- |
| Database migrations (website) | W1, one migration; (app) A2 N+1 and A4 N+2 |
| Backfill of free text in existing sandboxes | A4, normalisation at migration and on sync pull |
| Backfill of `AppInstall.loaderVersion` on the website | not needed: null until a client syncs |
| i18n | W3, all nine site locales; A5, the app's locales |
| Admin permission | admin only, plus MFA for publish, withdraw, import apply and snapshot apply (W2) |
| Rate limits | admin writes 60/min, compute 10/min (W2); public `appCatalog` 120/min, configurable, 0 = off (W5) |
| Moderation | not applicable to admin-only writes. **Open:** community-submitted loaders (approval queue, Phase 2) |
| Virus scanning and size limits | W2: the existing scan job; `catalogArtifactMaxBytes` and `catalogLinkMaxBytes`, 0 = unlimited; INFECTED and FAILED block publish |
| Mirror publishing | W6 mirrors the Fabric and Quilt JSON as uploads. **Open:** mirroring the Forge installer and the Mojang jars. That is not allowed (the vendors' terms), so they stay links |
| Pack publishing | **Open:** the GameSpice pack format carries no loader fields. Needs a pack schema change in its own repository |
| Cache invalidation | ETag plus `max-age`; withdrawal reaches clients within `catalogCacheSeconds` plus the app TTL. Admin "Purge CDN" is not needed, because API responses are not CDN-cached and artifacts are immutable |
| Pagination | cursor on sortKey and id (W5) |
| Time-based channel ordering | sortKey is the release time in epoch ms, admin-overridable (W1, W6) |
| Semver vs Mojang ordering | never parse; sortKey only (W1 tests) |
| Deprecation | DEPRECATED is listed with a warning; WITHDRAWN is refused at resolve, while existing layers keep launching |
| Deleting a version that sandboxes use | hard delete only if never published; `Restrict` on builds; sandboxes keep names (W2) |
| Offline first run | compiled snapshot plus compiled manifests; installs need a connection (A2, A4) |
| Telemetry | none: catalog requests carry nothing about the sandbox |
| Docs, CHANGELOG, README | A6 |
| Exec signing key | **Open:** the owner creates the real offline Ed25519 key and replaces the test key. Release CI refuses `tmc-exec-test` |
| Installer confinement | **Open:** `bwrap` on Linux and AppContainer on Windows (Phase 2). The design discloses that installers are not sandboxed |
| Launcher-less launch (no official launcher) | **Open (Phase 2):** needs Microsoft account sign-in for Minecraft, with an Azure app id that Mojang approves, plus libraries, natives, assets and argument rules from the version JSON. `downloadList` and `mergeJson` are the building blocks |
| Forge before 1.13, NeoForge for 1.20.1 | **Open:** other installer layouts and coordinates; the importers skip them and log it |
| Quilt server install | **Open:** placeholder; needs an exec template |
| NeoForge installer flags | **Open:** placeholder until verified with `--help` on three version lines |
| Detected system Java | **Open (Phase 2):** `auto` uses the managed runtime |
| Games other than Minecraft | the schema and the Rust are generic. **Open:** no second game's manifests are seeded; the first one should be a game with a plain-archive loader (a BepInEx-style loader), to prove `linkHashPolicy: fetch` |
| Per-feature admin permission | **Open:** the site has none; admin only until it does |
| Layer garbage collection | manual ("Clear unused layers"), never automatic (A5) |
