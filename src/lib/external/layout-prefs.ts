import { z } from 'zod'

import type { SortDir, SortKey } from './selection'

/** The picker's remembered layout: sort and which columns show. */
export const LayoutPrefsSchema = z.object({
    sort: z.object({
        key: z.enum(['name', 'size', 'type']),
        dir: z.enum(['asc', 'desc']),
    }),
    columns: z.object({ size: z.boolean(), type: z.boolean() }),
})
export type LayoutPrefsT = z.infer<typeof LayoutPrefsSchema>

export const LAYOUT_KEY = 'tmc.external.picker.layout'

export const DEFAULT_LAYOUT: LayoutPrefsT = {
    sort: { key: 'name' satisfies SortKey, dir: 'asc' satisfies SortDir },
    columns: { size: true, type: false },
}

type Store = Pick<Storage, 'getItem' | 'setItem'>

function store(s?: Store): Store | null {
    if (s) return s

    try {
        return typeof localStorage === 'undefined' ? null : localStorage
    } catch {
        return null
    }
}

/** Stored layout, or the default when absent, unreadable or from another version. */
export function loadLayout(s?: Store): LayoutPrefsT {
    try {
        const raw = store(s)?.getItem(LAYOUT_KEY)
        const parsed = raw ? LayoutPrefsSchema.safeParse(JSON.parse(raw)) : null

        return parsed?.success ? parsed.data : DEFAULT_LAYOUT
    } catch {
        return DEFAULT_LAYOUT
    }
}

export function saveLayout(prefs: LayoutPrefsT, s?: Store): void {
    try {
        store(s)?.setItem(LAYOUT_KEY, JSON.stringify(prefs))
    } catch {
        // Private mode or a full quota: the layout just is not remembered.
    }
}
