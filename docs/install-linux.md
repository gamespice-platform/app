# Installing on Linux

The one-liner:

```sh
curl -fsSL https://gamespice.net/install.sh | sh
```

`gamespice.net/install.sh` redirects to `https://github.com/gamespice-platform/app/releases/latest/download/install.sh`, the copy of `scripts/install.sh` the release workflow attaches to every release. That GitHub URL is the stable one and works on its own; the site route is the short name for it. Pass options after `sh -s --`, for example `sh -s -- --yes --channel beta`. `wget -qO- <url> | sh` works too.

## What it does

1. Detects the architecture (x86_64 and aarch64; anything else stops with a message), the distribution from `/etc/os-release` (`ID` and `ID_LIKE`), the package manager, an immutable or image-based root, WSL, containers, the session type (Wayland, X11, headless), the init system, musl, and whether FUSE 2 is usable.
2. Offers the prerequisites for this package manager, if any are missing.
3. Resolves the release, downloads the AppImage, and verifies it. Nothing is written to the install location until the download has verified.
4. Installs the AppImage to a per-user location, then registers the launcher entry, the icons and the `gs://` scheme.

The script is POSIX `sh` (dash, bash and busybox ash all run it). It is one `main` call at the end of the file, so a truncated download runs nothing.

## Where it installs, and why updates work

By default everything lands under `~/.local`: the program at `~/.local/bin/gamespice-app`, the launcher at `~/.local/share/applications/gamespice-app.desktop`, icons under `~/.local/share/icons/hicolor`, and a manifest of every file at `~/.local/share/gamespice-app-installer/manifest`.

The AppImage is a real file the user owns. The app's own updater (see [updater.md](updater.md)) replaces that file in place, so the app keeps itself current with no timer, no service and no root. There is no command-line update check in the app, so the installer does not register a scheduled job: re-running the installer is the manual update (it does nothing when the latest version is already installed; `--force` reinstalls).

`--system` installs under `/usr/local` for all users and needs root. The app then cannot replace its own binary as a normal user, and the installer says so. On an immutable system `--system` is refused; install per-user.

## Package managers and distributions

| Family | Detected by | Prerequisite command offered |
|---|---|---|
| Debian, Ubuntu, Mint, Pop, Raspbian | apt | `apt-get install libfuse2t64` (or `libfuse2`), `fuse3`/`fuse` |
| Fedora, RHEL, Rocky, Alma, CentOS | dnf, yum | `dnf install fuse-libs` |
| openSUSE, SLES | zypper | `zypper install libfuse2` |
| Arch, Manjaro, EndeavourOS, CachyOS | pacman | `pacman -S --needed fuse2` |
| Alpine | apk | none: musl, see below |
| Gentoo | emerge | `emerge sys-fs/fuse:0` |
| Void | xbps | `xbps-install fuse` |
| Solus, Clear Linux | eopkg, swupd | eopkg: `fuse2` |
| NixOS | nix | none: prints the `programs.appimage` snippet |
| Fedora Silverblue and Kinoite, Bazzite, Bluefin, MicroOS | rpm-ostree, transactional-update | none: nothing is layered, extract-and-run is used |
| SteamOS (Steam Deck) | pacman on a read-only root | none: extract-and-run from the home directory |

curl (with ca-certificates) is offered when neither curl nor wget exists; `xdg-utils` and `desktop-file-utils` are offered on a desktop session that lacks them. Flatpak is detected and reported only; the app is not shipped as one.

**The installer never runs sudo silently.** A prerequisite command runs only when you answer yes on the terminal (it reads `/dev/tty`, so it works when the script itself arrives on stdin) or pass `--yes`. Without either it prints the exact command to copy. Root, `sudo`, `doas` and `pkexec` are supported; with none of them and not root, it prints the command for root.

## FUSE and the sandboxed case

An AppImage mounts itself with FUSE 2. When `libfuse.so.2`, `/dev/fuse` or `fusermount` is missing (Ubuntu 24.04 and Debian 13 by default, containers, WSL, immutable systems) or `--extract` is given, the installer keeps the AppImage under `~/.local/lib/gamespice-app/` and writes `bin/gamespice-app` as a two-line wrapper that sets `APPIMAGE_EXTRACT_AND_RUN=1` and `exec`s it. The app starts the same way and the updater still replaces a real AppImage file.

