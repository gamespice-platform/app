# The library, sandboxes, and how mods reach the game

## The library

The device's answer to the account's subscriptions: which of them this machine holds, which are on disk, and how they got there.

> The Library SCREEN has three views and they are genuinely different nouns: **Games** is the catalogue of every game, **On this device** lists what somebody else's launcher installed and this app mods, then what this app installed from a build TMC published (see [playing.md](playing.md#games-tmc-publishes)), and **Subscribed** is what the account subscribed to. One list holding all of them would have to explain, per row, which kind of thing it was.

**Library > Mods** (`/installed-mods`) is the per-mod view of the same data: installed or disabled subscriptions plus imported local mods. Disabling a subscription removes its files and pauses it (a paused row is never installed by sync), so the row stays listed and Enable resumes and installs it. Open folder is `library_reveal`/`local_reveal`, which take an id and reveal the first installed file, or the local mod's store folder, in the OS file manager; the webview never sees the path.

**New sandbox offers every game this build has a manage rule for**, resolved from the supported slugs with `/apps?slugs=` (`lib/library/sandbox-games.ts`). It used to be the site's mod facets, which only list games with a mod published on the site, so a game set up through Find games with no site mods (Minecraft) was missing.

**SQLite, not a JSON file** (`library/db.rs`). Everything else the app persists — settings, the plugin registry — is a small file rewritten whole, and that is right for those: read at launch, written on a click, never contended. The library is not like that. The sync loop writes it on a timer while the UI reads it on every render and the installer writes single rows mid-install, and a whole-file rewrite under that pattern loses one writer to another exactly when somebody is watching a progress bar.

Schema changes are **stepwise** — each migration takes the database from `n-1` to `n` and runs only if it has not. Re-running one batch happens to be safe today because every statement is `IF NOT EXISTS`, and stops being safe the first time a step needs an `ALTER TABLE`.

**It polls** (`library/sync.rs`), because a subscription can be created in a BROWSER and there is no push channel to an installed desktop app that is reliable across three desktop platforms, two mobile ones, corporate firewalls and sleeping laptops. A watermark makes that cheap: each response carries a `revision`, the next request sends it back, and a device open for an hour has made sixty requests and transferred one row.

**A full sync still happens** every `FULL_SYNC_EVERY` passes and always on launch, because a delta cannot express a DELETION — there is nothing left to poll. Anything the device holds that the server did not send has been unsubscribed elsewhere.

**An upsert from the server never touches the device-local columns.** That is what stops a resync forgetting what is installed, which is the single easiest way to turn a working library into an endless reinstall loop.

**The plan is executed in Rust, not returned to the webview.** A frontend that decided what to install is one an injected script can talk into installing something.

## Sandboxes, and how mods reach the game

A **sandbox** is what other mod managers call a profile or an instance: a named set of mods with its own load order, its own deployment method and its own launch settings.

> The word is why `plugins::jail` is called a jail. It used to be `plugins::sandbox::Sandbox` — the path jail — and two `Sandbox` types in one crate, one of them a security boundary, is a mistake waiting for somebody to reach for the wrong one.

**On the wire a sandbox is an `AppInstall`.** The cloud model predates the word and `/api/app/v1/installs` is shipped, so renaming the endpoint would 400 every request from every installed copy of the app. The app says "sandbox"; the API says "install"; they are the same thing.

### What lives where

By the same test the settings use — *would this be wrong to apply on a different machine?*

| Fact | Where | Because |
| --- | --- | --- |
| name, mods, order, options, launch flags | the account (`AppInstall`) | signing in on a second machine should reproduce it |
| which folder it deploys into | the device | that path exists on one machine |
| which release of each mod is staged | the device | so does that |
| the deployment ledger | the device | it describes files on one disk |

The cloud half is **optional per sandbox**. `cloudSync` off means it is never sent anywhere and a reconcile cannot delete it — which is the whole answer to "I do not want my mod list on your server", and it has to be per-sandbox rather than global because the useful case is "sync my modpack profiles, not the one I use for testing".

### Two steps, not one

```text
  stage   — run the game's install rule with the sandbox's own staging folder
            standing in for the game directory
  deploy  — mirror every staged file into the real game folder, by whichever
            mechanism the sandbox is set to
```

Splitting them is what makes the whole feature work:

- **Every install rule written before sandboxes existed still works.** A rule that copies to `mods/{fileName}` writes to `<staging>/<sandbox>/<mod>/mods/foo.jar`, and deployment puts `mods/foo.jar` in the game folder. The rule never learns which strategy is in use, and it should not — "where does this game's mod go" and "how do files reach the game folder" are different questions.
- **Switching sandboxes is a link operation, not a download.** Staging survives an undeploy.
- **Nothing half-downloaded is ever in the game folder.**

