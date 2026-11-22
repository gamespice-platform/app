#!/bin/sh
# GameSpice App - Linux installer.
#
#   curl -fsSL https://gamespice.net/install.sh | sh
#   curl -fsSL https://gamespice.net/install.sh | sh -s -- --yes --channel beta
#   curl -fsSL https://github.com/gamespice-platform/app/releases/latest/download/install.sh | sh
#
# Read docs/install-linux.md for the full description. In short: it detects the
# architecture, distribution, package manager, immutable/atomic root, WSL,
# container, session type and FUSE, then installs the release AppImage to a
# per-user location the app's own updater can write to, verifies it against
# the release's checksums.txt (and its minisign signature when `minisign` is
# installed), and registers the launcher, icons and the gs:// scheme.
#
# It is written to be piped into a shell: POSIX sh (dash, bash, busybox ash);
# the whole script is one `main` call at the very end, so a truncated download
# runs nothing; nothing is changed until the download is verified; it never
# runs sudo/doas/pkexec silently (it asks on the terminal, or on --yes, or
# prints the exact command); and every external command gets /dev/null as
# stdin so nothing can eat the rest of the script.

SCRIPT_VERSION=1

# The public half of the key every release is signed with (src-tauri/updater.pub,
# base64 of a minisign public key file). Override with GS_PUBKEY.
DEFAULT_PUBKEY='dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEY3QjkyN0U4NDg2REQxRkIKUldUNzBXMUk2Q2U1OTRCMmg1K2FiQkdkTmtidlFtRldjUE5xbTJxWHkzK2dLK1dlMG9ZZ1JlSkwK'

usage() {
    cat <<'USAGE'
GameSpice App installer for Linux

usage: install.sh [options]

  --update            re-run as an update (same as running again)
  --uninstall         remove everything this installer wrote
  --version [TAG]     with TAG: install that release (e.g. v0.5.5);
                      without: print the installer and installed versions
  --channel NAME      stable (default) or beta: which release to resolve
  --prefix DIR        install under DIR (bin/, share/); default ~/.local
  --system            install for all users under /usr/local (needs root;
                      the app then cannot update itself unless run as root)
  --dry-run           detect and print the plan; change nothing, download nothing
  -y, --yes           answer yes to the prerequisite-install prompt
  --no-desktop        install only the program: no launcher, icons or gs://
  --no-verify         skip checksums.txt (refused by default)
  --no-verify-signature  skip the minisign signature check
  --extract           run the AppImage by extract-and-run (no FUSE needed)
  --force             reinstall the same version; proceed on musl systems
  --proxy URL         use this proxy (also honours HTTPS_PROXY / ALL_PROXY)
  -h, --help          this text

environment (flags win):
  GS_REPO        owner/name to download from        (gamespice-platform/app)
  GS_VERSION     release tag                        (latest)
  GS_CHANNEL     stable | beta
  GS_PREFIX      install prefix
  GS_APPIMAGE    install this local AppImage instead of downloading
  GS_BASE_URL    download assets from here instead of the GitHub release
  GS_YES=1  GS_DRY_RUN=1  GS_NO_DESKTOP=1  GS_PUBKEY=<base64 minisign key>
USAGE
}

