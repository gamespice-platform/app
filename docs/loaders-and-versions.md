Game versions, loaders and loader versions: what a sandbox runs, where the lists come from, and how picking one makes a deploy install it. This is the design and wire contract for a cross-repository feature (`tmc-app` and `website-city`). **Status: designed, not built.** Every part below is to be built by the units in [loaders-work-split.md](loaders-work-split.md); once a unit lands, the section it covers is rewritten to describe what was built.

Three documents make up the design:

| File | Subject |
| --- | --- |
| this file | the concepts, the data model in all four type systems, the website (AdminCP, admin procedures, the public API, uploads), the app (fields, pickers, cache, deploy, launch, migration) and the verbatim wire examples |
| [loader-manifests.md](loader-manifests.md) | the install manifest: the JSON Schema, the new steps and how they map onto the jail, the security model, trust tiers, the repository layout, import and export, and the seed manifests |
| [loaders-work-split.md](loaders-work-split.md) | the build units, file ownership, ordering, acceptance tests, the end-to-end plan and the gap list |

## Concepts

The model has five catalog records and one plan. None of it is Minecraft-specific: a game's own words (release, snapshot, Forge) are data, not code.

- A **game version** is one version of a game (`1.21.4`, `25w14a`, `1.0.3`). It has a channel, an explicit sort key, and an optional install manifest. The install manifest covers what the plain game needs (for Minecraft, the launcher profile).
- A **loader** is a way of running mods for a game (`fabric`, `forge`). It is identified by a key that is unique within the game. No loader means vanilla: a sandbox with `loader = null` runs the game unmodified.
- A **loader version** is one release of a loader (`0.16.9`, `47.3.0`). Some loaders version independently of the game (Fabric, Quilt). Others publish one version per game version (Forge, NeoForge).
- A **loader build** is a pair of one loader version and one game version that we can install. It is the concrete installable thing. It holds the bindings the loader's manifest needs: artifact hashes and variables such as `launchVersionId`. For Forge, each loader version has exactly one build. For Fabric, the importer creates builds for the pairs inside the configured window (see "Ordering and the last-N window").
- An **artifact** is a file a manifest needs: an installer, a profile JSON, a server jar. It is either an upload (stored in storez, served by the GameSpice CDN) or a link (any https URL). Either way it carries a sha256 and a size, and it is per platform.
- An **install manifest** is a versioned JSON document of steps. There is a template per game (`minecraft/version`) and one per loader (`minecraft/loader/forge`). It is kept in this repository under `manifests/`, imported into the AdminCP, and bound to a concrete game version or build when a plan is resolved.
- A **runtime** is a shared, hash-pinned program that installers need (a Java runtime per major version and platform). It is listed in the AdminCP like everything else.
- An **install plan** is a resolved list of steps for one sandbox. It is resolved from the game version's manifest plus the build's manifest, plus their bindings and artifacts, for one platform and one side. The user sees it before anything runs, and the app executes it.

Templates plus bindings, not one manifest per build, because the step logic is identical for every Forge build. Only the URL, the hash and the version id differ. One reviewed template keeps the repository small and the review honest: a person reads the Forge steps once.

## Data model

### Prisma (website-city)

One model per file under `prisma/models/`, per that repository's rule. All ids are `Int @id @default(autoincrement())`. Every model goes into `NOT_ADDRESSABLE` in `src/types/content/addressable.ts`: these are catalog rows reached through their app, not content with pages.

