# The Console and the bundled tmc

The Console route is a terminal for the `tmc` command-line tool (`tmc-cli`, Python). Desktop only: Android keeps the "desktop only" screen. Code: `src-tauri/core/src/console.rs` (lookup, environment), `src-tauri/src/commands/console.rs` (commands), `src/routes/console.tsx`.

## Which tmc runs

The first of these that exists wins:

1. **The override**: the `TMC_CLI_PATH` environment variable, naming a `tmc` program. A set but wrong path is not skipped silently; the Console reports it as missing, so a typo is visible.
2. **The bundled tmc**: a program called `tmc` (`tmc.exe` on Windows) in the same folder as the app's own executable.
3. **A tmc on PATH**, then the usual user places (`~/.local/bin`, Homebrew, `/usr/local/bin`, pip-user Scripts).
4. **Nothing**: the Console says so and shows `pip install tmc-cli`.

So an installed `tmc` on PATH does not shadow the bundled one; set `TMC_CLI_PATH` to use another build. The Console header shows "built in" for the bundled one and the path otherwise. `console_probe` returns this as `source` (`bundled`, `override`, `path`).

The resolution is the pure function `console::resolve`, tested with a fake folder layout; `bundled_cli_runs_through_the_console_path` runs a real frozen `tmc --version` and `--help` through it.

## Where the bundled tmc is

It is a Tauri sidecar (`externalBin`), a PyInstaller one-file build of tmc-cli, so there is no Python requirement. Tauri drops the target triple, so it sits beside the app: AppImage `usr/bin/tmc`, deb `/usr/bin/tmc`, Windows the install folder (NSIS and MSI), macOS `Contents/MacOS/tmc`.

- Not in the **portable Windows exe** (a bare file): it falls back to PATH.
- Not in `tauri dev` or plain `cargo` builds: `externalBin` is only passed for bundling, through `src-tauri/tauri.bundle-tmc.conf.json` (declaring it in `tauri.conf.json` would make every build fail without the file).
- Not on Android and iOS.

## Which revision

`packaging/tmc-cli.json` pins the tmc-cli repository and a commit. To update the bundled CLI, change `ref` and commit; a tag rebuilds exactly that.

## Building it

- Local, for this machine: `npm run build:tmc-cli` (`scripts/build-tmc-cli.sh`). It makes `src-tauri/binaries/tmc-<triple>` (git-ignored) in a throwaway venv, and checks `--version` against the source. `--src <checkout>` or `TMC_CLI_SRC` builds a local tmc-cli instead of the pin; `TMC_CLI_TOKEN` fetches a private repo.
- `npm run test:tmc-cli` builds it and runs the Rust test that starts it through the Console lookup.
- `npm run build:linux` builds the sidecar when missing and bundles it (`--no-tmc-cli` skips).
- Release: the `tmc-cli` job in `.github/workflows/release.yml` builds one per target on its own runner (PyInstaller cannot cross-compile): Linux x86_64, Windows x86_64, macOS arm64 and x86_64. The `build` job downloads them; macOS joins the two with `lipo` into `tmc-universal-apple-darwin`. The `tmc-cli` job in `ci.yml` builds and tests it on every push.
- The Windows build from Linux (`npm run build:windows`) cannot make `tmc.exe`. Put one built on Windows at `src-tauri/binaries/tmc-x86_64-pc-windows-msvc.exe` and the NSIS installer and the hand-built MSI include it; without it the build says so and the Console uses PATH.

## Updates and signing

The sidecar is inside every bundle, so the updater payload carries it and the update signature covers it; a new app version ships whatever `packaging/tmc-cli.json` pins. Windows and macOS code signing, when configured, sign the sidecar with the app (Tauri does this for `externalBin`).

## The child environment

An AppImage's `AppRun` points `LD_LIBRARY_PATH`, `PATH` and friends into its own mount. Those entries are removed before `tmc` starts (`console::sanitised_env`), and `PYTHONHOME` and `PYTHONPATH` are always dropped, so the child neither loads the app's libraries nor a foreign Python.

## Size

About 9 MB per platform added to the installer before compression (the frozen interpreter and the CLI).