say()  { printf '%s\n' "$*"; }
warn() { printf 'warning: %s\n' "$*" >&2; }
die()  { printf 'error: %s\n' "$*" >&2; exit 1; }
# One machine-readable line per detected fact; the test harness asserts these.
plan() { printf 'plan: %s=%s\n' "$1" "$2"; }
have() { command -v "$1" >/dev/null 2>&1; }
# $1 as one single-quoted sh word: safe for any path, including spaces, $ and quotes.
shquote() { printf "'%s'" "$(printf '%s' "$1" | sed "s/'/'\\\\''/g")"; }
# $1 as a quoted argument of a desktop-entry Exec key (spec: double quotes,
# with \, ", ` and $ escaped; then each \ doubled again as a string value).
desktop_quote() { printf '"%s"' "$(printf '%s' "$1" | sed -e 's/%/%%/g' -e 's/[\\"`$]/\\&/g' -e 's/\\/\\\\/g')"; }

# ------------------------------------------------------------------ detection

osr() {
    # the value of KEY in the os-release file, quotes stripped
    [ -r "$OSREL" ] || return 0
    sed -n "s/^$1=//p" "$OSREL" | head -n 1 | tr -d "\"'" || true
}

detect_arch() {
    m="${GS_UNAME_M:-$(uname -m)}"
    case "$m" in
        x86_64|amd64) ARCH=x86_64; ASSET_ARCH=amd64 ;;
        aarch64|arm64) ARCH=aarch64; ASSET_ARCH=aarch64 ;;
        *) die "no AppImage is published for $m. GameSpice App builds exist for x86_64 and aarch64 Linux only (macOS and Windows have their own downloads)." ;;
    esac
}

detect_distro() {
    OSREL="${GS_OS_RELEASE:-$R/etc/os-release}"
    [ -r "$OSREL" ] || OSREL="$R/usr/lib/os-release"
    ID="$(osr ID)"; ID_LIKE="$(osr ID_LIKE)"
    VARIANT_ID="$(osr VARIANT_ID)"; NAME="$(osr PRETTY_NAME)"
    [ -n "$NAME" ] || NAME="${ID:-unknown}"
    family=unknown
    for t in $ID $ID_LIKE; do
        case "$t" in
            debian|ubuntu|linuxmint|pop|raspbian|neon|elementary|zorin|kali|devuan|mx|deepin|pureos) family=debian ;;
            fedora|rhel|centos|rocky|almalinux|ol|amzn|nobara|scientific) family=fedora ;;
            opensuse*|sles|sled|suse) family=suse ;;
            arch|manjaro|endeavouros|cachyos|garuda|artix|arcolinux|steamos) family=arch ;;
            alpine|postmarketos) family=alpine ;;
            gentoo|funtoo|calculate) family=gentoo ;;
            void) family=void ;;
            nixos) family=nixos ;;
            solus) family=solus ;;
            clear-linux-os) family=clear ;;
        esac
        [ "$family" = unknown ] || break
    done
}

detect_immutable() {
    immutable=no
    if [ -e "$R/run/ostree-booted" ]; then immutable=ostree; fi
    case "$ID:$VARIANT_ID" in
        steamos:*) immutable=steamos ;;
        nixos:*) immutable=nixos ;;
        bazzite:*|bluefin:*|aurora:*|*:silverblue|*:kinoite|*:sericea|*:onyx|*:iot|*:coreos|*:cosmic-atomic) immutable=ostree ;;
        opensuse-microos:*|opensuse-aeon:*|opensuse-kalpa:*|*:microos) immutable=microos ;;
        vanilla:*|vanilla-os:*) immutable=abroot ;;
    esac
    return 0
}

detect_pm() {
    pm=unknown; pm_all=""
    for c in rpm-ostree transactional-update apt-get dnf yum zypper pacman apk xbps-install emerge nix-env swupd eopkg; do
        have "$c" && pm_all="$pm_all $c"
    done
    pm_all="${pm_all# }"
    case "$family" in
        debian) want=apt-get ;; fedora) want=dnf ;; suse) want=zypper ;;
        arch) want=pacman ;; alpine) want=apk ;; gentoo) want=emerge ;;
        void) want=xbps-install ;; nixos) want=nix-env ;; solus) want=eopkg ;;
        clear) want=swupd ;; *) want="" ;;
    esac
    if [ "$immutable" = ostree ] && have rpm-ostree; then pm=rpm-ostree
    elif [ "$immutable" = microos ] && have transactional-update; then pm=transactional-update
    elif [ -n "$want" ] && have "$want"; then pm="$want"
    elif [ "$family" = fedora ] && have yum; then pm=yum
    else
        for c in $pm_all; do pm="$c"; break; done
    fi
    # Normalise the names the plan shows.
    case "$pm" in
        apt-get) pm=apt ;; xbps-install) pm=xbps ;; emerge) pm=emerge ;; nix-env) pm=nix ;;
    esac
    flatpak=no; have flatpak && flatpak=yes
    return 0
}

detect_env() {
    wsl=no
    if [ -n "${WSL_DISTRO_NAME:-}" ] || { [ -r "$R/proc/version" ] && grep -qi microsoft "$R/proc/version" 2>/dev/null; }; then wsl=yes; fi
    # $container is the variable systemd-nspawn, podman and LXC set; read it
    # before this function reuses the name for its own answer.
    container_env="${container:-}"
    container=no
    if [ -e "$R/.dockerenv" ] || [ -e "$R/run/.containerenv" ] || [ -n "$container_env" ] ||
        { [ -r "$R/proc/1/cgroup" ] && grep -qE 'docker|lxc|kubepods|containerd' "$R/proc/1/cgroup" 2>/dev/null; }; then
        container=yes
    fi
    session=headless
    if [ -n "${WAYLAND_DISPLAY:-}" ] || [ "${XDG_SESSION_TYPE:-}" = wayland ]; then session=wayland
    elif [ -n "${DISPLAY:-}" ] || [ "${XDG_SESSION_TYPE:-}" = x11 ]; then session=x11
    fi
    desktop="${XDG_CURRENT_DESKTOP:-none}"
    if [ -d "$R/run/systemd/system" ]; then init=systemd
    elif have rc-service || have openrc; then init=openrc
    elif [ -d "$R/etc/runit" ] || have sv; then init=runit
    else init=unknown; fi
    musl=no
    case "$family" in alpine) musl=yes ;; esac
    for f in "$R"/lib/ld-musl-*.so.1; do [ -e "$f" ] && musl=yes; done
    uid="${GS_UID:-$(id -u)}"
    escalate=none
    if [ "$uid" = 0 ]; then escalate=root
    else
        for c in sudo doas pkexec; do
            if have "$c"; then escalate="$c"; break; fi
        done
    fi
    dl=none
    if have curl; then dl=curl; elif have wget; then dl=wget; fi
}

# Is the shared library $1 (e.g. libfuse.so.2) installed?
have_lib() {
    ld=""
    if have ldconfig; then ld=ldconfig
    elif [ -z "$R" ]; then ld=/sbin/ldconfig; [ -x "$ld" ] || ld=/usr/sbin/ldconfig; [ -x "$ld" ] || ld=""
    fi
    if [ -n "$ld" ] && "$ld" -p 2>/dev/null </dev/null | grep -q "$1"; then return 0; fi
    for f in "$R"/usr/lib*/"$1"* "$R"/lib*/"$1"* "$R"/usr/lib/*/"$1"* "$R"/lib/*/"$1"*; do
        [ -e "$f" ] && return 0
    done
    return 1
}

detect_fuse() {
    fuse2=missing; fuse3=missing; fuse_dev=no; fusermount=no
    have_lib libfuse.so.2 && fuse2=present
    have_lib libfuse3.so.3 && fuse3=present
    [ -e "$R/dev/fuse" ] && fuse_dev=yes
    { have fusermount || have fusermount3; } && fusermount=yes
    fuse_ok=no
    if [ "$fuse2" = present ] && [ "$fuse_dev" = yes ] && [ "$fusermount" = yes ]; then fuse_ok=yes; fi
}

# --------------------------------------------------------------- prerequisites

# The packages this system lacks, as logical names.
missing_logical() {
    m=""
    [ "$dl" = none ] && m="$m curl"
    if [ "$fuse_ok" = no ] && [ "$fuse_dev" = yes ] && [ "$immutable" = no ] && [ "$extract" != yes ]; then
        case "$family" in nixos|clear|alpine) ;; *) m="$m fuse2" ;; esac
    fi
    if [ "$no_desktop" = 0 ] && [ "$session" != headless ]; then
        have xdg-mime || m="$m xdg-utils"
        have update-desktop-database || m="$m desktop-file-utils"
    fi
    printf '%s' "${m# }"
}

# The root command that installs those, or nothing when we cannot know.
prereq_command() {
    pkgs=""
    for l in $1; do
        case "$pm:$l" in
            apt:fuse2)
                if have apt-cache && apt-cache show libfuse2t64 >/dev/null 2>&1 </dev/null; then pkgs="$pkgs libfuse2t64"; else pkgs="$pkgs libfuse2"; fi
                if have apt-cache && apt-cache show fuse3 >/dev/null 2>&1 </dev/null; then pkgs="$pkgs fuse3"; else pkgs="$pkgs fuse"; fi ;;
            apt:curl) pkgs="$pkgs curl ca-certificates" ;;
            dnf:fuse2|yum:fuse2) pkgs="$pkgs fuse-libs fuse" ;;
            zypper:fuse2) pkgs="$pkgs libfuse2 fuse" ;;
            pacman:fuse2) pkgs="$pkgs fuse2" ;;
            xbps:fuse2) pkgs="$pkgs fuse" ;;
            emerge:fuse2) pkgs="$pkgs sys-fs/fuse:0" ;;
            eopkg:fuse2) pkgs="$pkgs fuse2" ;;
            emerge:curl) pkgs="$pkgs net-misc/curl" ;;
            emerge:xdg-utils) pkgs="$pkgs x11-misc/xdg-utils" ;;
            emerge:desktop-file-utils) pkgs="$pkgs dev-util/desktop-file-utils" ;;
            *:fuse2) ;;
            *) pkgs="$pkgs $l" ;;
        esac
    done
    pkgs="${pkgs# }"
    [ -n "$pkgs" ] || return 0
    case "$pm" in
        apt) printf 'apt-get update && apt-get install -y %s' "$pkgs" ;;
        dnf) printf 'dnf install -y %s' "$pkgs" ;;
        yum) printf 'yum install -y %s' "$pkgs" ;;
        zypper) printf 'zypper --non-interactive install %s' "$pkgs" ;;
        pacman) printf 'pacman -S --needed --noconfirm %s' "$pkgs" ;;
        apk) printf 'apk add %s' "$pkgs" ;;
        xbps) printf 'xbps-install -Sy %s' "$pkgs" ;;
        emerge) printf 'emerge --ask=n --noreplace %s' "$pkgs" ;;
        eopkg) printf 'eopkg install -y %s' "$pkgs" ;;
        swupd) printf 'swupd bundle-add %s' "$pkgs" ;;
    esac
}

# What a person on this system should do instead, where installing a package is
# not the answer (immutable roots, NixOS).
prereq_note() {
    case "$immutable" in
        ostree) say "note: this is an image-based (rpm-ostree) system: nothing is layered. The AppImage runs by extract-and-run, so it does not need FUSE. (Layering fuse-libs is optional: rpm-ostree install fuse-libs, then reboot.)" ;;
        steamos) say "note: SteamOS has a read-only root: nothing is installed into it. The AppImage runs by extract-and-run from your home directory." ;;
        microos) say "note: this is a transactional system: nothing is layered. The AppImage runs by extract-and-run." ;;
        nixos) say "note: NixOS cannot run a generic AppImage as-is. Enable it once in configuration.nix: programs.appimage = { enable = true; binfmt = true; };  (or use nix-ld / appimage-run), then run this again." ;;
    esac
}

# --- privilege -------------------------------------------------------------

TTY="${GS_TTY:-/dev/tty}"

confirm() {
    [ "$yes" = 1 ] && return 0
    ( : <"$TTY" ) 2>/dev/null || return 1
    printf '%s [y/N] ' "$1" >"$TTY" 2>/dev/null || return 1
    ans=""
    read -r ans <"$TTY" || return 1
    case "$ans" in y|Y|yes|YES|Yes) return 0 ;; *) return 1 ;; esac
}

# Print the command the way a person would type it.
show_priv() {
    case "$escalate" in
        root|none) printf '%s' "$1"; return 0 ;;
        *) pre="$escalate " ;;
    esac
    case "$1" in
        *'&&'*) printf '%ssh -c '"'"'%s'"'"'' "$pre" "$1" ;;
        *) printf '%s%s' "$pre" "$1" ;;
    esac
}

run_priv() {
    case "$escalate" in
        root) sh -c "$1" </dev/null ;;
        sudo|doas|pkexec) "$escalate" sh -c "$1" </dev/null ;;
        *) return 1 ;;
    esac
}

# Offer the prerequisites. Never runs anything privileged without a yes.
prereqs() {
    prereq_note
    miss="$(missing_logical)"
    PREREQ_CMD=""
    [ -n "$miss" ] || { plan prereq none; return 0; }
    PREREQ_CMD="$(prereq_command "$miss")"
    if [ -z "$PREREQ_CMD" ]; then
        plan prereq none
        return 0
    fi
    plan prereq "$(show_priv "$PREREQ_CMD")"
    [ "$dry" = 1 ] && return 0
    say "Missing on this system: $miss"
    if [ "$escalate" = none ]; then
        say "No sudo, doas or pkexec found. As root, run:  $PREREQ_CMD"
        return 0
    fi
    if confirm "Run  $(show_priv "$PREREQ_CMD")  now?"; then
        if run_priv "$PREREQ_CMD"; then
            detect_fuse
            if [ "$dl" = none ]; then detect_env; set_protocols; fi
        else
            warn "that command failed; carrying on without it."
        fi
    else
        say "Not running it. To install the prerequisites yourself:"
        say "  $(show_priv "$PREREQ_CMD")"
    fi
}

# ------------------------------------------------------------------- download

proxy_env() {
    if [ -n "$proxy" ]; then
        http_proxy="$proxy"; https_proxy="$proxy"; HTTPS_PROXY="$proxy"; HTTP_PROXY="$proxy"; ALL_PROXY="$proxy"
        export http_proxy https_proxy HTTPS_PROXY HTTP_PROXY ALL_PROXY
    fi
}

netfail() {
    die "$1
  Could not reach the download. If you are offline, connect and retry. Behind a proxy, set HTTPS_PROXY (or pass --proxy URL).
  To install without the network: download the AppImage on another machine and run  GS_APPIMAGE=./GameSpice-App_<version>_$ASSET_ARCH.AppImage sh install.sh"
}

# Which URL schemes a download may use, and may be redirected to. HTTPS only,
# so a redirect cannot downgrade to plain HTTP. A mirror named in GS_BASE_URL
# may use its own scheme (file:// for tests and offline mirrors, http:// with
# a warning): the person running the script chose it.
set_protocols() {
    PROTO='=https'
    case "$BASE_URL" in
        file://*) PROTO='=https,file' ;;
        http://*) PROTO='=https,http'; warn "GS_BASE_URL is plain HTTP: the download and its checksums.txt can be altered in transit." ;;
        ''|https://*) ;;
        *) die "GS_BASE_URL must be an https://, http:// or file:// URL." ;;
    esac
    WGET_HTTPS=""
    if [ "$dl" = wget ] && [ "$PROTO" = '=https' ] && wget --help 2>&1 </dev/null | grep -q -- --https-only; then
        WGET_HTTPS=--https-only
    fi
}

fetch() {
    # $1 url, $2 destination ("-" = stdout)
    case "$dl" in
        curl) if [ "$2" = "-" ]; then curl -fsSL --proto "$PROTO" --proto-redir "$PROTO" --retry 3 --connect-timeout 20 "$1" </dev/null
              else curl -fsSL --proto "$PROTO" --proto-redir "$PROTO" --retry 3 --connect-timeout 20 -o "$2" "$1" </dev/null; fi ;;
        wget) if [ "$2" = "-" ]; then wget -qO- $WGET_HTTPS --tries=3 -T 20 "$1" </dev/null
              else wget -q $WGET_HTTPS --tries=3 -T 20 -O "$2" "$1" </dev/null; fi ;;
        *) die "neither curl nor wget is installed, and no prerequisite could be installed." ;;
    esac
}

resolve_version() {
    [ -z "$VERSION" ] || return 0
    [ -z "$BASE_URL" ] || die "GS_BASE_URL needs GS_VERSION too (the asset names contain it)."
    if [ "$channel" = beta ]; then
        # The newest release of any kind: the Atom feed lists prereleases and is
        # not rate-limited the way the REST API is (60 requests an hour per IP).
        VERSION="$(fetch "https://github.com/$REPO/releases.atom" - 2>/dev/null |
            sed -n 's|.*releases/tag/\([^"<]*\).*|\1|p' | head -n 1 || true)"
    elif [ "$dl" = curl ]; then
        resolved="$(curl -fsSLI --proto "$PROTO" --proto-redir "$PROTO" --retry 3 --connect-timeout 20 -o /dev/null -w '%{url_effective}' \
            "https://github.com/$REPO/releases/latest" </dev/null 2>/dev/null || true)"
        VERSION="${resolved##*/}"
    else
        resolved="$(wget -qS $WGET_HTTPS --max-redirect=10 -T 20 -O /dev/null "https://github.com/$REPO/releases/latest" 2>&1 </dev/null |
            sed -n 's/^ *Location: *//p' | tail -n 1 || true)"
        VERSION="${resolved##*/}"
    fi
    case "$VERSION" in
        ''|latest|releases|*[!A-Za-z0-9._+-]*) VERSION=""; netfail "could not work out the $channel release of $REPO." ;;
    esac
}

sha256_of() {
    if have sha256sum; then sha256sum "$1" | cut -d' ' -f1
    elif have shasum; then shasum -a 256 "$1" | cut -d' ' -f1
    else die "no sha256sum or shasum to verify with."; fi
}

# verify_file FILE ASSETNAME  (against checksums.txt in $tmp)
verify_checksum() {
    want="$(awk -v a="$2" '$2 == a || $2 == "*" a { print $1 }' "$tmp/checksums.txt" | head -n 1)"
    [ -n "$want" ] || return 2
    got="$(sha256_of "$1")"
    [ "$want" = "$got" ] || die "checksum mismatch for $2: expected $want, got $got. Nothing was installed."
    return 0
}

verify_signature() {
    # $1 file, $2 asset name
    [ "$verify_sig" = 1 ] || return 0
    if ! have minisign; then
        say "Signature not checked: minisign is not installed (the checksum was)."
        return 0
    fi
    if ! fetch "$BASE/$2.sig" "$tmp/asset.sig.b64" 2>/dev/null; then
        warn "release has no $2.sig; relying on the checksum."
        return 0
    fi
    have base64 || { warn "no base64 tool; signature not checked."; return 0; }
    base64 -d <"$tmp/asset.sig.b64" >"$tmp/asset.minisig" 2>/dev/null || die "the signature file is not valid."
    printf '%s' "${GS_PUBKEY:-$DEFAULT_PUBKEY}" | base64 -d >"$tmp/updater.pub" 2>/dev/null || die "GS_PUBKEY is not valid base64."
    if minisign -V -p "$tmp/updater.pub" -x "$tmp/asset.minisig" -m "$1" >/dev/null 2>&1 </dev/null; then
        say "Signature verified (minisign)."
    else
        die "the signature on $2 does not verify against the GameSpice release key. Nothing was installed."
    fi
}

download_appimage() {
    asset="GameSpice-App_${VERSION#v}_${ASSET_ARCH}.AppImage"
    BASE="${BASE_URL:-https://github.com/$REPO/releases/download/$VERSION}"
    say "Downloading the GameSpice App $VERSION ($ARCH)..."
    if ! fetch "$BASE/$asset" "$tmp/app.AppImage" 2>/dev/null; then
        # Releases from before the rename name the file TMC_...
        asset="TMC_${VERSION#v}_${ASSET_ARCH}.AppImage"
        fetch "$BASE/$asset" "$tmp/app.AppImage" 2>/dev/null ||
            netfail "no AppImage for $ARCH in release $VERSION (or the network is down)."
    fi
    have_sums=0
    if fetch "$BASE/checksums.txt" "$tmp/checksums.txt" 2>/dev/null; then have_sums=1; fi
    if [ "$verify" = 1 ]; then
        [ "$have_sums" = 1 ] || die "release $VERSION publishes no checksums.txt; re-run with --no-verify only if you understand that."
        rc=0; verify_checksum "$tmp/app.AppImage" "$asset" || rc=$?
        [ "$rc" -ne 2 ] || die "checksums.txt does not mention $asset."
        say "Checksum verified."
        verify_signature "$tmp/app.AppImage" "$asset"
    fi
}

# -------------------------------------------------------------------- layout

set_layout() {
    if [ "$system" = 1 ]; then
        PREFIX="${PREFIX:-/usr/local}"; DATA="$PREFIX/share"
    elif [ -n "$PREFIX" ]; then
        DATA="$PREFIX/share"
    else
        PREFIX="$HOME/.local"; DATA="${XDG_DATA_HOME:-$HOME/.local/share}"
    fi
    [ "$PREFIX" != "$HOME/.local" ] || DATA="${XDG_DATA_HOME:-$HOME/.local/share}"
    BIN="$PREFIX/bin/gamespice-app"
    LIBDIR="$PREFIX/lib/gamespice-app"
    DESKTOP="$DATA/applications/gamespice-app.desktop"
    ICONROOT="$DATA/icons"
    STATE="$DATA/gamespice-app-installer"
    MANIFEST="$STATE/manifest"
}

ICON_SIZES="16 22 24 32 48 64 96 128 192 256 384 512 1024"

# Is $1 an AppImage (type 1 or 2: "AI" and the type byte at offset 8)?
is_appimage() {
    [ -f "$1" ] || return 1
    m="$(dd if="$1" bs=1 skip=8 count=2 2>/dev/null </dev/null)"
    [ "$m" = AI ]
}

legacy_remove() {
    # What this script installed before the app was renamed: bin/tmc was the
    # AppImage itself, or a symlink to lib/tmc/AppRun in extract mode. Only
    # those are removed: `tmc` is also the name of the TMC command-line tool
    # (pip --user and others put it in ~/.local/bin), which the app's Console
    # runs, and an installer must never delete it.
    lt="$PREFIX/bin/tmc"
    if [ -L "$lt" ]; then
        case "$(readlink "$lt" 2>/dev/null </dev/null)" in
            "$PREFIX/lib/tmc/"*) rm -f "$lt" ;;
        esac
    elif is_appimage "$lt"; then
        rm -f "$lt"
    fi
    [ -e "$PREFIX/lib/tmc/AppRun" ] && rm -rf "$PREFIX/lib/tmc"
    if [ -f "$DATA/applications/tmc.desktop" ] && grep -qx 'Name=TMC' "$DATA/applications/tmc.desktop" 2>/dev/null; then
        rm -f "$DATA/applications/tmc.desktop"
    fi
    rm -f "$ICONROOT/hicolor/256x256/apps/tmc.png"
    return 0
}

refresh_desktop() {
    have update-desktop-database && update-desktop-database "$DATA/applications" >/dev/null 2>&1 </dev/null || true
    if have gtk-update-icon-cache; then gtk-update-icon-cache -qtfi "$ICONROOT/hicolor" >/dev/null 2>&1 </dev/null || true; fi
    if [ "$system" = 0 ] && have xdg-icon-resource; then xdg-icon-resource forceupdate >/dev/null 2>&1 </dev/null || true; fi
}

# The mimeapps.list files `xdg-mime default` may have written for this scope.
mimeapps_files() {
    [ "$system" = 1 ] || printf '%s\n' "${XDG_CONFIG_HOME:-$HOME/.config}/mimeapps.list"
    printf '%s\n' "$DATA/applications/mimeapps.list"
}

# Drop gamespice-app.desktop from the gs:/tmc: handler keys, and keys left
# empty, so the link handlers do not point at a deleted launcher.
unregister_scheme() {
    mimeapps_files | while IFS= read -r ma; do
        [ -f "$ma" ] && grep -q 'gamespice-app\.desktop' "$ma" 2>/dev/null || continue
        sed -e '/^x-scheme-handler\/gs=/s/\([=;]\)gamespice-app\.desktop;\{0,1\}/\1/g' \
            -e '/^x-scheme-handler\/tmc=/s/\([=;]\)gamespice-app\.desktop;\{0,1\}/\1/g' \
            -e '/^x-scheme-handler\/gs=$/d' -e '/^x-scheme-handler\/tmc=$/d' "$ma" >"$ma.gamespice-tmp" \
            && cat "$ma.gamespice-tmp" >"$ma"
        rm -f "$ma.gamespice-tmp"
    done
    return 0
}

do_uninstall() {
    if [ "$dry" = 1 ]; then
        plan remove "$BIN"
        plan remove "$LIBDIR"
        plan remove "$DESKTOP"
        plan remove "$ICONROOT/hicolor/*/{apps,mimetypes}/gamespice-app*,x-scheme-handler-gs*"
        plan remove "$STATE"
        mimeapps_files | while IFS= read -r ma; do plan unregister "$ma"; done
        say "Dry run: nothing was removed."
        return 0
    fi
    say "Removing the GameSpice App from $PREFIX ..."
    if [ -r "$MANIFEST" ]; then
        sed -n 's/^file=//p' "$MANIFEST" | while IFS= read -r f; do
            [ -n "$f" ] && rm -f "$f"
        done
    fi
    rm -f "$BIN" "$DESKTOP"
    rm -rf "$LIBDIR"
    for s in $ICON_SIZES; do
        rm -f "$ICONROOT/hicolor/${s}x${s}/apps/gamespice-app.png" "$ICONROOT/hicolor/${s}x${s}/mimetypes/x-scheme-handler-gs.png"
    done
    rm -f "$ICONROOT/hicolor/scalable/apps/gamespice-app.svg" "$ICONROOT/hicolor/scalable/mimetypes/x-scheme-handler-gs.svg"
    rm -f "$MANIFEST"; rmdir "$STATE" 2>/dev/null || true
    legacy_remove
    unregister_scheme
    refresh_desktop
    say "Removed. Your settings and library are untouched, in ${XDG_DATA_HOME:-$HOME/.local/share}/gamespice-app."
}

# ------------------------------------------------------------------ installing

FILES=""
TMPS=""

# put SRC DEST MODE: copy next to DEST, then rename into place.
put() {
    d="$(dirname "$2")"
    mkdir -p "$d" || return 1
    t="$d/.gamespice-installer.$$.$(basename "$2")"
    TMPS="$TMPS
$t"
    cp "$1" "$t" && chmod "$3" "$t" && mv -f "$t" "$2" || { rm -f "$t"; return 1; }
    FILES="$FILES
$2"
}

# Install the hicolor files found under $1/hicolor. Returns 0 if any landed.
install_icon_tree() {
    got=0
    for f in "$1"/hicolor/*/apps/gamespice-app.png "$1"/hicolor/*/apps/gamespice-app.svg \
             "$1"/hicolor/*/mimetypes/x-scheme-handler-gs.png; do
        [ -s "$f" ] || continue
        rel="${f#"$1"/}"
        sz="${rel#hicolor/}"; sz="${sz%%/*}"
        case "$sz" in scalable) ;; *[!0-9x]*|'') continue ;; esac
        put "$f" "$ICONROOT/$rel" 644 && got=1
    done
    [ "$got" = 1 ]
}

# Icons, best source first: the release's icon set, then what is inside the
# AppImage, then the AppImage's top-level icon.
install_icons() {
    ICON_SRC=none
    if [ -z "$LOCAL_IMAGE" ] && [ "$ICONS_FROM_RELEASE" = 1 ] && have tar; then
        itar="GameSpice-App-icons.tar.gz"
        if fetch "$BASE/$itar" "$tmp/$itar" 2>/dev/null &&
            { [ "$verify" = 0 ] || [ "$have_sums" = 0 ] || verify_checksum "$tmp/$itar" "$itar"; } &&
            ! tar -tzf "$tmp/$itar" 2>/dev/null | grep -qE '^/|(^|/)\.\.(/|$)'; then
            mkdir -p "$tmp/icons" && tar -xzf "$tmp/$itar" -C "$tmp/icons" 2>/dev/null &&
                install_icon_tree "$tmp/icons" && ICON_SRC=release
        fi
    fi
    if [ "$ICON_SRC" = none ]; then
        mkdir -p "$tmp/x"
        ( cd "$tmp/x" && "$tmp/app.AppImage" --appimage-extract >/dev/null 2>&1 </dev/null ) || true
        root="$tmp/x/squashfs-root"
        if [ -d "$root" ]; then
            mkdir -p "$tmp/fromapp"
            for f in "$root"/usr/share/icons/hicolor/*/apps/*.png; do
                [ -s "$f" ] || continue
                sz="${f#"$root"/usr/share/icons/hicolor/}"; sz="${sz%%/*}"
                case "$sz" in *[!0-9x]*|'') continue ;; esac
                mkdir -p "$tmp/fromapp/hicolor/$sz/apps" "$tmp/fromapp/hicolor/$sz/mimetypes"
                cp "$f" "$tmp/fromapp/hicolor/$sz/apps/gamespice-app.png"
                [ "$sz" = 256x256 ] || [ "$sz" = 48x48 ] &&
                    cp "$f" "$tmp/fromapp/hicolor/$sz/mimetypes/x-scheme-handler-gs.png"
            done
            if [ ! -d "$tmp/fromapp/hicolor" ]; then
                top="$(find "$root" -maxdepth 1 -name '*.png' 2>/dev/null | head -n 1)"
                if [ -n "$top" ]; then
                    mkdir -p "$tmp/fromapp/hicolor/256x256/apps" "$tmp/fromapp/hicolor/256x256/mimetypes"
                    cp "$top" "$tmp/fromapp/hicolor/256x256/apps/gamespice-app.png"
                    cp "$top" "$tmp/fromapp/hicolor/256x256/mimetypes/x-scheme-handler-gs.png"
                fi
            fi
            install_icon_tree "$tmp/fromapp" && ICON_SRC=appimage
        fi
    fi
    [ "$ICON_SRC" != none ] || warn "no icons could be installed; the launcher will show a generic icon."
}

write_desktop() {
    exec_q=$(desktop_quote "$BIN")
    tryexec_q=$(printf '%s' "$BIN" | sed 's/[\\]/&&/g')
    cat >"$tmp/gamespice-app.desktop" <<DESK
[Desktop Entry]
Type=Application
Name=GameSpice App
GenericName=Game Launcher
Comment=Play games, browse the store, and install mods, sandboxes and servers.
Exec=$exec_q %u
TryExec=$tryexec_q
Icon=gamespice-app
Terminal=false
Categories=Game;Utility;
MimeType=x-scheme-handler/gs;x-scheme-handler/tmc;
StartupWMClass=GameSpice App
DESK
    put "$tmp/gamespice-app.desktop" "$DESKTOP" 644
    have desktop-file-validate && { desktop-file-validate "$DESKTOP" >&2 </dev/null || warn "desktop-file-validate reported problems with $DESKTOP"; } || true
}

register_scheme() {
    for scheme in gs tmc; do
        have xdg-mime && xdg-mime default gamespice-app.desktop "x-scheme-handler/$scheme" >/dev/null 2>&1 </dev/null || true
    done
}

do_install() {
    if [ -r "$MANIFEST" ] && [ "$mode" != update ]; then
        say "Found an existing install: this run updates it."
    fi
    tmp="$(mktemp -d "${TMPDIR:-/tmp}/gamespice-install.XXXXXX")" || die "cannot create a temporary directory."
    trap 'cleanup' EXIT
    trap 'cleanup; exit 130' INT
    trap 'cleanup; exit 143' TERM
    trap 'cleanup; exit 129' HUP

    LOCAL_IMAGE=""
    ICONS_FROM_RELEASE=1
    have_sums=0; BASE=""
    if [ -n "$APPIMAGE_SRC" ]; then
        [ -f "$APPIMAGE_SRC" ] || die "$APPIMAGE_SRC does not exist."
        cp "$APPIMAGE_SRC" "$tmp/app.AppImage"
        LOCAL_IMAGE=1
        say "Installing $APPIMAGE_SRC (a local file: no checksum to verify)."
        [ -n "$VERSION" ] || VERSION="local"
    else
        resolve_version
        if [ "$force" = 0 ] && [ -r "$MANIFEST" ] && [ "$(sed -n 's/^version=//p' "$MANIFEST")" = "$VERSION" ] && [ -f "$BIN" ]; then
            say "The GameSpice App $VERSION is already installed. (The app updates itself in the background; --force reinstalls anyway.)"
            return 0
        fi
        download_appimage
    fi
    chmod +x "$tmp/app.AppImage"
    [ -s "$tmp/app.AppImage" ] || die "the AppImage is empty."

    # --- stage the binary: nothing below runs unless this succeeded ----------
    mkdir -p "$PREFIX/bin" || die "cannot create $PREFIX/bin."
    if [ "$run_mode" = direct ]; then
        put "$tmp/app.AppImage" "$BIN" 755 || die "could not write $BIN."
        # A previous extract-mode install kept the image here: it is stale now.
        rm -rf "$LIBDIR"
        upd_dir="$PREFIX/bin"
    else
        upd_dir="$LIBDIR"
        IMG="$LIBDIR/GameSpice-App.AppImage"
        put "$tmp/app.AppImage" "$IMG" 755 || die "could not write $IMG."
        case "$run_mode" in
            extract) env_line='APPIMAGE_EXTRACT_AND_RUN=1; export APPIMAGE_EXTRACT_AND_RUN'; runner="" ;;
            appimage-run) env_line=':'; runner="appimage-run " ;;
        esac
        {
            printf '#!/bin/sh\n# Written by the GameSpice installer (%s mode): runs the AppImage without FUSE.\n' "$run_mode"
            printf '%s\n' "$env_line"
            printf 'exec %s%s "$@"\n' "$runner" "$(shquote "$IMG")"
        } >"$tmp/wrapper"
        put "$tmp/wrapper" "$BIN" 755 || die "could not write $BIN."
    fi
    legacy_remove

    if [ "$no_desktop" = 0 ]; then
        install_icons
        write_desktop
        register_scheme
        refresh_desktop
    fi

    ver_line="$VERSION"
    {
        printf 'version=%s\nchannel=%s\nmode=%s\nicons=%s\nprefix=%s\n' "$ver_line" "$channel" "$run_mode" "${ICON_SRC:-none}" "$PREFIX"
        printf '%s\n' "$FILES" | sed -n 's/^\(.\)/file=\1/p'
    } >"$tmp/manifest"
    put "$tmp/manifest" "$MANIFEST" 644 || warn "could not write the install manifest; --uninstall will use the default paths."

    say ""
    say "The GameSpice App $VERSION is installed at $BIN ($run_mode mode)."
    [ "$ICON_SRC" = none ] || [ "$no_desktop" = 1 ] || say "Icons: $ICON_SRC set into $ICONROOT/hicolor."
    case ":$PATH:" in
        *":$PREFIX/bin:"*) say "Run it with: gamespice-app" ;;
        *) say "$PREFIX/bin is not on your PATH: run it with $BIN"
           say "  (or add it: echo 'export PATH=\"$PREFIX/bin:\$PATH\"' >> ~/.profile)" ;;
    esac
    if [ "$system" = 1 ] || [ ! -w "$upd_dir" ]; then
        warn "the app cannot update itself here: its updater needs to write to $upd_dir. A per-user install (no --system) updates automatically."
    else
        say "Updates: the app updates itself (Settings > App > Updates); re-running this installer also updates."
    fi
    say "Uninstall: re-run this installer with --uninstall."
}

cleanup() {
    [ -z "${tmp:-}" ] || rm -rf "$tmp"
    [ -z "$TMPS" ] || printf '%s\n' "$TMPS" | while IFS= read -r t; do [ -z "$t" ] || rm -f "$t"; done
    tmp=""; TMPS=""
}

# ----------------------------------------------------------------------- main

main() {
    mode=install; channel="${GS_CHANNEL:-stable}"; verify=1; verify_sig=1; extract=auto
    dry="${GS_DRY_RUN:-0}"; yes="${GS_YES:-0}"; no_desktop="${GS_NO_DESKTOP:-0}"
    force=0; system=0; proxy=""; show_version=0
    REPO="${GS_REPO:-gamespice-platform/app}"
    VERSION="${GS_VERSION:-}"; APPIMAGE_SRC="${GS_APPIMAGE:-}"; PREFIX="${GS_PREFIX:-}"
    BASE_URL="${GS_BASE_URL:-}"; R="${GS_ROOTFS:-}"
    tmp=""

    while [ $# -gt 0 ]; do
        case "$1" in
            --uninstall) mode=uninstall ;;
            --update) mode=update ;;
            --version)
                if [ $# -ge 2 ] && [ "${2#-}" = "$2" ]; then VERSION="$2"; shift; else show_version=1; fi ;;
            --version=*) VERSION="${1#--version=}" ;;
            --channel) [ $# -ge 2 ] || die "--channel needs stable or beta"; channel="$2"; shift ;;
            --channel=*) channel="${1#--channel=}" ;;
            --prefix) [ $# -ge 2 ] || die "--prefix needs a path"; PREFIX="$2"; shift ;;
            --prefix=*) PREFIX="${1#--prefix=}" ;;
            --system) system=1 ;;
            --dry-run|-n) dry=1 ;;
            -y|--yes) yes=1 ;;
            --no-desktop) no_desktop=1 ;;
            --no-verify) verify=0 ;;
            --no-verify-signature) verify_sig=0 ;;
            --extract) extract=yes ;;
            --force) force=1 ;;
            --proxy) [ $# -ge 2 ] || die "--proxy needs a URL"; proxy="$2"; shift ;;
            --proxy=*) proxy="${1#--proxy=}" ;;
            -h|--help) usage; return 0 ;;
            *) printf 'unknown argument: %s\n\n' "$1" >&2; usage >&2; return 2 ;;
        esac
        shift
    done
    case "$channel" in stable|beta) ;; *) die "--channel must be stable or beta, not '$channel'." ;; esac
    case "$VERSION" in *[!A-Za-z0-9._+-]*) die "'$VERSION' is not a release tag." ;; esac
    case "$REPO" in */*/*|/*|*/|*[!A-Za-z0-9._/-]*) die "GS_REPO must be owner/name, not '$REPO'." ;; */*) ;; *) die "GS_REPO must be owner/name, not '$REPO'." ;; esac
    case "$dry" in 1|yes|true) dry=1 ;; *) dry=0 ;; esac
    case "$yes" in 1|yes|true) yes=1 ;; *) yes=0 ;; esac
    case "$no_desktop" in 1|yes|true) no_desktop=1 ;; *) no_desktop=0 ;; esac
    [ -n "${HOME:-}" ] || die "HOME is not set."

    case "${GS_UNAME_S:-$(uname -s)}" in
        Linux) ;;
        Darwin) die "macOS builds are .dmg downloads, not this script: see https://gamespice.net/app" ;;
        *) die "$(uname -s) is not supported by this installer." ;;
    esac

    detect_arch; detect_distro; detect_immutable; detect_pm; detect_env; detect_fuse
    proxy_env; set_protocols
    uid_root=0; [ "$uid" = 0 ] && uid_root=1
    # Root with no explicit prefix means a system install.
    if [ "$uid_root" = 1 ] && [ -z "$PREFIX" ]; then system=1; fi
    set_layout

    if [ "$show_version" = 1 ]; then
        say "installer: $SCRIPT_VERSION"
        if [ -r "$MANIFEST" ]; then say "installed: $(sed -n 's/^version=//p' "$MANIFEST") ($(sed -n 's/^channel=//p' "$MANIFEST"))"
        else say "installed: nothing recorded under $PREFIX"; fi
        return 0
    fi

    # How the AppImage will be started.
    run_mode=direct
    if [ "$extract" = yes ] || [ "$fuse_ok" = no ]; then run_mode=extract; fi
    if [ "$family" = nixos ] && [ ! -e "$R/proc/sys/fs/binfmt_misc/appimage-type2" ]; then
        if have appimage-run; then run_mode=appimage-run; else run_mode=extract; fi
    fi
    if [ "$family" = nixos ] && [ -e "$R/proc/sys/fs/binfmt_misc/appimage-type2" ]; then run_mode=direct; fi

    plan arch "$ARCH"
    plan distro "${ID:-unknown}"
    plan family "$family"
    plan name "$NAME"
    plan pm "$pm"
    plan pm_all "${pm_all:-none}"
    plan flatpak "$flatpak"
    plan immutable "$immutable"
    plan wsl "$wsl"
    plan container "$container"
    plan session "$session"
    plan desktop "$desktop"
    plan init "$init"
    plan fuse2 "$fuse2"
    plan fuse3 "$fuse3"
    plan fuse_dev "$fuse_dev"
    plan fusermount "$fusermount"
    plan escalate "$escalate"
    plan downloader "$dl"
    plan channel "$channel"
    plan prefix "$PREFIX"
    plan scope "$([ "$system" = 1 ] && echo system || echo user)"
    plan mode "$mode"

    # Hard stops come before anything is changed.
    if [ "$musl" = yes ] && [ "$force" = 0 ]; then
        plan warn musl-unsupported
        say "This system uses musl libc. The GameSpice AppImage is built for glibc and will not run here (gcompat is not enough for its web engine). Use a glibc distribution, a distrobox/glibc container, or --force to install anyway."
        [ "$dry" = 1 ] || exit 1
    fi
    if [ "$system" = 1 ] && [ "$uid_root" = 0 ] && { [ "$mode" != uninstall ] || [ "$dry" = 0 ]; }; then
        say "--system writes to $PREFIX and needs root. Re-run with:"
        say "  curl -fsSL https://gamespice.net/install.sh | $(case "$escalate" in none) printf 'su -c' ;; *) printf '%s' "$escalate" ;; esac) sh -s -- --system"
        [ "$dry" = 1 ] || exit 1
    fi
    if [ "$system" = 1 ] && [ "$immutable" != no ]; then
        plan warn immutable-system-install
        say "This root file system is read-only or image-based ($immutable); --system would be lost or refused. Install per-user instead (leave out --system): it needs no root and the app can update itself."
        [ "$dry" = 1 ] || exit 1
    fi
    if [ "$wsl" = yes ] && [ "$session" = headless ]; then
        plan warn wsl-no-gui
        say "WSL without a display: the app needs WSLg (Windows 11 / current WSL) to show a window. Otherwise use the Windows installer."
    fi
    if [ "$container" = yes ]; then
        plan warn container
        say "Running in a container: the window needs a display passed in, and FUSE is usually unavailable (extract-and-run is used)."
    fi

    plan run_mode "$run_mode"
    if [ -n "$VERSION" ]; then plan version "$VERSION"; else plan version "latest-$channel"; fi

    if [ "$mode" = uninstall ]; then
        do_uninstall
        return 0
    fi

    prereqs
    # A refused or failed prerequisite install leaves FUSE missing: fall back.
    if [ "$run_mode" = direct ] && [ "$fuse_ok" = no ]; then run_mode=extract; fi
    # ...and one that installed FUSE lets the AppImage run directly after all.
    if [ "$run_mode" = extract ] && [ "$extract" != yes ] && [ "$fuse_ok" = yes ] && [ "$family" != nixos ]; then run_mode=direct; fi
    if [ "$dry" = 1 ]; then
        plan icons "$([ "$no_desktop" = 1 ] && echo none || echo "release-tarball>appimage")"
        plan files "$BIN $DESKTOP $ICONROOT/hicolor/*/apps/gamespice-app.png"
        say "Dry run: nothing was downloaded or changed."
        return 0
    fi
    [ "$extract" = yes ] || [ "$run_mode" != extract ] || say "No usable FUSE here, so the AppImage will run by extract-and-run: it works the same and still updates itself."
    do_install
}

main "$@"