```prisma
// prisma/models/game-version.prisma
enum CatalogChannel { STABLE PREVIEW EXPERIMENTAL LEGACY }
enum CatalogStatus  { DRAFT PUBLISHED DEPRECATED WITHDRAWN }

model GameVersion {
  id            Int            @id @default(autoincrement())
  appId         Int
  app           App            @relation(fields: [appId], references: [id], onDelete: Cascade, onUpdate: Cascade)
  /// The game's own id for the version, verbatim ("1.21.4", "25w14a").
  versionId     String         @db.Citext
  /// Display text; null shows versionId.
  label         String?
  channel       CatalogChannel @default(STABLE)
  /// The game's own word for the channel ("Snapshot"); null shows the generic one.
  channelLabel  String?
  /// Ordering. Newest first = sortKey DESC. Importers set releasedAt epoch ms; admins may override.
  sortKey       BigInt
  sortKeyManual Boolean        @default(false)
  releasedAt    DateTime?
  status        CatalogStatus  @default(DRAFT)
  deprecatedAt  DateTime?
  deprecationNote String?
  /// Variables bound into manifests ({"javaMajor":"21"}). String values only.
  vars          Json           @default("{}")
  /// Where the importer found it; null for admin-entered rows.
  sourceUrl     String?
  createdById   String?
  createdAt     DateTime       @default(now())
  updatedAt     DateTime       @updatedAt
  builds        GameLoaderBuild[]
  artifacts     InstallArtifact[]

  @@unique([appId, versionId])
  @@index([appId, status, sortKey])
}

// prisma/models/game-loader.prisma
enum LoaderSides { CLIENT SERVER BOTH }

model GameLoader {
  id          Int           @id @default(autoincrement())
  appId       Int
  app         App           @relation(fields: [appId], references: [id], onDelete: Cascade, onUpdate: Cascade)
  /// Lowercase, [a-z0-9-]{1,32}. What the app stores in sandbox.loader.
  key         String        @db.Citext
  name        String
  description String?
  homepage    String?
  iconUploadId Int?
  sides       LoaderSides   @default(BOTH)
  /// Shown on the consent screen ("Some servers refuse modified clients").
  onlineNote  String?
  /// Explicit display order in the picker; ties break on name.
  position    Int           @default(0)
  /// The InstallManifest.manifestKey of the template ("minecraft/loader/forge").
  manifestKey String?
  status      CatalogStatus @default(DRAFT)
  deprecatedAt DateTime?
  createdAt   DateTime      @default(now())
  updatedAt   DateTime      @updatedAt
  versions    GameLoaderVersion[]

  @@unique([appId, key])
}

// prisma/models/game-loader-version.prisma
model GameLoaderVersion {
  id          Int            @id @default(autoincrement())
  loaderId    Int
  loader      GameLoader     @relation(fields: [loaderId], references: [id], onDelete: Cascade, onUpdate: Cascade)
  version     String         @db.Citext
  channel     CatalogChannel @default(STABLE)
  sortKey     BigInt
  sortKeyManual Boolean      @default(false)
  releasedAt  DateTime?
  status      CatalogStatus  @default(DRAFT)
  deprecatedAt DateTime?
  deprecationNote String?
  /// True for versions admins added by hand (a patched fork, a pinned old build).
  custom      Boolean        @default(false)
  sourceUrl   String?
  createdAt   DateTime       @default(now())
  updatedAt   DateTime       @updatedAt
  builds      GameLoaderBuild[]

  @@unique([loaderId, version])
  @@index([loaderId, status, sortKey])
}

// prisma/models/game-loader-build.prisma
model GameLoaderBuild {
  id              Int               @id @default(autoincrement())
  loaderVersionId Int
  loaderVersion   GameLoaderVersion @relation(fields: [loaderVersionId], references: [id], onDelete: Cascade, onUpdate: Cascade)
  gameVersionId   Int
  gameVersion     GameVersion       @relation(fields: [gameVersionId], references: [id], onDelete: Restrict, onUpdate: Cascade)
  /// The loader's own promotion for this game version.
  recommended     Boolean           @default(false)
  /// {"launchVersionId":"1.21.4-forge-54.0.12"}; merged over the game version's vars.
  vars            Json              @default("{}")
  status          CatalogStatus     @default(DRAFT)
  createdAt       DateTime          @default(now())
  updatedAt       DateTime          @updatedAt
  artifacts       InstallArtifact[]

  @@unique([loaderVersionId, gameVersionId])
  @@index([gameVersionId, status])
}

// prisma/models/install-manifest.prisma
model InstallManifest {
  id          Int           @id @default(autoincrement())
  appId       Int
  app         App           @relation(fields: [appId], references: [id], onDelete: Cascade, onUpdate: Cascade)
  /// The manifest's own "id" ("minecraft/loader/forge"). One row per revision.
  manifestKey String        @db.Citext
  revision    Int
  schemaVersion Int
  /// Canonical JSON (see loader-manifests.md "Canonical form").
  body        Json
  /// sha256 hex of the canonical bytes; the idempotency key for import.
  sha256      String
  /// True when any step is runInstaller.
  exec        Boolean       @default(false)
  /// Detached Ed25519 signature over sha256, base64; required when exec.
  signature   String?
  signatureKeyId String?
  status      CatalogStatus @default(DRAFT)
  publishedAt DateTime?
  publishedById String?
  createdById String?
  createdAt   DateTime      @default(now())

  @@unique([manifestKey, revision])
  @@unique([manifestKey, sha256])
  @@index([appId, manifestKey, status])
}

// prisma/models/install-artifact.prisma
enum ArtifactSource   { UPLOAD LINK }
enum ArtifactScan     { PENDING CLEAN INFECTED SKIPPED FAILED }

model InstallArtifact {
  id            Int             @id @default(autoincrement())
  /// Exactly one owner is set; enforced in src/lib/game-catalog/artifact.ts and verify:run game-catalog.
  gameVersionId Int?
  gameVersion   GameVersion?    @relation(fields: [gameVersionId], references: [id], onDelete: Cascade, onUpdate: Cascade)
  buildId       Int?
  build         GameLoaderBuild? @relation(fields: [buildId], references: [id], onDelete: Cascade, onUpdate: Cascade)
  runtimeId     Int?
  runtime       InstallRuntime? @relation(fields: [runtimeId], references: [id], onDelete: Cascade, onUpdate: Cascade)
  /// The manifest's artifact key ("installer", "profileJson").
  key           String
  /// "any" or one of the platform ids in loader-manifests.md.
  platform      String          @default("any")
  source        ArtifactSource
  uploadId      Int?            @unique
  upload        FileUpload?     @relation(fields: [uploadId], references: [id], onDelete: Restrict, onUpdate: Cascade)
  /// GameStore key under game/catalog/ for UPLOAD; null for LINK.
  storeKey      String?
  /// Required for LINK; for UPLOAD it is computed (GameFileUrl) and never stored.
  url           String?
  /// Further https URLs with the same bytes. At most 8 (the step vocabulary's own bound).
  mirrors       String[]        @default([])
  sha256        String
  size          BigInt
  /// Provenance for mirrored uploads (the meta URL a profile JSON came from).
  sourceUrl     String?
  /// false = must stay a LINK (a game's own binaries we may not redistribute).
  redistributable Boolean       @default(true)
  scan          ArtifactScan    @default(PENDING)
  createdById   String?
  createdAt     DateTime        @default(now())

  @@unique([gameVersionId, key, platform])
  @@unique([buildId, key, platform])
  @@unique([runtimeId, key, platform])
}

// prisma/models/install-runtime.prisma
model InstallRuntime {
  id         Int           @id @default(autoincrement())
  /// "java"; any [a-z0-9-]{1,32}. Not a Rust enum: a game that needs .NET adds a row, not code.
  kind       String
  /// The major version a manifest asks for ("21").
  major      String
  /// Display only ("Temurin 21.0.5+11").
  vendorLabel String
  /// Path of the executable inside the extracted archive, per platform artifact
  /// ({"linux-x64":"bin/java","windows-x64":"bin/javaw.exe"}).
  entry      Json
  /// How the archive unpacks: the Extract step's strip and stripSingleRoot.
  stripSingleRoot Boolean    @default(true)
  status     CatalogStatus @default(DRAFT)
  createdAt  DateTime      @default(now())
  updatedAt  DateTime      @updatedAt
  artifacts  InstallArtifact[]

  @@unique([kind, major])
}

// prisma/models/game-catalog-audit.prisma
model GameCatalogAudit {
  id         Int      @id @default(autoincrement())
  actorId    String?
  /// "version.create", "manifest.publish", "import.apply", "artifact.upload", ...
  action     String
  appId      Int?
  /// "GameVersion:12", "InstallManifest:4".
  target     String
  /// {"before":...,"after":...} for edits; {"sha256":...} for imports.
  detail     Json     @default("{}")
  createdAt  DateTime @default(now())

  @@index([appId, createdAt])
  @@index([target])
}
```

`AppInstall` gains `loaderVersion String?`, next to its existing `gameVersion` and `loader` (website `prisma/models/app-install.prisma`). It is free text on the wire, for the same reason the other two are: a sandbox may name a custom version.

Why `Restrict` on `GameLoaderBuild.gameVersion`: deleting a game version that builds point at must be a decision. The admin is shown the builds and withdraws them first, rather than having them vanish in a cascade. Sandboxes never reference catalog rows by id (they store names), so no delete can break a sandbox row. What happens to a sandbox whose version is withdrawn is in "Deprecation and withdrawal".

