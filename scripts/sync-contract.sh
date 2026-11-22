#!/usr/bin/env bash
#
# Copy the API contract across from website-city, which owns it.
#
# The app parses every response through these schemas, so the two copies have to
# agree — and a hand-edited copy is exactly how they stop agreeing. Run this
# after any change to `website-city/src/types/app-api/contract.ts`, then
# `npm run check:web`: a field the app relies on and the server stopped sending
# becomes a type error rather than a runtime surprise.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="${CONTRACT_SRC_ROOT:-$here/../../website-city}"
src="$root/src/types/app-api/contract.ts"
dst="$here/../src/lib/api/contract.ts"

[ -f "$src" ] || {
    echo "No contract at $src — is website-city checked out next door?" >&2
    exit 1
}

header='/**
 * The contract for `/api/app/v1` — the app'"'"'s whole view of TMC.
 *
 * **This file is a VERBATIM MIRROR of
 * `website-city/src/types/app-api/contract.ts`, which is the source of truth.**
 * Copy it across whenever the server side changes; do not edit it here
 * (`npm run contract:sync` does the copy). Every response is parsed through
 * these schemas before it reaches a component, so a drift shows up as a loud
 * validation error on the first request rather than as an `undefined` three
 * screens deep.
 *'

# Replace the source-of-truth header block with the mirror one. Everything from
# the first `/**` to the line before ` * Why a separate API` is the header.
awk -v header="$header" '
    NR == 1 { print; next }
    /^\/\*\*$/ && !seen { seen = 1; print header; skipping = 1; next }
    skipping && /^ \* Why a separate API/ { skipping = 0 }
    !skipping { print }
' "$src" > "$dst"

echo "✓ contract mirrored into src/lib/api/contract.ts"

# The game-catalog examples the server's routes produce (W5). The app's tests
# parse every one through the mirrored schemas and through its own IPC schemas,
# and the Rust catalog client reads them as wire input, so a field renamed on
# the server fails here before it fails on a user's machine.
fixtures="$root/src/types/game-catalog/fixtures"
fixtures_dst="$here/../src/lib/api/fixtures/catalog"

if [ -d "$fixtures" ]; then
    rm -rf "$fixtures_dst"
    mkdir -p "$fixtures_dst/wire"
    for name in catalog versions loader-versions install-plan runtime; do
        cp "$fixtures/$name.json" "$fixtures_dst/$name.json"
    done
    cp "$fixtures"/wire/*.json "$fixtures_dst/wire/"
    echo "✓ catalog fixtures mirrored into src/lib/api/fixtures/catalog"
else
    echo "No catalog fixtures at $fixtures — left src/lib/api/fixtures/catalog as it was" >&2
fi