`pluginData` is scoped per sandbox and per mod (`jail_for_scoped`). Two sandboxes staging the same mod run identical steps with an identical `{fileName}`; a shared scratch directory meant the second run's download landed on the first's, and a sandbox pinned to an older release quietly got the newer file.

### The four strategies

| Strategy | Game folder | Cost | Fails when |
| --- | --- | --- | --- |
| `direct` | modified | a full copy | never; it is the fallback |
| `hardlink` | link pointers | nothing | staging is on another drive |
| `symlink` | link pointers | nothing | Windows without Developer Mode |
| `usvfs` (EXPERIMENTAL) | untouched | nothing | not Windows, not built with `usvfs-hooks`, or the game has kernel anti-cheat |

#### USVFS: the one that deploys nothing

**EXPERIMENTAL.** Minecraft Java lists it as a supported strategy (`plugins/app/minecraft/sandbox.json`) but the default stays `direct`. It has never been run on Windows by its authors; see the table below for what is tested.

The user-space virtual filesystem approach, in `src-tauri/usvfs/` (`tmc-usvfs`): the mods stay in staging and the game is *told* they are there, by a DLL injected before its entry point runs that patches the file calls in its import table (`CreateFileW`, `GetFileAttributesW`/`ExW`, `FindFirstFileW`/`ExW`, `FindNextFileW`, `FindClose`) and answers them from a merged in-memory tree. Directory listings are merged with the real folder, so a launcher that enumerates `mods/` sees the staged jars. The process-creation calls (`CreateProcessW`, `CreateProcessAsUserW`) are hooked too: a child of the game is created suspended, gets the same DLL and the same tree, then resumes, so a wrapper that spawns the real `java.exe` stays virtual. Only children this process just created are ever injected, never an arbitrary pid; 32-bit children are skipped.

```
  app                              game process
  ───                              ────────────
  build the merged tree
  publish it            ──blob──▶
  launch suspended
  inject the hook DLL   ──────▶    DllMain → read the tree, patch the IAT
  resume                ──────▶    every open of a virtual path is answered
                                   from staging
```

**Deploying is not a file operation**, and everything downstream follows from that:

- the merged tree is serialised to a blob inside the sandbox's own staging folder, and **that publish IS the deploy**;
- **the ledger comes back empty**, which is the correct record rather than a gap — it names files in the game folder and there are none, so `purge` correctly does nothing and `verify` correctly reports a healthy folder;
- **a sandbox switching TO it still purges what the last strategy left**, which is the one write to the game folder a virtual deploy performs. Without it the user gets both: every file twice, with the virtual copy winning only where the tree happens to cover it;
- **there is no fallback in either direction.** Whether the game folder is modified at all is the reason somebody picks this, so silently linking instead would dirty a folder chosen to stay clean, and silently going virtual would leave a game that was never told about the mods.

**The revision is a content hash, not a counter or a clock.** The injected DLL compares it to notice that a blob it already read has changed; a counter would also tick for a redeploy producing an identical tree.