### zod and TypeScript (website-city)

- Import-free types go in `src/types/game-catalog/`:
  - `catalog.ts`: `CatalogChannelT`, `CatalogStatusT`, `PlatformT`, `LoaderSidesT`.
  - `wire.ts`: the zod schemas of every public response below, exported as `GameVersionWireSchema`, `GameLoaderWireSchema`, `LoaderVersionWireSchema`, `InstallPlanWireSchema`, `RuntimeWireSchema`, plus their `z.infer` types.
  - `manifest.ts`: `LoaderManifestSchema`, the zod mirror of the JSON Schema.
- The contract additions go in `src/types/app-api/contract.ts`. `npm run contract:sync` then copies them into `tmc-app/src/lib/api/contract.ts`, so the app never hand-writes them.
- The JSON Schema itself is copied, not mirrored. `npm run catalog:schema:sync` copies `tmc-app/schemas/loader-manifest.v1.schema.json` to `website-city/src/types/game-catalog/loader-manifest.v1.schema.json`. A node test fails when the zod mirror accepts something the JSON Schema rejects, or the other way round, over the fixture set `src/types/game-catalog/fixtures/*.json`.

### Rust (tmc-app, tmc-core)

```rust
// src-tauri/core/src/catalog/types.rs — serde camelCase, deny_unknown_fields on input
pub enum Channel { Stable, Preview, Experimental, Legacy }
pub struct GameVersionEntry { pub version_id: String, pub label: Option<String>, pub channel: Channel,
    pub channel_label: Option<String>, pub sort_key: i64, pub released_at: Option<String>,
    pub deprecated: Option<Deprecation>, pub has_manifest: bool }
pub struct LoaderEntry { pub key: String, pub name: String, pub description: Option<String>,
    pub sides: Sides, pub online_note: Option<String>, pub position: i32, pub custom_versions: bool }
pub struct LoaderVersionEntry { pub version: String, pub channel: Channel, pub sort_key: i64,
    pub recommended: bool, pub custom: bool, pub deprecated: Option<Deprecation> }
pub struct Deprecation { pub at: String, pub note: Option<String>, pub withdrawn: bool }

// src-tauri/core/src/layers/plan.rs
pub struct InstallPlan { pub plan_hash: String, pub app_slug: String, pub platform: Platform,
    pub side: Side, pub layers: Vec<PlannedLayer>, pub runtimes: Vec<RuntimeNeed>,
    pub consents: Vec<Consent>, pub launch_vars: BTreeMap<String, String>,
    pub deploy_patches: Vec<DeployPatch>, pub trust: Trust }
pub struct PlannedLayer { pub manifest_key: String, pub revision: u32, pub manifest_sha256: String,
    pub layer_hash: String, pub steps: Vec<Step>, pub artifacts: BTreeMap<String, ResolvedArtifact>,
    pub exec: bool, pub signature: Option<String> }
pub struct ResolvedArtifact { pub url: String, pub mirrors: Vec<String>, pub sha256: String, pub size: u64 }
pub enum Trust { Compiled, Official, Local }
```

`Step` is the existing `plugins::manifest::Step`, extended (see [loader-manifests.md](loader-manifests.md)); there is no second step enum. The webview mirror goes in `src/lib/ipc/schemas.ts` as `InstallPlanSchema`, `CatalogVersionsSchema` and the rest, parsed on every IPC return as usual.

### Relations, at a glance

```
App 1─* GameVersion 1─* GameLoaderBuild *─1 GameLoaderVersion *─1 GameLoader *─1 App
GameVersion 1─* InstallArtifact *─1 GameLoaderBuild        InstallRuntime 1─* InstallArtifact
InstallArtifact ─0..1 FileUpload                           App 1─* InstallManifest (by manifestKey, revision)
GameLoader.manifestKey ─> InstallManifest (latest PUBLISHED revision)
the game's version manifest = InstallManifest "<appSlug>/version" (latest PUBLISHED)
```

A game version or loader without a published manifest is listable but **unmanaged**: picking it records the name and installs nothing, exactly as the free-text field does today.

## Ordering and the last-N window

- Lists are ordered by `sortKey DESC`, then `id DESC`. Strings are never compared.
- Mojang ids defeat both lexical and semver ordering: `1.21.10` is newer than `1.21.9`, and `25w14a`, `1.21.5-pre1`, `1.21.5-rc1` and `26.1-snapshot-1` all sit between releases in time, not by any parse.
- Importers therefore set `sortKey` to the upstream release time in epoch milliseconds.
  - For Forge and NeoForge, whose maven metadata has no times, the importer uses the order of the `<versions>` list, scaled into the build's game-version time range. That keeps a loader version's builds after their game version.
  - An admin may set `sortKey` by hand. `sortKeyManual = true` then makes later imports leave it alone.
- `tmc_core::version` stays the comparison for mod versions. Catalog ordering never calls it.
- "The most recent N" is a request parameter: `limit`.
  - The default is the site setting `catalogListDefault` (30), and the cap is `catalogListMax` (200). Following the no-hard-caps rule, `0` means unlimited for both.
  - The app asks for its own setting `versionListCount` (30, where 0 means everything).
- Channels filter the list:
  - The default is `STABLE` only.
  - The picker has a "Show previews and snapshots" switch, which adds `PREVIEW` and `EXPERIMENTAL`.
  - `LEGACY` (old alpha and beta) appears only under "Show all".
  - A version already chosen on a sandbox is always shown, whatever the filter.
- Loader versions are filtered to the builds that exist for the chosen game version.
  - A loader without builds for that version shows as disabled, with the hint "No Fabric build for 25w14a".
  - If the loader allows custom versions, the user can still type one.

## Deprecation and withdrawal

- **DEPRECATED** means still listed, still installable, and shown with a warning and the note. New sandboxes do not get it as a default.
- **WITHDRAWN** means not listed and not installable.
  - A sandbox that already names a withdrawn version keeps the name.
  - Its next deploy refuses to resolve a plan, with the error "This Forge version was withdrawn: <note>. Pick another version."
  - Layers already built for it stay on disk, and launching keeps working. Withdrawal stops new installs; it does not break running games.
- **Hard delete** is allowed only for rows that were never PUBLISHED. Once published, rows are withdrawn instead, because sandboxes on other machines name them. This is enforced in the admin procedure, not by the UI.

