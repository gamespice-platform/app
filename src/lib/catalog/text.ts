/**
 * The `catalog.*` strings: every word the version and loader pickers, the
 * install plan panel and the consent dialog put on screen.
 *
 * The app has no translation layer yet, so English lives here as one keyed
 * table rather than scattered through JSX. When one arrives, this file is the
 * catalog namespace it loads; until then `ct` is the lookup.
 */
const TEXT = {
    'catalog.version.label': 'Game version',
    'catalog.version.any': 'Any version',
    'catalog.version.search': 'Find or type a version',
    'catalog.version.customRow': 'Custom…',
    'catalog.version.custom':'Use “{value}” as a custom version',
    'catalog.version.customHint': 'TMC will not install this; install it yourself.',
    'catalog.version.customBad':
        'A version is letters, digits and . _ + - only, starting with a letter or digit, at most 64 characters.',
    'catalog.version.unknown': '{value} (custom)',
    'catalog.version.deprecated': 'Deprecated',
    'catalog.version.withdrawn': 'Withdrawn',
    'catalog.version.unmanaged': 'Not installed by TMC',
    'catalog.loader.label': 'Loader',
    'catalog.loader.none': 'None (vanilla)',
    'catalog.loader.unknown': 'Unknown loader “{value}”',
    'catalog.loader.unknownHint': 'Not in this game’s catalog. Kept as typed; TMC will not install it.',
    'catalog.loader.serverOnly': 'Server only',
    'catalog.loader.clientOnly': 'Client only',
    'catalog.loaderVersion.label': 'Loader version',
    'catalog.loaderVersion.newest': 'Newest',
    'catalog.loaderVersion.recommended': 'Recommended',
    'catalog.loaderVersion.none': 'No {loader} build for {gameVersion}',
    'catalog.loaderVersion.moved': '{loader} {from} has no build for {gameVersion}; switched to {to}.',
    'catalog.loaderVersion.loading': 'Loading versions…',
    'catalog.source.stale': 'Offline: list from {date}.',
    'catalog.source.compiled': 'Offline: this list shipped with the app and may be missing newer versions.',
    'catalog.source.refresh': 'Try again',
    'catalog.source.failed': 'Could not load the version list: {error}',
    'catalog.free.version': 'Any',
    'catalog.free.loader': 'None',
    'catalog.free.hint': 'This game has no version list, so type the version and loader yourself.',
    'catalog.plan.title': 'What installing this does',
    'catalog.plan.loading': 'Working out the install…',
    'catalog.plan.nothing': 'Nothing to install: the game’s own files are used as they are.',
    'catalog.plan.step': 'Step {n}',
    'catalog.plan.details': 'Show the step as written',
    'catalog.plan.hosts': 'Downloads from {hosts}',
    'catalog.plan.runtime': 'Needs {kind} {major} to {purpose}; TMC downloads it if it is missing.',
    'catalog.plan.runtimeInstall': 'run the installer',
    'catalog.plan.runtimeLaunch': 'start the game',
    'catalog.plan.argv': 'Runs, one argument per line:',
    'catalog.consent.title': 'Install {title}?',
    'catalog.consent.intro': 'These are the exact steps the install runs, in order.',
    'catalog.consent.exec':
        'This runs a program from {host} on your computer with your user’s permissions. TMC checked its fingerprint; it is not sandboxed.',
    'catalog.consent.trust.official': 'Published by TMC, signed. Plan {hash}.',
    'catalog.consent.trust.compiled': 'Shipped inside this app by TMC. Plan {hash}.',
    'catalog.consent.trust.local': 'Local file, unsigned. Plan {hash}. Only continue if you wrote it or trust whoever did.',
    'catalog.consent.read': 'Read it',
    'catalog.consent.install': 'Install',
    'catalog.consent.cancel': 'Cancel',
    'catalog.consent.changed': 'The plan changed while this was open. Review it again.',
    'catalog.progress.title': 'Installing',
    'catalog.progress.done': 'Installed and deployed.',
    'catalog.progress.failed': 'Install failed.',
    'catalog.progress.log': 'Show the log',
    'catalog.progress.close': 'Close',
    'catalog.progress.waiting': 'Waiting',
    'catalog.progress.running': 'Running',
    'catalog.progress.stepDone': 'Done',
    'catalog.progress.skipped': 'Already there',
    'catalog.progress.stepFailed': 'Failed',
    'catalog.progress.noLog': 'The step left no log.',
} as const

export type CatalogTextKey = keyof typeof TEXT

/** The text for `key` with `{name}` filled from `vars`. */
export function ct(
    key: CatalogTextKey,
    vars: Record<string, string | number> = {}
): string {
    return TEXT[key].replace(/\{(\w+)\}/g, (whole, name: string) =>
        name in vars ? String(vars[name]) : whole
    )
}
