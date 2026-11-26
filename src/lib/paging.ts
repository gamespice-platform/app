/**
 * Paging and windowing rules for the server browser, as pure functions.
 *
 * The browser used to fetch "while the sentinel is near the viewport" from an
 * effect that re-ran on every render, so a short page, a re-render (live
 * latency ticks, refresh) or a repeating cursor kept it requesting pages with
 * nobody scrolling, until thousands of rows were in the DOM. These rules make
 * each of those cases a no-op and keep the rendered row count bounded.
 */

/** Keep the first occurrence of each id; later copies (overlapping pages) are dropped. */
export function dedupeById<T>(
    rows: readonly T[],
    idOf: (row: T) => string | number
): T[] {
    const seen = new Set<string | number>()
    const out: T[] = []

    for (const row of rows) {
        const id = idOf(row)

        if (seen.has(id)) continue

        seen.add(id)
        out.push(row)
    }

    return out
}

/**
 * How close (px) the list's end must be to the bottom of the view before the
 * next page loads. Small on purpose: scrolling anywhere in the middle of the
 * list must not fetch, only reaching the bottom does. (It was 1500, which put
 * the trigger several screens early, so every scroll loaded servers.)
 */
export const PAGE_MARGIN = 120

export interface PageState {
    hasNextPage: boolean
    isFetchingNextPage: boolean
    /** A previous page failed; wait for an explicit retry. */
    pageError: boolean
    /** Distance from the sentinel's top to the scroll container's bottom, px. */
    distanceToEnd: number
    /** How far ahead of the end we start loading, px. */
    margin: number
    /** True only once the user has actually scrolled (wheel/touch/key/scroll). */
    userScrolled: boolean
    /** The cursor/length that the last request used; unchanged means no progress. */
    stalled: boolean
}

/** Whether the next page should be requested right now. */
export function shouldFetchNext(s: PageState): boolean {
    if (!s.userScrolled || s.stalled) return false
    if (!s.hasNextPage || s.isFetchingNextPage || s.pageError) return false

    return s.distanceToEnd < s.margin
}

/**
 * Stop paging when a page added nothing new: the API repeating a cursor or
 * returning an empty/overlapping page must end the walk, not restart it.
 */
export function pageMadeProgress(before: number, after: number): boolean {
    return after > before
}

export interface WindowInput {
    count: number
    /** Estimated height of one rendered row (a grid row counts once), px. */
    rowHeight: number
    /** Items per rendered row (1 for the table). */
    perRow: number
    /** Scroll offset of the list's top edge above the viewport top, px (>= 0 once scrolled in). */
    scrolledPast: number
    viewport: number
    /** Extra rows rendered each side of the viewport. */
    overscan: number
    /** Hard row limit; 0 = unlimited, i.e. no windowing at all. */
    maxRendered: number
}

export interface WindowRange {
    start: number
    end: number
    /** Spacer heights replacing the rows left out, px. */
    before: number
    after: number
}

/** The slice of items to render, plus spacers that keep the scroll height true. */
export function windowRange(w: WindowInput): WindowRange {
    if (w.maxRendered === 0 || w.count === 0 || w.rowHeight <= 0) {
        return { start: 0, end: w.count, before: 0, after: 0 }
    }

    const per = Math.max(1, w.perRow)
    const rows = Math.ceil(w.count / per)
    const first = Math.floor(Math.max(0, w.scrolledPast) / w.rowHeight) - w.overscan
    const visible = Math.ceil(w.viewport / w.rowHeight) + 2 * w.overscan
    const maxRows = Math.max(1, Math.ceil(w.maxRendered / per))
    const span = Math.min(rows, Math.max(visible, 1), maxRows)
    const startRow = Math.min(Math.max(0, first), Math.max(0, rows - span))
    const endRow = startRow + span

    return {
        start: startRow * per,
        end: Math.min(w.count, endRow * per),
        before: startRow * w.rowHeight,
        after: (rows - endRow) * w.rowHeight,
    }
}

const KEY = 'gs.browse.maxRendered'

/** Rendered-row limit; 0 disables windowing. Override via localStorage `gs.browse.maxRendered`. */
export function maxRenderedRows(
    storage: Pick<Storage, 'getItem'> | undefined = safeStorage()
): number {
    const raw = storage?.getItem(KEY)
    const n = raw == null || raw === '' ? NaN : Number(raw)

    return Number.isInteger(n) && n >= 0 ? n : 400
}

function safeStorage(): Storage | undefined {
    try {
        return typeof localStorage === 'undefined' ? undefined : localStorage
    } catch {
        return undefined
    }
}