## Website

### AdminCP: the Mods tab

- A new top-level entry goes in `TABS` (`src/app/_components/lib/admin/wrapper.tsx`): `{ key: 'mods', labelKey: 'tabs.mods', url: '/admin/mods/loaders', icon: <puzzle icon> }`.
- The secondary tabs follow the servers pattern in the same file: a `modsSec = sec.startsWith('mods-')` branch renders a `TabMenu` with three items:
  - Loaders, at `/admin/mods/loaders` (`sec="mods-loaders"`);
  - Versions, at `/admin/mods/versions` (`sec="mods-versions"`);
  - Runtimes, at `/admin/mods/runtimes` (`sec="mods-runtimes"`).
- `PanelTabs` gets `active={modsSec ? 'mods' : ...}`.
- Pages: `src/app/[locale]/admin/mods/{loaders,versions,runtimes}/page.tsx`, plus `src/app/[locale]/admin/mods/page.tsx`, which redirects to loaders. Each page follows the admin page pattern: `auth()` and `IsAdmin`, otherwise 404; `PanelMeta(t('meta.adminMods<X>.title'))`; then `Wrapper`, `HydrateClient` and `AdminWrapper`.
- Components go in `src/app/_components/lib/admin/mods/`:
  - `game-select.tsx`: every screen is scoped to one app (game), chosen from apps of type GAME, and the choice is kept in the URL (`?app=<id>`).
  - `versions/table.tsx` and `versions/form.tsx`. The table shows `versionId`, label, channel, status, sortKey, releasedAt and build count. Its filters are status, channel and text. The form uses `FillDefaults`.
  - `loaders/table.tsx` and `loaders/form.tsx`, which include the manifest picker and "custom versions allowed".
  - `loaders/versions-table.tsx`: one loader's versions, with a builds count. A row opens a drawer with its builds, each build's vars and artifacts, and recommended flags.
  - `artifacts/editor.tsx`: an artifact row is either an upload (drop zone) or a link (URL, sha256, size, mirrors).
    - For links, the "Compute" button fetches the URL server-side, under the size limit, and fills in the sha256 and size.
    - The scan state is shown as a badge.
  - `manifests/panel.tsx` covers manifests:
    - It lists each manifestKey with its revisions and status.
    - A read-only rendered view of the steps uses the same renderer as the app's preview, from the shared step-label table (see "Step labels" below).
    - A JSON editor validates against the schema live.
    - Its buttons are Publish (admin plus MFA), Import JSON (file), Export JSON, and Diff between revisions.
  - `sync/panel.tsx`: "Sync from upstream" for apps that have an importer registered (Minecraft at launch). It shows a dry-run diff first, then Apply.
  - `runtimes/table.tsx` and `runtimes/form.tsx`.
  - `audit/feed.tsx`: the `GameCatalogAudit` rows for the selected app, newest first.
- Copy goes into `locales/en/admin.json` under `mods.*`, `tabs.mods`, `tabs.modsLoaders`, `tabs.modsVersions`, `tabs.modsRuntimes` and `meta.adminMods*`.
  - Machine-translated copies go to all eight other locales, then `i18n:bundle:gen` and `i18n:ns:gen`.
  - The route namespaces go in `src/types/i18n/route-namespaces.ts`.

### Admin tRPC procedures

- The router is `src/server/api/routers/game-catalog.ts`, mounted in `admin.ts` as `admin.gameCatalog`. Business rules live in `src/lib/game-catalog/*`, and the router only parses and calls them.
- Every procedure is `adminProcedure`.
- Every write is also `rateLimit` (60 a minute per admin) and writes a `GameCatalogAudit` row in the same transaction.
- Procedures marked MFA use `requireMfa`.

| Procedure | Kind | Notes |
| --- | --- | --- |
| `versions.list({appId, status?, channel?, q?, cursor?, limit})` | query | admin sees DRAFT and WITHDRAWN too |
| `versions.upsert({appId, versionId, ...})` | mutation | look up by (appId, versionId) then create or update; never `ON CONFLICT` |
| `versions.setStatus({id, status, note?})` | mutation, MFA for PUBLISHED and WITHDRAWN | refuses hard delete after publish |
| `versions.delete({id})` | mutation | only never-published rows |
| `loaders.list / upsert / setStatus / delete / reorder({appId, keys[]})` | as above | `reorder` rewrites `position` |
| `loaderVersions.list / upsert / setStatus / delete` | as above | `custom: true` on admin-created rows |
| `builds.list({loaderVersionId})`, `builds.upsert({loaderVersionId, gameVersionId, vars, recommended})`, `builds.setStatus` | | a build can only be PUBLISHED when its loader version, its game version and the loader's manifest are |
| `artifacts.presign({owner, key, platform, size, sha256, contentType})` | mutation | returns a storez presigned PUT (see "Upload flow") |
| `artifacts.commit({uploadId})` | mutation | verifies, then creates the `InstallArtifact` |
| `artifacts.link({owner, key, platform, url, sha256, size, mirrors, redistributable})` | mutation | |
| `artifacts.compute({url})` | mutation, rate limit 10 a minute | server-side fetch, returns `{sha256, size}`; honours `catalogLinkMaxBytes` |
| `artifacts.delete({id})` | mutation | refused while a PUBLISHED build or version references the key |
| `manifests.list({appId})`, `manifests.get({manifestKey, revision?})` | query | |
| `manifests.import({appId, json, signature?})` | mutation | idempotent; see [loader-manifests.md](loader-manifests.md) "Import and export" |
| `manifests.publish({manifestKey, revision})` | mutation, MFA | refuses `x-tmc-placeholder`, refuses exec without a valid signature |
| `manifests.export({manifestKey, revision?})` | query | canonical JSON plus signature |
| `catalog.exportSnapshot({appId})` | query | `catalog.snapshot.json` for the app repo |
| `catalog.importSnapshot({appId, json, dryRun})` | mutation, MFA when not dryRun | |
| `sync.preview({appId})`, `sync.apply({appId, token})` | mutation | the upstream importers; `token` ties apply to the previewed diff |
| `runtimes.list / upsert / setStatus` | as above | |
| `audit.list({appId?, target?, cursor})` | query | |

Permission is admin only. Moderators get no access: an install manifest is code-adjacent, and the site's moderator role reviews content, not executables. A finer `catalog.manage` permission is not invented, because the site has no per-feature admin permission system to hang it on.

### Upload flow

