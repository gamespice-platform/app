#!/usr/bin/env bash
#
# Build the `tmc` command-line tool into ONE self-contained executable and put
# it where Tauri's `externalBin` expects it:
#
#   src-tauri/binaries/tmc-<target-triple>[.exe]
#
#   scripts/build-tmc-cli.sh [--src DIR] [--out DIR] [--triple T] [--test]
#
# The CLI is Python 3.11+ on the standard library alone, so a PyInstaller
# one-file build is the whole story: no interpreter on the user's machine, no
# pip step. PyInstaller cannot cross-compile, so this builds for the host it
# runs on; CI runs it once per platform (see .github/workflows/release.yml).
#
# WHICH SOURCE: `--src DIR` (or TMC_CLI_SRC) builds a checkout as it is, for
# trying a CLI change here. Without either, the ref pinned in
# packaging/tmc-cli.json is cloned from the repo named there, so a release is
# reproducible and bumping the CLI is a one-line, reviewable change. Clone URL
# override: TMC_CLI_REPO_URL; TMC_CLI_TOKEN adds a GitHub token (CI).
#
# --test builds, then runs `cargo test` for the one test that starts the result
# through the Console's own lookup and spawn code (TMC_BUNDLED_BIN).
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/.." && pwd)"

src="${TMC_CLI_SRC:-}"
out="$root/src-tauri/binaries"
triple=""
run_test=0

while [ $# -gt 0 ]; do
    case "$1" in
        --src) src="${2:?--src needs a directory}"; shift ;;
        --out) out="${2:?--out needs a directory}"; shift ;;
        --triple) triple="${2:?--triple needs a value}"; shift ;;
        --test) run_test=1 ;;
        -h|--help) sed -n '2,22p' "$0"; exit 0 ;;
        *) echo "unknown argument: $1" >&2; exit 2 ;;
    esac
    shift
done

# PyInstaller is pinned for the same reason the Rust toolchain is: a new
# release changing how a one-file build behaves should fail a pull request.
PYINSTALLER_VERSION="6.22.3"

host_triple() {
    if command -v rustc >/dev/null 2>&1; then
        rustc -vV | sed -n 's/^host: //p'
        return
    fi
    case "$(uname -s)-$(uname -m)" in
        Linux-x86_64)  echo x86_64-unknown-linux-gnu ;;
        Linux-aarch64) echo aarch64-unknown-linux-gnu ;;
        Darwin-arm64)  echo aarch64-apple-darwin ;;
        Darwin-x86_64) echo x86_64-apple-darwin ;;
        MINGW*|MSYS*|CYGWIN*) echo x86_64-pc-windows-msvc ;;
        *) echo "cannot work out the target triple; pass --triple" >&2; exit 1 ;;
    esac
}

[ -n "$triple" ] || triple="$(host_triple)"

ext=""
case "$triple" in *windows*) ext=".exe" ;; esac

python="${PYTHON:-}"
if [ -z "$python" ]; then
    for c in python3 python; do
        command -v "$c" >/dev/null 2>&1 && { python="$c"; break; }
    done
fi
[ -n "$python" ] || { echo "python 3.11+ is needed to BUILD the CLI (it is not needed to run it)." >&2; exit 1; }
"$python" -c 'import sys; sys.exit(sys.version_info < (3, 11))' ||
    { echo "$python is older than 3.11." >&2; exit 1; }

work="$root/src-tauri/target/tmc-cli-build"
mkdir -p "$work" "$out"

if [ -z "$src" ]; then
    # Relative to $root: on Windows Git Bash $root is a POSIX path (/d/a/...)
    # that a native node.exe cannot resolve.
    repo="$(cd "$root" && node -p "require('./packaging/tmc-cli.json').repo")"
    ref="$(cd "$root" && node -p "require('./packaging/tmc-cli.json').ref")"
    url="${TMC_CLI_REPO_URL:-https://github.com/$repo.git}"
    # CI: tmc-cli may be private. The token rides in the URL only for this one
    # fetch; nothing below prints it.
    [ -z "${TMC_CLI_TOKEN:-}" ] || url="https://x-access-token:$TMC_CLI_TOKEN@github.com/$repo.git"
    src="$work/src"
    rm -rf "$src"
    git init -q "$src"
    git -C "$src" fetch -q --depth 1 "$url" "$ref"
    git -C "$src" checkout -q FETCH_HEAD
    echo "tmc-cli $repo@$(git -C "$src" rev-parse --short HEAD)"
else
    echo "tmc-cli from $src (not the pinned ref)"
fi

[ -f "$src/src/main.py" ] || { echo "$src/src/main.py not found: not a tmc-cli checkout." >&2; exit 1; }

venv="$work/venv"
[ -d "$venv" ] || "$python" -m venv "$venv"
if [ -x "$venv/bin/python" ]; then vpy="$venv/bin/python"; else vpy="$venv/Scripts/python.exe"; fi
"$vpy" -m pip install -q --disable-pip-version-check "pyinstaller==$PYINSTALLER_VERSION"

# --collect-submodules: the CLI loads command modules by name, which a static
# import scan cannot see. The excludes are stdlib packages with native deps the
# CLI never touches; they only add size.
rm -rf "$work/dist" "$work/build"
"$vpy" -m PyInstaller --onefile --clean --noconfirm --name tmc \
    --paths "$src/src" \
    --collect-submodules tmc_cli \
    --exclude-module tkinter --exclude-module unittest --exclude-module pydoc \
    --distpath "$work/dist" --workpath "$work/build" --specpath "$work" \
    "$src/src/main.py" >"$work/pyinstaller.log" 2>&1 ||
    { tail -30 "$work/pyinstaller.log" >&2; exit 1; }

built="$work/dist/tmc$ext"
[ -f "$built" ] || { echo "PyInstaller produced no $built" >&2; exit 1; }

# The build is only good if it starts and reports the version the source says.
# (Only when the target is the host: a cross target cannot run here.)
want="$(sed -n 's/^__version__ = "\(.*\)"$/\1/p' "$src/src/tmc_cli/version.py")"
got="$("$built" --version)"
case "$got" in
    *"$want"*) ;;
    *) echo "built tmc reports '$got', expected version $want" >&2; exit 1 ;;
esac

cp "$built" "$out/tmc-$triple$ext"
chmod +x "$out/tmc-$triple$ext"
echo "built $out/tmc-$triple$ext ($(du -h "$out/tmc-$triple$ext" | cut -f1)): $got"

if [ "$run_test" -eq 1 ]; then
    TMC_BUNDLED_BIN="$out/tmc-$triple$ext" \
        bash "$here/cargo.sh" test -p tmc-core --lib console::tests::bundled_cli
fi
