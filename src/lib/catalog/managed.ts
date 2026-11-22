import { useSyncExternalStore } from 'react'

/**
 * Which apps the user has chosen to manage from this device.
 *
 * WHY LOCAL: the app API has no per-user "managed apps" list. Its `managed`
 * filter on `/apps` means something else (the game has the TMC_APP_MANAGE
 * integration, i.e. it CAN be managed). Until the API grows a user-level list,
 * the choice lives in localStorage as a set of app ids, the same way
 * favourite servers do (`~/lib/favorites`). GameSpice's own games are always
 * managed and never stored here: see `isManaged` in `./filter`.
 */

export const MANAGED_KEY = 'tmc.catalog.managed'
export const MAX_MANAGED = 1000

export type ManagedSet = ReadonlySet<number>

/** Pure toggle. Never mutates; drops the oldest ids past the cap. */
export function toggleManaged(set: ManagedSet, id: number): ManagedSet {
    const next = new Set(set)

    if (next.has(id)) next.delete(id)
    else next.add(id)

    if (next.size > MAX_MANAGED) return new Set([...next].slice(next.size - MAX_MANAGED))

    return next
}

/** Tolerant decode: anything but a list of positive integers is ignored. */
export function parseManaged(raw: string | null): ManagedSet {
    if (!raw) return new Set()

    try {
        const value: unknown = JSON.parse(raw)

        if (!Array.isArray(value)) return new Set()

        return new Set(
            value.filter((v): v is number => typeof v === 'number' && Number.isInteger(v) && v > 0)
        )
    } catch {
        return new Set()
    }
}

// ------------------------------------------------------------------ store

let current: ManagedSet | null = null
const listeners = new Set<() => void>()

function read(): ManagedSet {
    if (current === null) {
        let raw: string | null = null

        try {
            raw = typeof localStorage === 'undefined' ? null : localStorage.getItem(MANAGED_KEY)
        } catch {
            // Blocked storage: start empty rather than crash every render.
        }

        current = parseManaged(raw)
    }

    return current
}

function write(next: ManagedSet): void {
    current = next

    try {
        localStorage.setItem(MANAGED_KEY, JSON.stringify([...next]))
    } catch {
        // Blocked storage: the choice lasts until the window closes.
    }

    for (const listener of listeners) listener()
}

export function toggleStoredManaged(id: number): void {
    write(toggleManaged(read(), id))
}

function subscribe(listener: () => void): () => void {
    listeners.add(listener)

    return () => listeners.delete(listener)
}

export function useManaged(): ManagedSet {
    return useSyncExternalStore(subscribe, read, read)
}