1. The admin drops a file. The browser computes its sha256 (Web Crypto, streamed) and calls `artifacts.presign`.
2. The server checks the size against `catalogArtifactMaxBytes` (default 2 GiB; 0 = unlimited) and the upload policy (`src/lib/file/upload-policy.ts`).
   - It creates a `FileUpload` row, tied to the artifact by its own `uploadId`, per the "every upload has a FileUpload row" rule.
   - It picks the key `game/catalog/<appId>/<sha256>/<safe file name>`. The key is content-addressed, so the same bytes uploaded twice reuse one object.
   - The key passes `AssertGamePrefix` and `GameKeyProblem`.
   - The server then returns `GameObjectPutUrl({key, sha256, size})`, so the storez presign covers the body hash.
3. The browser PUTs the bytes and calls `artifacts.commit`.
4. The server runs `HeadGameObject` and checks size and hash.
   - It writes the `InstallArtifact` (`source UPLOAD`, `storeKey`).
   - It queues a virus scan with the existing job machinery (`src/lib/file/jobs.ts`, `UploadJobType`), following the file-upload checklist for the new job owner.
   - `scan` starts `PENDING`. Builds and versions whose artifacts are `INFECTED` or `FAILED` cannot be published. `SKIPPED` (scanning disabled or over the scan size limit) may be published, and the AdminCP shows the reason.
5. Public URLs are `GameFileUrl(storeKey)`: the `NEXT_PUBLIC_GAME_ORIGIN` CDN, with `Cache-Control: public, max-age=31536000, immutable`, as for every game object.

