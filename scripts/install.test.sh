#!/bin/sh
# Test harness for scripts/install.sh. No root, no network, no docker needed:
# each distro profile is a fake root (GS_ROOTFS) with an os-release file and a
# PATH of stub package managers, and the assertions read the installer's
# `--dry-run` plan lines. A second part installs a fake release (file:// via
# GS_BASE_URL) into a temp prefix and checks the files, icons, checksums,
# idempotency and --uninstall.
#
#   sh scripts/install.test.sh            # uses sh
#   dash scripts/install.test.sh; bash scripts/install.test.sh
#
# Set INSTALLER_SHELL to run the installer under another shell (default: the
# shell running this script).
here="$(cd "$(dirname "$0")" && pwd)"
INSTALLER="$here/install.sh"
SH="${INSTALLER_SHELL:-sh}"
# the stub PATH hides shells, so pin the installer shell to an absolute path
case "$SH" in
    "busybox sh") SH="$(command -v busybox) sh" ;;
    /*) ;;
    *) SH="$(command -v "$SH")" ;;
esac
W="$(mktemp -d "${TMPDIR:-/tmp}/gs-install-test.XXXXXX")"
trap 'rm -rf "$W"' EXIT
pass=0; fail=0

ok()  { pass=$((pass + 1)); }
bad() { fail=$((fail + 1)); printf 'FAIL: %s\n' "$*"; }

# Basic tools the installer needs, linked into one dir so the stub PATH hides
# everything else (a real apt-get on the host must not leak into a Fedora profile).
BASE="$W/base"; mkdir -p "$BASE"
for t in sh sed grep tr head tail cat uname id awk dirname basename cp mv chmod mkdir rm mktemp cut tar gzip printf env sleep find sha256sum rmdir ls date true false dd readlink ln; do
    p="$(command -v $t 2>/dev/null)" && ln -sf "$p" "$BASE/$t"
done

stub() { # stub DIR NAME...
    d="$1"; shift
    mkdir -p "$d"
    for sn in "$@"; do printf '#!/bin/sh\nexit 0\n' >"$d/$sn"; chmod +x "$d/$sn"; done
}

# plan_of PROFILE-DIR [installer args]: the plan output
run_plan() {
    pd="$1"; shift
    PATH="$pd/bin:$BASE" GS_ROOTFS="$pd/root" GS_OS_RELEASE="$pd/root/etc/os-release" \
        GS_UNAME_S=Linux GS_UNAME_M="${ARCHM:-x86_64}" GS_UID="${UIDV:-1000}" HOME="$pd/home" \
        XDG_DATA_HOME="" GS_TTY=/nonexistent \
        $SH "$INSTALLER" --dry-run "$@" 2>&1 </dev/null
}

expect() { # expect "output" key=value label
    case "
$1
" in
        *"
plan: $2
"*) ok ;;
        *) bad "$3: wanted 'plan: $2'" ;;
    esac
}
expect_text() {
    case "$1" in *"$2"*) ok ;; *) bad "$3: wanted text '$2'" ;; esac
}

# profile NAME "os-release body" "stubs" [extra root files...]
n=0
profile() {
    n=$((n + 1))
    body="$2"; stubs="$3"
    pd="$W/p$n"; mkdir -p "$pd/root/etc" "$pd/root/dev" "$pd/home"; : >"$pd/root/dev/fuse"
    printf '%s\n' "$body" >"$pd/root/etc/os-release"
    # shellcheck disable=SC2086
    stub "$pd/bin" $stubs
    P="$pd"
}
fuse_present() { mkdir -p "$P/root/usr/lib"; : >"$P/root/usr/lib/libfuse.so.2.9.9"; mkdir -p "$P/root/dev"; : >"$P/root/dev/fuse"; stub "$P/bin" fusermount; }

echo "== distro profiles (dry-run plan)"

profile ubuntu 'ID=ubuntu
ID_LIKE=debian
PRETTY_NAME="Ubuntu 24.04"' "apt-get apt-cache curl sudo"
fuse_present; : >"$P/root/dev/fuse"; rm -f "$P/root/usr/lib/libfuse.so.2.9.9"
o="$(run_plan "$P")"
expect "$o" family=debian ubuntu; expect "$o" pm=apt ubuntu; expect "$o" fuse2=missing ubuntu
expect_text "$o" "apt-get install -y" ubuntu; expect "$o" run_mode=extract ubuntu
# prereq command is shown with sudo, never run in dry-run
expect_text "$o" "plan: prereq=sudo sh -c" ubuntu-sudo

profile mint 'ID=linuxmint
ID_LIKE="ubuntu debian"' "apt-get curl"
o="$(run_plan "$P")"; expect "$o" family=debian mint; expect "$o" pm=apt mint

profile debian-root 'ID=debian' "apt-get curl"
o="$(UIDV=0 run_plan "$P")"; expect "$o" escalate=root debian-root; expect "$o" scope=system debian-root
expect_text "$o" "plan: prereq=apt-get update" debian-root-noprefix

profile fedora 'ID=fedora
PRETTY_NAME="Fedora 41"' "dnf curl sudo"
o="$(run_plan "$P")"; expect "$o" family=fedora fedora; expect "$o" pm=dnf fedora; expect_text "$o" "dnf install -y fuse-libs" fedora

profile rocky 'ID=rocky
ID_LIKE="rhel centos fedora"' "yum curl doas"
o="$(run_plan "$P")"; expect "$o" family=fedora rocky; expect "$o" pm=yum rocky; expect "$o" escalate=doas rocky

profile opensuse 'ID=opensuse-tumbleweed
ID_LIKE="opensuse suse"' "zypper curl sudo"
o="$(run_plan "$P")"; expect "$o" family=suse opensuse; expect "$o" pm=zypper opensuse; expect_text "$o" "zypper --non-interactive install libfuse2" opensuse

profile arch 'ID=arch' "pacman curl sudo"
o="$(run_plan "$P")"; expect "$o" pm=pacman arch; expect_text "$o" "pacman -S --needed --noconfirm fuse2" arch

profile manjaro 'ID=manjaro
ID_LIKE=arch' "pacman curl pkexec"
o="$(run_plan "$P")"; expect "$o" family=arch manjaro; expect "$o" escalate=pkexec manjaro

profile alpine 'ID=alpine' "apk curl doas"
o="$(run_plan "$P")"; expect "$o" pm=apk alpine; expect "$o" warn=musl-unsupported alpine

profile gentoo 'ID=gentoo' "emerge curl sudo"
o="$(run_plan "$P")"; expect "$o" pm=emerge gentoo; expect_text "$o" "emerge --ask=n --noreplace sys-fs/fuse:0" gentoo

profile void 'ID=void' "xbps-install curl sudo"
o="$(run_plan "$P")"; expect "$o" pm=xbps void; expect_text "$o" "xbps-install -Sy fuse" void

profile nixos 'ID=nixos' "nix-env curl"
o="$(run_plan "$P")"; expect "$o" family=nixos nixos; expect "$o" immutable=nixos nixos
expect "$o" run_mode=extract nixos
profile nixos-appimage-run 'ID=nixos' "nix-env curl appimage-run"
o="$(run_plan "$P")"; expect "$o" run_mode=appimage-run nixos-appimage-run

profile silverblue 'ID=fedora
VARIANT_ID=silverblue' "rpm-ostree dnf curl sudo"
mkdir -p "$P/root/run"; : >"$P/root/run/ostree-booted"
o="$(run_plan "$P")"; expect "$o" immutable=ostree silverblue; expect "$o" pm=rpm-ostree silverblue
expect "$o" run_mode=extract silverblue; expect "$o" prereq=none silverblue
o="$(run_plan "$P" --system)"; expect "$o" warn=immutable-system-install silverblue-system

profile bazzite 'ID=bazzite
ID_LIKE="fedora"
VARIANT_ID=kinoite' "rpm-ostree curl"
o="$(run_plan "$P")"; expect "$o" immutable=ostree bazzite

profile steamos 'ID=steamos
ID_LIKE=arch' "pacman curl"
o="$(run_plan "$P")"; expect "$o" immutable=steamos steamdeck; expect_text "$o" "read-only root" steamdeck

profile aarch64 'ID=debian' "apt-get curl"
o="$(ARCHM=aarch64 run_plan "$P")"; expect "$o" arch=aarch64 aarch64
o="$(ARCHM=riscv64 run_plan "$P")"; expect_text "$o" "no AppImage is published for riscv64" riscv64

profile wsl 'ID=ubuntu
ID_LIKE=debian' "apt-get curl"
mkdir -p "$P/root/proc"; printf 'Linux version 5.15 microsoft-standard-WSL2\n' >"$P/root/proc/version"
o="$(run_plan "$P")"; expect "$o" wsl=yes wsl; expect "$o" warn=wsl-no-gui wsl

profile wayland 'ID=fedora' "dnf curl xdg-mime update-desktop-database"
o="$(WAYLAND_DISPLAY=wayland-0 run_plan "$P")"; expect "$o" session=wayland wayland
profile x11 'ID=fedora' "dnf curl"
o="$(DISPLAY=:0 run_plan "$P")"; expect "$o" session=x11 x11
expect_text "$o" "xdg-utils" x11-wants-xdg-utils

profile nocurl 'ID=debian' "apt-get sudo"
o="$(run_plan "$P")"; expect "$o" downloader=none nocurl; expect_text "$o" "curl ca-certificates" nocurl

profile noescalate 'ID=debian' "apt-get curl"
o="$(run_plan "$P")"; expect "$o" escalate=none noescalate

profile nocontainer 'ID=debian' "apt-get curl"
o="$(container='' run_plan "$P")"; expect "$o" container=no no-container-env
o="$(container=podman run_plan "$P")"; expect "$o" container=yes container-env

echo "== no silent privilege"
# Without a tty and without --yes the installer must not run the stub sudo.
profile priv 'ID=debian' "apt-get apt-cache curl"
printf '#!/bin/sh\necho RAN >"%s/sudo-ran"\nexit 0\n' "$P" >"$P/bin/sudo"; chmod +x "$P/bin/sudo"
mkdir -p "$P/root/dev"; : >"$P/root/dev/fuse"
GS_APPIMAGE="$W/none" PATH="$P/bin:$BASE" GS_ROOTFS="$P/root" GS_OS_RELEASE="$P/root/etc/os-release" GS_UID=1000 GS_TTY=/nonexistent HOME="$P/home" \
    $SH "$INSTALLER" >"$P/out" 2>&1 </dev/null || true
[ ! -e "$P/sudo-ran" ] && ok || bad "sudo ran without confirmation"
expect_text "$(cat "$P/out")" "To install the prerequisites yourself" print-command

echo "== argument handling"
o="$($SH "$INSTALLER" --bogus 2>&1)"; expect_text "$o" "unknown argument" bogus
o="$($SH "$INSTALLER" --help 2>&1)"; expect_text "$o" "--uninstall" help
o="$($SH "$INSTALLER" --channel nightly --dry-run 2>&1)"; expect_text "$o" "stable or beta" bad-channel
o="$(GS_BASE_URL=ftp://example.org/x $SH "$INSTALLER" --dry-run 2>&1)"; expect_text "$o" "must be an https://" bad-base-scheme
o="$(GS_VERSION='v1;rm' $SH "$INSTALLER" --dry-run 2>&1)"; expect_text "$o" "is not a release tag" bad-version
o="$(GS_REPO='a/b/../c' $SH "$INSTALLER" --dry-run 2>&1)"; expect_text "$o" "must be owner/name" bad-repo

echo "== real install of a fake release (file://)"
REL="$W/rel"; mkdir -p "$REL"
ASSET="GameSpice-App_9.9.9_amd64.AppImage"
mk_appimage() { # mk_appimage FILE with-icons
    cat >"$1" <<'IMG'
#!/bin/sh
if [ "$1" = "--appimage-extract" ]; then
    mkdir -p squashfs-root/usr/share/icons/hicolor/256x256/apps squashfs-root/usr/share/icons/hicolor/48x48/apps
    printf 'PNG-from-appimage-256' >squashfs-root/usr/share/icons/hicolor/256x256/apps/app.png
    printf 'PNG-from-appimage-48' >squashfs-root/usr/share/icons/hicolor/48x48/apps/app.png
    exit 0
fi
echo "fake gamespice app $*"
IMG
    chmod +x "$1"
}
mk_appimage "$REL/$ASSET"
# icon tarball: take the repo's real set when present, else a fake one
ICONSRC="$here/../packaging/linux/icons"
if [ -d "$ICONSRC/hicolor" ]; then tar -C "$ICONSRC" -czf "$REL/GameSpice-App-icons.tar.gz" hicolor
else mkdir -p "$W/ic/hicolor/256x256/apps"; printf x >"$W/ic/hicolor/256x256/apps/gamespice-app.png"; tar -C "$W/ic" -czf "$REL/GameSpice-App-icons.tar.gz" hicolor; fi
mksums() { ( cd "$REL" && sha256sum "$ASSET" GameSpice-App-icons.tar.gz >checksums.txt ); }
mksums

# a real-ish environment: real curl on PATH, no FUSE (so extract mode)
mkdir -p "$W/rb"; ln -sf "$(command -v curl)" "$W/rb/curl"
inst() { # inst PREFIX args...
    pre="$1"; shift
    PATH="$W/rb:$BASE" GS_ROOTFS="$W/rootfs" GS_OS_RELEASE="$W/rootfs/etc/os-release" GS_UNAME_S=Linux GS_UNAME_M=x86_64 GS_UID=1000 \
        GS_TTY=/nonexistent HOME="$W/home" XDG_DATA_HOME="$W/home/.local/share" \
        GS_BASE_URL="file://$REL" GS_VERSION=v9.9.9 $SH "$INSTALLER" --prefix "$pre" --no-verify-signature "$@" 2>&1 </dev/null
}
mkdir -p "$W/rootfs/etc" "$W/home"; printf 'ID=debian\n' >"$W/rootfs/etc/os-release"

PRE="$W/home/.local"
o="$(inst "$PRE")"
[ -x "$PRE/bin/gamespice-app" ] && ok || bad "binary not installed: $o"
expect_text "$o" "Checksum verified" checksum
expect_text "$o" "extract mode" extract-mode
grep -q APPIMAGE_EXTRACT_AND_RUN "$PRE/bin/gamespice-app" && ok || bad "wrapper lacks extract-and-run"
[ -x "$PRE/lib/gamespice-app/GameSpice-App.AppImage" ] && ok || bad "AppImage file missing under lib"
D="$W/home/.local/share"
grep -q '^Icon=gamespice-app$' "$D/applications/gamespice-app.desktop" && ok || bad "desktop Icon= wrong"
grep -q "^Exec=\"$PRE/bin/gamespice-app\" %u" "$D/applications/gamespice-app.desktop" && ok || bad "desktop Exec= wrong"
grep -q 'x-scheme-handler/gs' "$D/applications/gamespice-app.desktop" && ok || bad "desktop lacks gs:// handler"
cnt="$(find "$D/icons/hicolor" -name gamespice-app.png | wc -l)"
if [ -d "$ICONSRC/hicolor" ]; then
    [ "$cnt" -ge 7 ] && ok || bad "expected >=7 app icon sizes, found $cnt"
    for s in 16 32 48 64 128 256 512; do [ -s "$D/icons/hicolor/${s}x${s}/apps/gamespice-app.png" ] && ok || bad "icon ${s} missing"; done
    cmp -s "$ICONSRC/hicolor/256x256/apps/gamespice-app.png" "$D/icons/hicolor/256x256/apps/gamespice-app.png" && ok || bad "installed 256 icon differs from the repo GameSpice icon"
    [ -s "$D/icons/hicolor/256x256/mimetypes/x-scheme-handler-gs.png" ] && ok || bad "gs mime icon missing"
fi
expect_text "$o" "Icons: release set" icon-source
grep -q '^version=v9.9.9' "$D/gamespice-app-installer/manifest" && ok || bad "manifest version"

# idempotent re-run
o="$(inst "$PRE")"; expect_text "$o" "already installed" idempotent
# --force reinstalls
o="$(inst "$PRE" --force)"; expect_text "$o" "is installed at" force

# Tampered checksum must refuse and leave the install untouched
sha_before="$(sha256sum "$PRE/bin/gamespice-app" | cut -d' ' -f1)"
printf 'tampered\n' >>"$REL/$ASSET"
o="$(inst "$PRE" --force)"
expect_text "$o" "checksum mismatch" tampered
[ "$(sha256sum "$PRE/bin/gamespice-app" | cut -d' ' -f1)" = "$sha_before" ] && ok || bad "tampered download modified the install"
mk_appimage "$REL/$ASSET"; mksums

# Fallback: no icon tarball -> icons come from the AppImage
rm -f "$REL/GameSpice-App-icons.tar.gz"; ( cd "$REL" && sha256sum "$ASSET" >checksums.txt )
o="$(inst "$PRE" --force)"
expect_text "$o" "Icons: appimage" icon-fallback
[ "$(cat "$D/icons/hicolor/256x256/apps/gamespice-app.png")" = "PNG-from-appimage-256" ] && ok || bad "AppImage fallback icon not installed"

# The tmc CLI (same name as the pre-rename app) must survive install and uninstall.
printf '#!/bin/sh\necho tmc-cli\n' >"$PRE/bin/tmc"; chmod +x "$PRE/bin/tmc"
o="$(inst "$PRE" --force)"
[ -x "$PRE/bin/tmc" ] && ok || bad "install deleted a foreign ~/.local/bin/tmc"
# xdg-mime wrote the handlers (simulated: no xdg-mime in the stub PATH)
mkdir -p "$W/home/.config"
printf '[Default Applications]\nx-scheme-handler/gs=gamespice-app.desktop\nx-scheme-handler/tmc=other.desktop;gamespice-app.desktop;\ntext/html=firefox.desktop\n' >"$W/home/.config/mimeapps.list"

# --uninstall --dry-run changes nothing
o="$(inst "$PRE" --uninstall --dry-run)"
expect_text "$o" "nothing was removed" uninstall-dry
[ -x "$PRE/bin/gamespice-app" ] && [ -e "$D/applications/gamespice-app.desktop" ] && ok || bad "--uninstall --dry-run removed files"

# Uninstall removes binary, desktop entry, all icons, manifest
o="$(inst "$PRE" --uninstall)"
[ -x "$PRE/bin/tmc" ] && ok || bad "uninstall deleted a foreign ~/.local/bin/tmc"
grep -q 'gamespice-app' "$W/home/.config/mimeapps.list" && bad "uninstall left gs:// handlers in mimeapps.list" || ok
grep -qx 'x-scheme-handler/tmc=other.desktop;' "$W/home/.config/mimeapps.list" && grep -qx 'text/html=firefox.desktop' "$W/home/.config/mimeapps.list" && ok || bad "uninstall damaged other mimeapps.list entries"
rm -f "$PRE/bin/tmc"
[ ! -e "$PRE/bin/gamespice-app" ] && ok || bad "binary remains after uninstall"
[ ! -e "$D/applications/gamespice-app.desktop" ] && ok || bad "desktop entry remains"
left="$(find "$D/icons" "$D/gamespice-app-installer" "$PRE/lib" \( -name 'gamespice-app*' -o -name 'x-scheme-handler-gs*' -o -name manifest \) 2>/dev/null | wc -l)"
[ "$left" -eq 0 ] && ok || bad "$left installer files remain after uninstall"

# --no-desktop installs no launcher or icons
o="$(inst "$PRE" --force --no-desktop)"
[ -x "$PRE/bin/gamespice-app" ] && [ ! -e "$D/applications/gamespice-app.desktop" ] && ok || bad "--no-desktop wrote desktop files"
inst "$PRE" --uninstall >/dev/null

# Offline: unreachable base -> clear message, nothing installed
o="$(PATH="$W/rb:$BASE" GS_ROOTFS="$W/rootfs" GS_OS_RELEASE="$W/rootfs/etc/os-release" GS_UID=1000 GS_TTY=/nonexistent HOME="$W/home" \
    GS_BASE_URL="file://$W/nothing" GS_VERSION=v9.9.9 $SH "$INSTALLER" --prefix "$PRE" 2>&1 </dev/null)"
expect_text "$o" "Could not reach the download" offline
[ ! -e "$PRE/bin/gamespice-app" ] && ok || bad "offline run left a binary"

# local AppImage
mk_appimage "$W/local.AppImage"
o="$(GS_APPIMAGE="$W/local.AppImage" inst "$PRE" --no-desktop)"
[ -x "$PRE/bin/gamespice-app" ] && ok || bad "GS_APPIMAGE install failed: $o"
inst "$PRE" --uninstall >/dev/null

# A pre-rename AppImage at bin/tmc is removed (AppImage magic at offset 8).
printf '\177ELF\002\001\001\000AI\002rest' >"$PRE/bin/tmc"
inst "$PRE" --force >/dev/null
[ ! -e "$PRE/bin/tmc" ] && ok || bad "old tmc AppImage not removed"
inst "$PRE" --uninstall >/dev/null

# A prefix with spaces, quotes and $ still yields a working wrapper and Exec=.
SP="$W/my apps/it's \$HOME"
o="$(inst "$SP" --force)"
out="$(PATH="$BASE" "$SP/bin/gamespice-app" --probe 2>&1)"
expect_text "$out" "fake gamespice app --probe" spaced-prefix-wrapper
ex="$(sed -n 's/^Exec=//p' "$SP/share/applications/gamespice-app.desktop")"
[ "$ex" = "\"$W/my apps/it's \\\\\$HOME/bin/gamespice-app\" %u" ] && ok || bad "spaced prefix Exec= not quoted: $ex"
inst "$SP" --uninstall >/dev/null
[ ! -e "$SP/bin/gamespice-app" ] && ok || bad "spaced prefix uninstall left the binary"

printf '\n%s passed, %s failed\n' "$pass" "$fail"
[ "$fail" -eq 0 ]
