import type { SchemeStatusT } from './schemas'

/** What each link scheme normally belongs to, for the warning text. */
export const SCHEME_APPS: Record<string, { label: string; owner: string }> = {
    nxm: { label: 'Nexus Mods links', owner: 'Vortex or Mod Organizer 2' },
    curseforge: { label: 'CurseForge links', owner: 'the CurseForge app' },
    modrinth: { label: 'Modrinth links', owner: 'the Modrinth App' },
    ror2mm: { label: 'Thunderstore links', owner: 'r2modman or the Thunderstore app' },
    prismlauncher: { label: 'Prism Launcher links', owner: 'Prism Launcher' },
}

export type SchemeAdvice =
    | { kind: 'ours' }
    /** Nothing handles it: safe to enable, recommended. */
    | { kind: 'recommend' }
    /** Another app owns it. Opt in only; `warning` must be shown. */
    | { kind: 'optIn'; by: string; warning: string }
    /** Cannot be read or changed here (macOS). */
    | { kind: 'unavailable'; note: string }

export function adviseScheme(scheme: string, status: SchemeStatusT): SchemeAdvice {
    switch (status.state) {
        case 'free':
            return { kind: 'recommend' }
        case 'unknown':
            return {
                kind: 'unavailable',
                note: 'This system does not let the app read or change who opens these links. Change it in the other app or your system settings.',
            }
        case 'registered': {
            if (status.ours) return { kind: 'ours' }

            const owner = SCHEME_APPS[scheme]?.owner ?? 'another app'

            return {
                kind: 'optIn',
                by: status.by,
                warning: `${scheme}:// links currently open in ${status.by} (${owner}). Turning this on will replace it: those links will open GameSpice instead, until you turn it off here and re-enable the other app's link setting.`,
            }
        }
    }
}

/**
 * First run: which schemes to pre-tick. Only free ones. A scheme another app
 * owns is never pre-ticked and is never changed without `overrideExisting`.
 */
export function recommendedSchemes(
    reports: { scheme: string; status: SchemeStatusT }[]
): string[] {
    return reports
        .filter((r) => adviseScheme(r.scheme, r.status).kind === 'recommend')
        .map((r) => r.scheme)
}

export const SETUP_SEEN_KEY = 'tmc.external.setup.seen'

export type SchemeChange = { scheme: string; enable: boolean; overrideExisting: boolean }

/**
 * What to send to `external_scheme_set`, given what the user ticked.
 * Taking over a scheme another app owns needs the scheme in `overrides`
 * (a separate, explicit tick); without it, it is skipped, never forced.
 * Turning one off only applies to a scheme that is ours.
 */
export function planSchemeChanges(
    reports: { scheme: string; status: SchemeStatusT }[],
    wanted: ReadonlySet<string>,
    overrides: ReadonlySet<string>
): SchemeChange[] {
    const out: SchemeChange[] = []

    for (const { scheme, status } of reports) {
        const advice = adviseScheme(scheme, status)
        const want = wanted.has(scheme)

        if (advice.kind === 'ours' && !want) {
            out.push({ scheme, enable: false, overrideExisting: false })
        } else if (advice.kind === 'recommend' && want) {
            out.push({ scheme, enable: true, overrideExisting: false })
        } else if (advice.kind === 'optIn' && want && overrides.has(scheme)) {
            out.push({ scheme, enable: true, overrideExisting: true })
        }
    }

    return out
}