Link rules:
- Links must be `https://`.
- They are re-validated by `artifacts.link` against `catalogLinkHostAllow`. That is a site setting: a list of hosts, where empty means any host and `*.` wildcards are allowed.
- Links are never fetched from the browser.
- Uploads of `redistributable: false` artifacts are refused (a game's own client and server jars are links to the vendor).

URLs in public text use prod ids. Wire responses carry whatever the serving site's own ids are, which is correct for that site. The rule bites in two places. Repository files, meaning seed manifests and `catalog.snapshot.json`, may only contain prod CDN URLs or vendor links. And the exporter refuses to write a URL whose origin is not the prod game origin when `--for-repo` is set.

### Public app API

These routes are under the existing `/api/app/v1` surface, so there is no new `API_SURFACE_PREFIXES` entry and no new nginx block. Each handler uses `handleAppApi` with auth `'optional'`. The catalog is public, and a signed-in user is rate-limited on the account rather than the IP.

| Route | Purpose |
| --- | --- |
| `GET /api/app/v1/apps/:id/catalog` | one call for the picker: loaders plus the first page of game versions |
| `GET /api/app/v1/apps/:id/versions?limit&cursor&channels` | game versions, newest first |
| `GET /api/app/v1/apps/:id/loaders/:key/versions?gameVersion&limit&cursor&channels` | loader versions with a build for `gameVersion`; without `gameVersion`, all |
| `GET /api/app/v1/apps/:id/install-plan?gameVersion&loader&loaderVersion&platform&side` | the resolved, unexecuted plan |
| `GET /api/app/v1/runtimes/:kind/:major?platform` | a runtime and its artifacts |

Rules shared by all five routes:
- `:id` is the numeric app id, as in `apps/[id]/settings`.
- **Pagination** uses an opaque cursor: base64url of `sortKey:id`. The response is `{items, nextCursor|null}`. Offsets are never used, because the list grows at the head.
- **ETag**: the body is hashed with `JsonEtag` (`src/lib/integration/etag.ts`).
  - `If-None-Match` is checked with `EtagMatches`, and a match gets a 304 with no body.
  - `Cache-Control` is `public, max-age=<catalogCacheSeconds>, stale-while-revalidate=86400`, with `catalogCacheSeconds` defaulting to 300.
  - `install-plan` responses for a PUBLISHED build never change in place (any change is a new manifest revision or artifact row), but they are still revalidated, because withdrawal must reach clients.
- **Rate limit**: a new `appCatalog` bucket at 120 requests a minute per account or IP. It uses the site setting `catalogRateLimitPerMinute` (0 = unlimited).
- **Errors** use the `/api/app/v1` error shape (`apiError`) with these codes:
  - `catalog_not_found`: unknown app, version, loader or build;
  - `catalog_withdrawn`, with `note`;
  - `catalog_unmanaged`: the names are valid but there is no published manifest;
  - `catalog_platform`: no artifact for this platform.
- Only PUBLISHED and DEPRECATED rows are visible. Hidden apps (`hidden` or `modHidden`) return `catalog_not_found` to everyone but their owner.
- The wire doc is `website-city/docs/api/app-catalog.md`. It holds every route, every field, the examples below verbatim, and the error codes.
- The probes go in `scripts/verify-game-catalog.ts`. It is not `verify-api-surface-split.ts`, because there is no new surface.

## Wire examples (verbatim contract)

These are the shared contract between the units. Field names, nesting and value formats are exactly as shown. The tests in both repositories parse these files from `website-city/src/types/game-catalog/fixtures/` and `tmc-app/src/lib/api/fixtures/catalog/` (the app copy is synced by `contract:sync`).

`GET /api/app/v1/apps/412/catalog?limit=3`

```json
{
  "app": { "id": 412, "slug": "minecraft" },
  "loaders": [
    { "key": "fabric", "name": "Fabric", "description": "Lightweight loader", "sides": "BOTH", "onlineNote": null, "position": 0, "customVersions": false, "managed": true },
    { "key": "forge", "name": "Forge", "description": null, "sides": "BOTH", "onlineNote": null, "position": 1, "customVersions": false, "managed": true },
    { "key": "neoforge", "name": "NeoForge", "description": null, "sides": "BOTH", "onlineNote": null, "position": 2, "customVersions": false, "managed": true },
    { "key": "quilt", "name": "Quilt", "description": null, "sides": "BOTH", "onlineNote": null, "position": 3, "customVersions": false, "managed": true }
  ],
  "versions": {
    "items": [
      { "versionId": "1.21.4", "label": null, "channel": "STABLE", "channelLabel": "Release", "sortKey": 1733234400000, "releasedAt": "2024-12-03T10:40:00Z", "deprecated": null, "managed": true },
      { "versionId": "1.21.3", "label": null, "channel": "STABLE", "channelLabel": "Release", "sortKey": 1729591200000, "releasedAt": "2024-10-22T10:00:00Z", "deprecated": null, "managed": true },
      { "versionId": "1.21.2", "label": null, "channel": "STABLE", "channelLabel": "Release", "sortKey": 1729591100000, "releasedAt": "2024-10-22T09:58:20Z", "deprecated": { "at": "2025-01-10T00:00:00Z", "note": "Superseded by 1.21.3 the same day", "withdrawn": false }, "managed": true }
    ],
    "nextCursor": "MTcyOTU5MTEwMDAwMDo4OA"
  },
  "generatedAt": "2026-11-22T09:00:00Z"
}
```

`GET /api/app/v1/apps/412/loaders/forge/versions?gameVersion=1.21.4&limit=2`

```json
{
  "loader": "forge",
  "gameVersion": "1.21.4",
  "items": [
    { "version": "54.1.0", "channel": "STABLE", "sortKey": 1736100000000, "recommended": true, "custom": false, "deprecated": null },
    { "version": "54.0.34", "channel": "STABLE", "sortKey": 1735500000000, "recommended": false, "custom": false, "deprecated": null }
  ],
  "nextCursor": null
}
```

`GET /api/app/v1/apps/412/install-plan?gameVersion=1.21.4&loader=fabric&loaderVersion=0.16.9&platform=linux-x64&side=client`

```json
{
  "app": { "id": 412, "slug": "minecraft" },
  "request": { "gameVersion": "1.21.4", "loader": "fabric", "loaderVersion": "0.16.9", "platform": "linux-x64", "side": "client" },
  "layers": [
    {
      "manifestKey": "minecraft/version",
      "revision": 1,
      "sha256": "6f1c0d6b2a8f4e0c9a7d3b5e1f2a4c6d8e0b1a3c5d7e9f1a2b4c6d8e0f1a3b5c",
      "exec": false,
      "signature": null,
      "manifest": { "schemaVersion": 1, "id": "minecraft/version", "revision": 1, "kind": "version", "app": "minecraft", "title": "Minecraft", "steps": [] },
      "bindings": {
        "vars": { "gameVersion": "1.21.4", "javaMajor": "21", "launchVersionId": "1.21.4" },
        "artifacts": {
          "versionJson": { "url": "https://piston-meta.mojang.com/v1/packages/a3bcba436caa849622fd7e1e5b89489ed6c9ac63/1.21.4.json", "mirrors": [], "sha256": "0f7f2fbb3c7e94b4d5e1a3b2c9d8e7f6a5b4c3d2e1f0a9b8c7d6e5f4a3b2c1d0", "size": 27461 }
        }
      }
    },
    {
      "manifestKey": "minecraft/loader/fabric",
      "revision": 1,
      "sha256": "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90",
      "exec": false,
      "signature": null,
      "manifest": { "schemaVersion": 1, "id": "minecraft/loader/fabric", "revision": 1, "kind": "loader", "app": "minecraft", "loader": "fabric", "title": "Fabric", "steps": [] },
      "bindings": {
        "vars": { "loaderVersion": "0.16.9", "launchVersionId": "fabric-loader-0.16.9-1.21.4" },
        "artifacts": {
          "profileJson": { "url": "https://cdn.gamespice.net/game/catalog/412/9c1d.../fabric-loader-0.16.9-1.21.4.json", "mirrors": [], "sha256": "9c1d4e2f3a4b5c6d7e8f9a0b1c2d3e4f5a6b7c8d9e0f1a2b3c4d5e6f7a8b9c0d", "size": 5120 }
        }
      }
    }
  ],
  "runtimes": [],
  "deprecated": null,
  "planVersion": 1
}
```

The `manifest` objects are elided to `"steps": []` here for length. On the wire they are the full canonical manifest, exactly as in [loader-manifests.md](loader-manifests.md). The CDN host in the example is a placeholder: the real one is the prod `NEXT_PUBLIC_GAME_ORIGIN`.

`GET /api/app/v1/runtimes/java/21?platform=linux-x64`

```json
{
  "kind": "java",
  "major": "21",
  "vendorLabel": "Temurin 21.0.5+11",
  "entry": "bin/java",
  "stripSingleRoot": true,
  "artifact": { "url": "https://github.com/adoptium/temurin21-binaries/releases/download/jdk-21.0.5%2B11/OpenJDK21U-jre_x64_linux_hotspot_21.0.5_11.tar.gz", "mirrors": [], "sha256": "553dda64b3b1c3c16f8afe402377ffebe64fb4a1721a46ed426a91fd18185e62", "size": 52412345 }
}
```

The error shape is as follows. The status is 410 for `catalog_withdrawn`, 404 for `catalog_not_found` and `catalog_unmanaged`, and 422 for `catalog_platform`.

```json
{ "error": { "code": "catalog_withdrawn", "message": "This Forge version was withdrawn.", "note": "Crashes on world load; use 54.1.0" } }
```

The sync shape of `AppInstall` adds one field:

```json
{ "id": "c0a8...", "appId": 412, "name": "Survival 1.21", "gameVersion": "1.21.4", "loader": "fabric", "loaderVersion": "0.16.9" }
```

## App

### Fields

- First-class sandbox fields (Rust `Sandbox`, the library database, the `AppInstall` wire):
  - `game_version: Option<String>`, which already exists;
  - `loader: Option<String>`, which already exists. From now on it holds a loader **key** (lowercase), and `None` means vanilla;
  - `loader_version: Option<String>`, which is new.
- They are first-class because sync, share codes, `.mrpack` export, launch placeholders, the mod filter (mods declare loaders and versions) and the deploy all read them. Options are per-game tuning that only `launch.json` reads.
- `install_state: Option<InstallState>` is new and local only, never synced: `{ plan_hash, layers: [{manifest_key, revision, layer_hash}], launch_vars, deployed_at }`. It records what was last installed. Deploy compares it with the freshly resolved plan to decide what to build, and launch reads `launch_vars` from it, so launching works offline.
- Nothing about versions or loaders goes in `sandbox.options`.
  - The existing Minecraft option `javaPath` (auto/17/21) changes meaning: `auto` now means "the version the plan requires". Its select gains `managed` (always use the TMC-downloaded runtime) and `custom` (a path, validated through the existing path rules).
- `.mrpack` export now writes `dependencies` with the loader and its version, which closes the gap noted in [sandboxes.md](sandboxes.md). `TMC1-` share codes carry the same three names; they already carry nothing machine-specific.

### Pickers

- Components:
  - `src/components/catalog/version-picker.tsx`: a game version.
  - `src/components/catalog/loader-picker.tsx`: the loader, with "None (vanilla)" first.
  - `src/components/catalog/loader-version-picker.tsx`: a loader version, filtered by the chosen game version.
  - The three are built on the existing `select.tsx` (no native `<select>`), extended with a search box and a "Custom…" row.
- "Custom…" turns the control into a `TextField` with inline validation.
- Each row shows:
  - the label;
  - the channel badge (the game's `channelLabel`);
  - "Recommended";
  - "Deprecated" with its note as the hint;
  - "Not installed by TMC" for unmanaged entries.
- Changing the game version keeps the loader. It keeps the loader version only if a build exists for the new game version. Otherwise it picks the recommended build, or the newest one, and says so in a one-line notice under the field.
- These pickers replace the two `TextField`s in `src/routes/sandboxes.tsx` (the "Game version" and "Loader" fields) and the matching fields in the create flow (`src/routes/installs.tsx`).
- A game with no catalog shows the free-text fields as today. Every game without catalog rows keeps working unchanged.

### Lists: remote, cached, compiled in

- The new Rust module `src-tauri/core/src/catalog/` fetches the lists from the public API through the app's own HTTP client. The webview never fetches anything.
- Responses are cached in a new library database table:
  `catalog_cache(key TEXT PRIMARY KEY, etag TEXT, body TEXT, fetched_at INTEGER)`.
  The key is the request path plus its query.
- A read inside the TTL is served from the cache. The TTL is the app setting `catalogTtlMinutes`: default 360, `0` means always revalidate.
- After the TTL, the read revalidates with `If-None-Match`, and a 304 refreshes `fetched_at`.
- When the network fails, the stale cache is served and flagged `stale: true`, and the picker says "Offline: list from <date>".
- With no cache at all, the compiled-in fallback is used:
  - The source is `manifests/<appSlug>/catalog.snapshot.json`, compiled in by `core/build.rs` the same way app rules are.
  - The test `the_compiled_in_catalog_matches_the_repository` checks it.
  - It is flagged `source: "compiled"`.
- Because the seed manifests are compiled in too, an offline first run can still resolve plans for the snapshot's versions from compiled data. The downloads themselves still need the network: Maven, Mojang and the CDN. An offline first run therefore gets a complete picker and a clear "needs a connection to install" at deploy time, never an empty list.
- IPC commands go in `src-tauri/src/commands/catalog.rs`, and their wrappers in `src/lib/ipc/commands.ts`. The webview names only slugs and ids:
  - `catalog_get(appSlug, {channels, limit})`;
  - `catalog_loader_versions(appSlug, loader, gameVersion?)`;
  - `catalog_refresh(appSlug)`;
  - `install_plan_preview(sandboxId)`;
  - `install_plan_consent(sandboxId, planHash, consentIds[])`.
- No telemetry is involved. A catalog request carries the app version header that every request already carries, and nothing about the sandbox.

### Custom versions

- A custom game version or loader version must match `^[A-Za-z0-9][A-Za-z0-9._+\-]{0,63}$`. That rules out path separators, spaces, `..` and placeholders.
- A game may narrow this with `sandbox.json` `catalog.versionPattern`, a regex checked in Rust; Minecraft sets none.
- A custom value is recorded and **unmanaged**:
  - deploy installs no loader for it;
  - launch passes it as `{gameVersion}`;
  - the UI shows "TMC will not install this; install it yourself".
- The exception is a loader that is `customVersions: true` and whose manifest declares that its artifact URLs are fully templated (`urlTemplate`) with `linkHashPolicy: "fetch"`. For such a loader the plan can be resolved for a typed version: the app downloads the URL, records its sha256, and shows "unverified version: hash recorded on first install" in the preview.
  - This is never allowed for an exec manifest. A typed Forge version cannot run an installer whose hash nobody pinned.
  - None of the seed loaders set it. It exists for games whose loaders are plain archives.

### Deploy consumption

Deploy (`library/deploy.rs`, `deploy/engine.rs`) gains one phase before the merge tree is built:

1. **Resolve.**
   - The plan comes from the API, or from the cache, or is compiled in. Its order is the game version's layer first, then the loader build's layer.
   - Resolving checks the signatures and trust ([loader-manifests.md](loader-manifests.md) "Trust").
   - It computes each `layer_hash`: sha256 over the canonical manifest bytes, the bound vars, the resolved artifacts (sha256 only, never URLs, so a mirror change does not rebuild), the platform and the side.
2. **Consent.**
   - If the plan has exec steps, runtime downloads or `consents` that have not been accepted for this `plan_hash`, the deploy stops with `NeedsConsent(plan)`.
   - The UI shows the preview and calls `install_plan_consent`, then deploys again.
   - Consent is stored per sandbox and per `plan_hash`. A new revision or a new artifact asks again.
3. **Runtimes.** Each required runtime is ensured under `<app data>/runtimes/<kind>/<major>/<platform>/<sha256-12>/`. It is downloaded with its pinned hash, extracted with the existing Extract step and its bounds, and marked complete with `.tmc-runtime.json`.
4. **Layers.** Each layer is built at `<app data>/layers/<appSlug>/<layer_hash>/`.
   - A layer whose `.tmc-layer.json` says complete is reused. The marker holds the manifest key, the revision, the `files` list with sizes and sha256, and the build time.
   - Otherwise the executor runs the manifest's `steps` in `<layer_hash>.partial/`, with the jail roots `layer`, `downloads` and `runtime` (read-only).
   - It then runs the post-build checks in [loader-manifests.md](loader-manifests.md) "Layer checks", writes the marker and renames the directory into place.
   - A failure deletes `.partial` and stops the deploy **before the game folder is touched**.
5. **Merge.**
   - Each layer's `files/` subtree becomes a source in the merge tree, as a pseudo-mod `layer:<manifestKey>` at a priority below every real mod. A mod that ships the same path wins, as mods always do.
   - Every strategy works unchanged. Hardlink and symlink share one layer between all sandboxes that use the same build. Direct copies it. Usvfs maps it.
   - The ledger records layer files like any other deployed file, so undo removes them and never removes a file it did not place.
6. **Deploy patches.**
   - A manifest's `deployPatches` are JSON edits to files in the game folder that are not ours, such as `launcher_profiles.json`.
   - They run after the merge, through the jail with root `gameDir`.
   - Each one is recorded in the ledger as a new entry kind:
     `JsonPatch { path, pointer, previous: Option<Value>, applied: Value }`.
   - Undo restores `previous`, but only if the current value at `pointer` still equals `applied`. That is the same "only if it is still ours" rule files follow.
   - With usvfs, patches go to a per-sandbox overlay copy (`<app data>/sandboxes/<id>/patched/`), mapped at the highest priority. The real file is never edited, which keeps usvfs's empty ledger true.
7. **Record.** `install_state` is written with the plan hash, the layers and the `launch_vars`.

Undo (`undeploy`) is unchanged in shape: it walks the ledger, which now includes layer files and JSON patches. Layers themselves are kept for reuse. `Settings → Storage → Clear unused layers` removes layers that no sandbox's `install_state` names. It is never automatic, because a layer may be gigabytes of Forge libraries that took minutes to build.

A partial deploy writes its ledger as today. JSON patches are applied after all files, so a failure during the merge leaves no patch behind.

### Step preview, progress and errors

- `src/components/catalog/install-plan-panel.tsx` renders a plan as a numbered list grouped by layer ("Minecraft 1.21.4", then "Forge 54.1.0"). Each step is one line drawn from the shared step-label table:
  - "Download installer (forge-1.21.4-54.1.0-installer.jar, 6.2 MB) from maven.minecraftforge.net";
  - "Run installer: java -jar forge-…-installer.jar --installClient <layer>". This always shows the full argv, with placeholders shown resolved;
  - "Edit launcher_profiles.json: set /profiles/tmc-ab12".
- Nothing is collapsed. A step can be expanded to show its JSON.
- The panel appears in three places:
  - **In Settings**, under the pickers, as a "What installing this does" disclosure. It is computed with `install_plan_preview`, which never downloads.
  - **In the consent dialog** (`src/components/catalog/install-consent-dialog.tsx`), when the deploy returns `NeedsConsent`. The dialog shows:
    - the plan;
    - the trust line ("Published by TMC, signed 2026-11-20" or "Local file, unsigned");
    - one checkbox per `consents` entry (for example, the game's EULA, with its link);
    - for exec steps, an explicit statement: "This runs a program from <host> on your computer with your user's permissions. TMC checked its fingerprint; it is not sandboxed."
    The buttons are **Install** and **Cancel**. There is no "don't ask again" across plan hashes.
  - **During the deploy**, as live progress. The executor emits `deploy://plan-step` events, `{sandboxId, planHash, layer, index, total, status: "running"|"done"|"skipped"|"failed", bytes?, totalBytes?, logTail?}`. For `runInstaller`, `logTail` is the last 20 lines of the installer's output, streamed.
- On failure the panel keeps the list. The failed step is marked, the executor's error is shown, and the full installer log opens from `<layer>.partial/work/installer.log`, which is copied to the app log directory before `.partial` is deleted.
- A skipped step (its `when` did not match) is shown as skipped, never hidden.

### Launch

- `launch.json` placeholders gain everything in `install_state.launch_vars`. Minecraft's rule changes from `"--version","{gameVersion}"` to `"--version","{launchVersionId}"`.
- Every plan defines `launchVersionId`. The vanilla manifest sets it to `{gameVersion}`, and loader manifests override it.
- An unmanaged sandbox launches with `launchVersionId = gameVersion`, which is the current behaviour.
- The vanilla manifest's deploy patch creates a launcher profile:
  - the key is `tmc-<sandboxId>`;
  - `lastVersionId` is `{launchVersionId}`;
  - `gameDir` is the target dir;
  - `javaArgs` comes from the memory options.
  The official launcher therefore shows "TMC: <sandbox name>" even when started by hand.
- `execSearch` is untouched. It still only finds the launch target (the official launcher). `runInstaller` never uses it: installers run under a runtime from the Runtimes catalog, by its pinned hash, never a `java` found on PATH or in Program Files.
- For running the game, the plan's runtime need sets `{javaPath}`. The value is the managed runtime when the option is `managed`, or when it is `auto` and no detected Java has the required major. Detection is Phase 2; at launch, `auto` falls back to managed.

### isolatedInstance

- Minecraft's `sandbox.json` keeps `isolatedInstance: true` (commit 74c8a33). The target dir is `<app data>/instances/minecraft/<name>`, so layers deploy into the instance, never the shared `.minecraft`.
- Each instance therefore has its own `versions/`, `libraries/` and `launcher_profiles.json`. The official launcher is started with `--workDir <instance>` and sees them.
- Library duplication across instances is avoided by the hardlink and symlink strategies, which point at the one layer.
- Minecraft keeps `direct` as its default strategy: the launcher rewrites files under `versions/`, and a symlinked file rewritten in place would change the shared layer. Hardlinks have the same hazard.
  - The layer checks therefore record sha256 per file.
  - Before reuse, a layer whose files no longer match is rebuilt, never trusted. This check is cheap, because it compares sizes and modification times first.

### usvfs

- The layer is a source in the virtual tree like any mod, so nothing special is needed. The installer never runs inside the virtual view: it runs in the layer directory, at build time.
- Deploy patches use the overlay copy described in "Deploy consumption", step 6.
- Usvfs still never falls back and still has an empty ledger.

### Migration of free-text values

This is library database migration N+1, stepwise like the others, in `library/db.rs`. For every sandbox:

1. `loader` is trimmed and lowercased.
   - `""`, `vanilla`, `none` and `minecraft` become NULL.
   - `neo forge`, `neo-forge` and `neoforged` become `neoforge`.
   - `fabricmc` becomes `fabric`, and `quiltmc` becomes `quilt`.
2. If `loader` still holds a version, it is split into the new `loader_version`. The patterns are, in order:
   - `^(fabric|quilt|forge|neoforge)[\s\-@:_]+v?([0-9][A-Za-z0-9._+\-]*)$`;
   - `^([0-9.]+)-forge-([0-9][0-9.]*)$` (the Forge version id `1.20.1-forge-47.3.0`). This also fills `game_version` if it was empty.
3. `game_version` is only trimmed. Values that are not in the catalog stay, and show as custom.
4. Anything left that matches no catalog loader key stays as is and shows as custom ("unknown loader 'optifine'"). Nothing is dropped.

The migration is pure string work with no network. It runs once, and the same function is exported for the sync path: remote `AppInstall` rows from older clients are normalised on pull. The website does not normalise them, because it does not own loader keys per game. Tests cover each pattern and the unchanged-unknown case.
