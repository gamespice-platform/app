/**
 * What a queue row says about itself beyond its label.
 *
 * Rust puts `role`, `action`, `fromVersion`, `platform`, `usedBy` and `reason`
 * in a game download's `meta` (`games/describe.rs`); every key is optional,
 * because a row from an older queue or another kind carries none of them.
 */

export interface DownloadInfo {
    /** One line under the title, or null when meta says nothing. */
    subtitle: string | null
    /** Extra rows for the expanded details, in display order. */
    details: { label: string; value: string }[]
}

export function downloadInfo(meta: Record<string, string>): DownloadInfo {
    if (meta.kind !== 'game' || !meta.role) {
        return { subtitle: null, details: [] }
    }

    const engine = meta.role === 'engine'
    const update = meta.action === 'update'
    const noun = engine ? 'Engine' : 'Game'

    const parts = [
        update
            ? `${noun} update${meta.fromVersion ? ` from ${meta.fromVersion}` : ''}`
            : `${noun} install`,
    ]

    if (engine && meta.usedBy) {
        parts.push(`used by ${meta.usedBy}`)
    }

    const details: DownloadInfo['details'] = []

    if (meta.version) details.push({ label: 'Version', value: meta.version })
    if (meta.fromVersion) {
        details.push({ label: 'Previous version', value: meta.fromVersion })
    }
    if (meta.platform) details.push({ label: 'Platform', value: meta.platform })
    if (meta.usedBy) details.push({ label: 'Used by', value: meta.usedBy })
    if (meta.reason) {
        details.push({
            label: 'Started',
            value:
                meta.reason === 'automatic' ? 'Automatic update' : 'Manual request',
        })
    }

    return { subtitle: parts.join(' · '), details }
}
