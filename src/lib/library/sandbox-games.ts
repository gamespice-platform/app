import { appLabel } from '~/lib/api/labels'

/**
 * The games New sandbox offers: every game this build has a manage rule for.
 *
 * It used to be the site's mod facets (`/facets?kind=mod`) narrowed to the
 * supported slugs, which only lists games with at least one mod PUBLISHED on
 * the site. A game the user set up through Find games but nobody has uploaded
 * a mod for yet (Minecraft on a fresh site) was missing from the picker even
 * though the app manages it fine. The list now comes from the supported slugs
 * themselves, resolved to apps with `/apps?slugs=`, so a game is offered
 * exactly when the app can build a sandbox for it.
 */

export type SandboxGame = {
    id: number
    name: string
    slug: string | null
    icon: string | null
}

type AppRow = {
    id: number
    name: string
    slug: string | null
    images: { icon: string | null }
}

/** The supported slugs, lower-cased and de-duplicated, in a stable order. */
export function sandboxSlugs(supported: readonly string[]): string[] {
    return [...new Set(supported.map((s) => s.trim().toLowerCase()).filter(Boolean))].sort()
}

/** One option per app whose slug has a manage rule, sorted by name. */
export function sandboxGameOptions(
    apps: readonly AppRow[],
    supported: readonly string[]
): SandboxGame[] {
    const slugs = new Set(sandboxSlugs(supported))
    const seen = new Set<number>()
    const out: SandboxGame[] = []

    for (const app of apps) {
        const slug = app.slug?.replace(/^\//, '').toLowerCase() ?? null

        if (!slug || !slugs.has(slug) || seen.has(app.id)) continue
        seen.add(app.id)
        out.push({ id: app.id, name: appLabel(app), slug, icon: app.images.icon })
    }

    return out.sort((a, b) => a.name.localeCompare(b.name))
}
