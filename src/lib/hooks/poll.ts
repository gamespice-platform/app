/**
 * The pure half of `usePoll`: when to ask next, and how to fold a poll's answer
 * into what is already on screen. Kept apart from React so it is tested alone.
 */

/** Per-failure growth of the wait. */
export const POLL_BACKOFF_FACTOR = 2

/**
 * The longest the wait may grow to, as a multiple of the base interval. A knob,
 * not a limit on retries: polling never gives up on a transient failure, it just
 * slows down. 0 would mean "no growth ceiling".
 */
export const POLL_BACKOFF_MAX_MULTIPLE = 8

/** Milliseconds until the next poll after `failures` consecutive errors. */
export function pollDelay(
    baseMs: number,
    failures: number,
    maxMultiple = POLL_BACKOFF_MAX_MULTIPLE
): number {
    if (failures <= 0) return baseMs

    const grown = baseMs * POLL_BACKOFF_FACTOR ** failures

    return maxMultiple > 0 ? Math.min(grown, baseMs * maxMultiple) : grown
}

/**
 * Merge a newer page into the list by id, keeping the oldest-first order the
 * rooms render in. A message already held is replaced, which is how an edit, a
 * deletion or a reaction count arrives.
 */
export function mergeById<T extends { id: string }>(
    held: readonly T[],
    incoming: readonly T[],
    compare: (a: T, b: T) => number
): T[] {
    if (incoming.length === 0) return held as T[]

    const byId = new Map<string, T>()

    for (const row of held) byId.set(row.id, row)
    for (const row of incoming) byId.set(row.id, row)

    return [...byId.values()].sort(compare)
}

/** Decimal-string ids order by length then text, which is numeric order and needs no BigInt. */
export function compareIds(a: string, b: string): number {
    if (a.length !== b.length) return a.length - b.length

    return a < b ? -1 : a > b ? 1 : 0
}