**It is gated twice, at deploy and at launch** (`deploy::usvfs_unavailable`, `spawn::launch_with_vfs`), on Windows AND on the `usvfs-hooks` feature, which the app crate now enables **by default** (`tmc-core` itself keeps it off so its own tests do not need the Windows crate's extras; a build without it refuses a virtual deploy with "not included in this build"). The two are halves of one decision: a build with only one on could publish a tree no launch would carry, or inject with nothing to inject. The refusals are separate strings because they are separate facts — "this is Windows-only" can never change for a Mac user, and "this build has it off" can.

Why the gate exists at all is a confidence split, stated in `usvfs/src/lib.rs`:

| Half | State |
| --- | --- |
| `tree` — the merged view, resolution, listings, case rules | **Tested**, on every platform |
| `shm` — publishing and reading the blob atomically | **Tested**, including every malformed input |
| `env` — rebuilding a child's environment block | **Tested**, on every platform |
| `tree` listings — merging real and virtual directory entries | **Tested**, on every platform |
| `hooks` — IAT patching, `CreateFileW`, the attribute and `Find*` calls, `CreateProcess*` propagation | **Type-checked against `x86_64-pc-windows-gnu` and clippy-clean. Never run against a game.** |
| `inject` — suspended launch, remote `LoadLibrary` | **Type-checked.** Argument quoting and environment building are tested; the launch is not |

`cargo check -p tmc-usvfs --target x86_64-pc-windows-gnu` is how that type-checking happens and it is worth keeping working — it has found real bugs reading missed, such as `IMAGE_NT_HEADERS64` living in `Diagnostics::Debug` while being gated behind the `Win32_System_SystemInformation` feature. `tmc-core` cannot be cross-checked that cheaply (bundled SQLite needs a C cross-compiler, which only the `cargo-xwin` toolchain in [BUILDING.md](BUILDING.md) supplies), which is exactly why the injector is its own dependency-light crate.

**What it cannot do, even when it works:** calls through `GetProcAddress` are not redirected (the pointer never came from an import table), direct `ntdll` syscalls are not redirected, writes are not redirected (no copy-on-write: a mod that rewrites its own jar writes into the real folder), `CreateProcessA`/`CreateProcessWithLogonW`/`CreateProcessWithTokenW` are not hooked, and every kernel-level anti-cheat treats injection as an attack — correctly. A game declaring `antiCheat: "kernel"` in its `sandbox.json` is refused this strategy at deploy (`library::deploy::deploy_sandbox`) and shown disabled in the picker (`strategies_for`).

**The DLL ships beside the exe.** `src-tauri/tauri.windows.conf.json` (merged on Windows builds only) builds `tmc-usvfs` explicitly, because a dependency's `cdylib` is not produced as a side effect of building the app, and bundles `target/release/tmc_usvfs.dll` as a resource next to the exe, which is where `spawn` looks for it. This bundling is configured but has not been exercised on Windows; a cross-triple `--target` build puts the DLL under `target/<triple>/release/` and the resource path would need to follow.

**A URI launch rule and a virtual deploy are refused together.** A game started through `steam://` is started by Steam, and there is no process of ours to inject into; the alternative starts the game with none of the sandbox's mods and reports success, which is the hardest kind of bug to diagnose because the folder is stock and there is nothing to find.

**Capability is probed, not assumed.** `deploy::link::probe` creates one file and links it, twice, and reports what worked. Every rules table for this is wrong somewhere — a Linux box with staging on an exFAT USB drive, a macOS volume with links disabled, a Windows machine with Developer Mode on.

**Falling back between the two LINK strategies is automatic and reported. Falling back to copying is not.** Copying is not a worse link, it is a different decision: it writes gigabytes, it modifies the game's own files, and its failure mode is a game folder that needs restoring rather than unlinking. A sandbox that asked for links and can have neither gets an error naming both fixes.

### Three properties that hold across every strategy

- **Nothing is overwritten.** A file already in the game folder that the app did not put there is MOVED to the backup store before its place is taken, and a purge puts it back.
- **Nothing is removed unless it is still ours.** Every removal re-checks the file against the ledger row that claims it — a symbolic link must still point at its staging file, a hard link must still share its identity, a copy must still have its recorded size and mtime. Anything else is left alone and reported, so a config the user edited after deploying survives.
- **Staging is never modified.** Deployment only ever reads from it, which is what lets two sandboxes share one downloaded copy of a mod.

### The ledger

`deploy::ledger` is the only record of which files in somebody's game folder belong to us, and undeploying by rescanning cannot replace it: a rescan can see that `Data/textures/sky.dds` exists, and cannot see whether the app put it there or whether it shipped with the game. Guessing wrong either strands a modded file forever or deletes a base-game asset.

It is written even when a deploy reported errors — a partial deploy put files on disk, and losing the record of them creates exactly the orphans the ledger exists to prevent.

### Layers

A sandbox with a game version or a loader also gets **layers**: the files an install manifest produces (a loader's jars and version JSON, an installer's output), built once per content hash under `<data>/layers/<app>/<layer hash>/` and shared by every sandbox that resolves to the same hash. The plan, the manifests and the consent rules are in [loaders-and-versions.md](loaders-and-versions.md#deploy-consumption) and [loader-manifests.md](loader-manifests.md); what matters here is how a layer meets the game folder.

- A layer is merged into the deploy as the lowest-priority source, so a mod with the same path wins, and it goes through the same strategies, ledger and purge as any mod.
- A layer is built in a `.partial` folder and renamed into place only when it is complete, then verified again each time it is reused. A file that changed since the build fails verification and the layer is rebuilt.
- **A finished layer is sealed.** On Linux and macOS the write bits come off every file under the layer's `files/`, so an edit through a hard link fails instead of changing the copy every other sandbox shares. A Direct (copy) deploy makes its copy writable again, because that copy belongs to the game folder. Windows does not seal yet ([gaps.md](gaps.md)); there the next verify still catches the edit and rebuilds the layer.
- Deploy patches (a `launcher_profiles.json` profile, for instance) are a `JsonPatch` ledger entry. Undo restores the original bytes when the file is untouched, and otherwise puts back only the values that are still ours, so a value the user changed is kept. Under USVFS the patch goes to an overlay copy and the real file is never touched.
- A plan that runs code (an installer, a runtime download) needs consent, which is stored per sandbox and **plan hash**. Recording consent for a new hash deletes the sandbox's consents for every other hash in the same transaction, so going back to an earlier plan asks again instead of replaying an old yes.
- Each step of each layer reports `deploy://plan-step` (running, done, skipped, failed with the end of its log) to the deploy-progress view.

### Conflicts

The merge tree decides one winner per path before anything touches the disk, by priority, with ties broken on the mod key so the answer is stable across runs. Losers are **reported**, never merged: nothing here understands any game's file formats, and a manager that silently produces a file neither author wrote is how "it works for me" bug reports are made.

Sorting happens inside `merge::build`, not in the caller. A tree built from an unsorted list is wrong in a way that looks right until somebody reorders their list and nothing changes.

### `sandbox.json`

The strategy matrix per game, plus presets, options and detection hints:

```json
{
  "manifestVersion": 1,
  "sandbox": {
    "deploy": {
      "defaultStrategy": "symlink",
      "supportedStrategies": ["symlink", "hardlink", "direct"],
      "antiCheat": "kernel",
      "modTargets": [{ "type": "loader_mod", "relPath": "mods" }],
      "notes": ["Shown verbatim in the sandbox's settings"]
    },
    "presets": [{ "id": "fabric", "label": "Fabric", "loader": "fabric" }],
    "options": [{ "key": "memoryMb", "label": "Memory", "type": "int", "max": 65536 }],
    "detect": { "steamAppIds": ["271590"], "markers": ["Game.exe"] }
  }
}
```

- **`options`** is a *form description* and nothing more. It cannot express a condition, a computation or a dependency between fields — a settings schema that can do those is a program, and the plugin model rests on plugins not being programs. A game needing one needs a second preset instead.
- **Values are clamped against the schema on every write**, and a key the schema does not declare is dropped. The options reach the cloud and come back, so "the server said 900 GB of heap" is answered here rather than by a game that will not start.
- **`antiCheat: "kernel"`** is load-bearing. Kernel-level anti-cheat systems watch for a game folder whose files are not where they should be, so a game declaring it gets Direct as its default and a warning on every other option — the cost of guessing wrong is somebody's account, not a failed install.

Options reach a command line only through `optionArgs` in the game's own `launch.json`. `LaunchOptions` carries them in a flattened `extra` map; an object or an array contributes nothing, and there is still no shell.

## Exporting a file

`sandbox_export_pack(id, kind)` writes a file into the downloads folder (`download_target_dir()`) and returns its name, file count and size. The webview names an id and `"mrpack"` or `"share"`, never a path; the file name is the sandbox name made safe, made unique, and never overwrites.

The sandbox screen's Actions row has the buttons: `ipc.sandboxExportPack` (zod-parsed `ExportReportSchema`) and a notice with the saved name, size and the export's notes.

- **`mrpack`** (`library::export`): the sandbox's enabled, staged mods go in as `overrides/`, resolved through the same merge tree a deploy uses (so priority winners are what is written, and symlinks are skipped). `files` is empty: a subscribed mod has no host the format permits, so its bytes travel in the archive. Only Minecraft sandboxes with a game version export. The loader goes in as its dependency key (`fabric-loader`, `quilt-loader`, `forge`, `neoforge`) with the sandbox's `loaderVersion`; a loader with no recorded version, or with no `.mrpack` key, is left out and the report says why, because a made-up version would read as a promise. The report's notes list skipped and unstaged mods. A test imports the result with the real importer and compares SHA-512 of every file.
- **`share`**: a zip of the same `sandbox.json` and code `share` produces, with nothing about this machine.

Both are written to a `.tmcpart` file and renamed on success, so a failure leaves nothing behind.

## Sharing a sandbox

A code, not a file. `library::share` encodes a sandbox as `TMC1-<base64>`.

**Why a code.** A file export needs a path to write and a file import needs a path to read, and the webview names neither — the one command that takes a path returns directory names and reads nothing. Trading that guarantee for a save dialog is not a trade worth making, and a code is what people do with an exported profile anyway: put it in a message.

**Item ids, never files.** Shipping the files would make this a redistribution channel for other people's work, with no download counted, no licence respected and no update path.

**Nothing about this machine.** No game directory, no launch environment, no ledger, no staging path — the same test the settings split uses, and a code is by definition going somewhere else. A game folder in a shared code is also somebody's username in a chat message. `a_code_carries_nothing_about_this_machine` decodes an export and asserts none of it is there.

**Importing previews first**, because a code is a string from somebody else and pressing Import can make two hundred subscriptions on an account. It subscribes and adds; it does not stage — two hundred mods is two hundred downloads, and starting them inside a command the UI is awaiting is a frozen dialog with no queue to look at. `cloudSync` and `autoUpdate` come from the importer's own defaults: whether somebody's mod list leaves their machine is their decision, not the exporter's.
