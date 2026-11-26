# Estimated latency (opt-in)

Settings, App, Servers, "Estimate latency" (default off) shows `~45 ms est.` beside a server: a guess from the distance between you and the server, labelled apart from the measured ping. Code: `core/src/net/geo.rs` (database, lookup, formula), `core/src/net/geo_store.rs` (download, refresh, gate), `src/commands/geo.rs` (IPC), `src/components/geo-settings.tsx` and `latency-estimate.tsx` (UI).

## The formula

The same as website-city's `EstimatePingMs`: haversine distance (R = 6371 km) at 0.02 ms/km, 5 ms floor, then times 1.3 and rounded. `geo.rs` carries a parity test with fixed vectors; a change to the website's constants must change both. There is no shared package between the TypeScript website and this Rust crate, so it is a port plus a test, not a dependency.

## The data

IP2Location LITE `DB5LITECSVIPV6` (country to lat/lon, IPv4 and IPv6), **CC BY-SA 4.0**. The attribution line is shown in Settings and in `CREDITS.md`. It is a free download that needs an account's download token and is limited to a few downloads per day. The app stores a compact converted file (`geoip/ip2location.gsgeo` in the app data dir, about 8 bytes per IPv4 and 12 per IPv6 range, loaded into memory on first use), not the CSV.

## Source and token

The source is a URL template (`{token}` is replaced), editable under "Download source"; https only (http to localhost for a mirror). The token is a credential: it is kept in the secure store, never in `settings.json`, never sent back to the webview, and is settable only through `geo_configure`. The source and the "own address" URL are not settings either, so a `settings_patch` cannot aim the token at another host.

## Opt-in gate

`estimateLatency` off: `geo_download` and `geo_estimate` refuse before any I/O, the settings section polls nothing, and rows make no IPC call. On: the first estimate loads the database; one older than 30 days is replaced lazily (at most hourly after a failure). Delete database removes the file and every cache.

## Lookups

Server hosts resolve locally (DNS for names, none for literals) and are looked up in memory. Your own public address is learned with one GET to `selfIpUrl` (default `https://api.ipify.org`, cached an hour); this is the only request besides the download.

## Failures

A missing token, an HTTP error (wrong token, daily limit), a plain-text refusal sent as a 200, a broken zip and a CSV with bad rows each surface as a red message in Settings and leave no half-written database (download to `.part`, atomic rename). A row whose estimate cannot be had shows "est. unavailable" with the reason as its tooltip.
