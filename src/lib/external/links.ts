/**
 * Pull candidate mod links out of pasted text. A cheap prefilter only: the
 * real parse (and every decision about what is allowed) is `external_parse`.
 * Its job is to leave an ordinary paste of ordinary text alone.
 */

const SCHEMES = ['nxm', 'curseforge', 'modrinth', 'ror2mm', 'prismlauncher', 'steam']

const LINK = new RegExp(`(?:https?|${SCHEMES.join('|')}):\\/\\/[^\\s<>"']+`, 'gi')

export function extractLinks(text: string): string[] {
    const found = text.match(LINK) ?? []

    return [...new Set(found.map((l) => l.replace(/[).,;]+$/, '')))]
}

/** True for a paste target the user is typing into: leave those alone. */
export function isEditable(target: EventTarget | null): boolean {
    const el = target as { tagName?: string; isContentEditable?: boolean } | null

    if (!el) return false

    const tag = el.tagName?.toLowerCase()

    return tag === 'input' || tag === 'textarea' || tag === 'select' || el.isContentEditable === true
}

/** `tmc://external-link` and friends are not installable text, only OS events. */
export function describeSize(bytes: number | null | undefined): string {
    if (bytes == null) return 'unknown size'

    return bytes < 1024 * 1024
        ? `${Math.max(1, Math.round(bytes / 1024))} KB`
        : `${(bytes / 1024 / 1024).toFixed(1)} MB`
}