## musl

Alpine and other musl systems cannot run the glibc-built AppImage. The installer stops with that explanation; use a glibc distribution or a distrobox container, or `--force` to try anyway.

## Verification

The AppImage is checked against the release's `checksums.txt` (required; `--no-verify` is the explicit opt-out). Where `minisign` is installed the `.sig` file is verified too against the release key (the one in `src-tauri/updater.pub`, override with `GS_PUBKEY`); without minisign the installer says the signature was not checked. A mismatch aborts before anything is installed. Every download is HTTPS-only, redirects included (curl `--proto =https --proto-redir =https`, wget `--https-only` where it has it), so a redirect cannot downgrade to plain HTTP. `GS_BASE_URL` may be `https://`, `file://`, or `http://` with a warning; any other scheme is refused, and `GS_VERSION` and `GS_REPO` must look like a tag and `owner/name`. Downloads and installs are staged next to the destination and renamed into place, and a trap removes the temp directory on exit, interrupt and termination, so a failed run leaves the previous install untouched.

## Icons

The release attaches `GameSpice-App-icons.tar.gz`, the hicolor set from `packaging/linux/icons/hicolor` (16 to 512 px, all derived from `src-tauri/icons/app-icon.png`, the GameSpice flask mark), plus the `x-scheme-handler-gs` mime icon. The installer unpacks it to `~/.local/share/icons/hicolor/<size>/apps/gamespice-app.png`, matching `Icon=gamespice-app` in the `.desktop` file. If the tarball is unavailable it extracts the icons from the AppImage. It then runs `gtk-update-icon-cache` and `update-desktop-database` where present and registers `gs://` with `xdg-mime`. There is no scalable SVG because the repo has none.

Regenerate the set from `app-icon.png` if the icon changes; the tarball is built by the release workflow from that directory.

## Flags and environment

`--update`, `--uninstall`, `--version [TAG]`, `--channel stable|beta`, `--prefix DIR`, `--system`, `--dry-run`, `--yes`, `--no-desktop`, `--no-verify`, `--no-verify-signature`, `--extract`, `--force`, `--proxy URL`, `--help`. Environment: `GS_REPO`, `GS_VERSION`, `GS_CHANNEL`, `GS_PREFIX`, `GS_APPIMAGE` (install a local file), `GS_BASE_URL` (download assets from a mirror; needs `GS_VERSION`), `GS_YES`, `GS_DRY_RUN`, `GS_NO_DESKTOP`, `GS_PUBKEY`. Flags win. `HTTPS_PROXY`, `ALL_PROXY` and `NO_PROXY` are honoured by curl and wget. The `beta` channel reads the newest release from the repository's `releases.atom` feed, since `/releases/latest` never names a pre-release and the feed has no rate limit.

`--dry-run` prints one `plan: key=value` line per detected fact and downloads and changes nothing. Offline, it fails with a message that names `HTTPS_PROXY` and `GS_APPIMAGE`.

`--uninstall` removes the files in the manifest, the wrapper, the launcher, every icon and the legacy `tmc` entries, and drops `gamespice-app.desktop` from the `x-scheme-handler/gs` and `x-scheme-handler/tmc` keys of `mimeapps.list` (`~/.config` and the data dir). It leaves the user's settings and library alone. With `--dry-run` it only prints what it would remove. The legacy `bin/tmc` is removed only when it is the old AppImage or a link into `lib/tmc/`: `tmc` is also the TMC command-line tool the app's Console runs, and the installer never deletes that.

## Testing

`scripts/install.test.sh` builds a fake root per distribution (an `os-release` plus stub package managers on `PATH`), asserts the `--dry-run` plan for 20 profiles, then installs a fake release over `file://` into a temp prefix and checks the files, icons, checksum refusal, idempotency, the AppImage icon fallback, `--uninstall` (and its dry run, the `mimeapps.list` cleanup, and a foreign `bin/tmc` surviving), and a prefix containing spaces, quotes and `$`. Run it under `sh`, `dash`, `bash` and `busybox sh` (`INSTALLER_SHELL=dash sh scripts/install.test.sh`); it needs no root, network or container. What it cannot cover is the real package managers, a real FUSE mount and `minisign`.
