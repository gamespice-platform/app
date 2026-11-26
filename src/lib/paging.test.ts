import { describe, expect, it } from 'vitest'

import {
    dedupeById,
    maxRenderedRows,
    pageMadeProgress,
    shouldFetchNext,
    windowRange,
    type PageState,
} from './paging'

const base: PageState = {
    hasNextPage: true,
    isFetchingNextPage: false,
    pageError: false,
    distanceToEnd: 100,
    margin: 1500,
    userScrolled: true,
    stalled: false,
}

describe('shouldFetchNext', () => {
    it('fetches when the user has scrolled near the end', () => {
        expect(shouldFetchNext(base)).toBe(true)
    })

    /* The regression: a visible sentinel with no scrolling used to chain fetches. */
    it('never fetches before the user scrolls, however close the sentinel is', () => {
        expect(
            shouldFetchNext({ ...base, userScrolled: false, distanceToEnd: -500 })
        ).toBe(false)
    })

    it.each([
        ['no more pages', { hasNextPage: false }],
        ['already fetching', { isFetchingNextPage: true }],
        ['a failed page', { pageError: true }],
        ['a page that added nothing', { stalled: true }],
        ['far from the end', { distanceToEnd: 5000 }],
    ])('does not fetch with %s', (_n, patch) => {
        expect(shouldFetchNext({ ...base, ...patch })).toBe(false)
    })
})

describe('pageMadeProgress', () => {
    it('is false when a page adds no new rows', () => {
        expect(pageMadeProgress(24, 24)).toBe(false)
        expect(pageMadeProgress(24, 48)).toBe(true)
    })
})

describe('dedupeById', () => {
    it('keeps the first of each id and the order', () => {
        const rows = [
            { id: 1, n: 'a' },
            { id: 2, n: 'b' },
            { id: 1, n: 'c' },
        ]

        expect(dedupeById(rows, (r) => r.id).map((r) => r.n)).toEqual(['a', 'b'])
    })

    it('handles a refresh replacing the list by content, not appending', () => {
        const page = [{ id: 1 }, { id: 2 }]

        expect(dedupeById([...page, ...page], (r) => r.id)).toHaveLength(2)
    })
})

describe('windowRange', () => {
    const w = {
        count: 100_000,
        rowHeight: 50,
        perRow: 1,
        scrolledPast: 0,
        viewport: 800,
        overscan: 5,
        maxRendered: 400,
    }

    it('bounds the rendered rows for a huge list and keeps scroll height', () => {
        const r = windowRange({ ...w, scrolledPast: 250_000 })

        expect(r.end - r.start).toBeLessThanOrEqual(400)
        expect(r.end - r.start).toBeGreaterThan(0)
        expect(r.before + r.after + (r.end - r.start) * 50).toBe(100_000 * 50)
        expect(r.start).toBeLessThanOrEqual(5000)
        expect(r.end).toBeGreaterThanOrEqual(5000)
    })

    it('clamps at both ends', () => {
        expect(windowRange({ ...w, scrolledPast: -10 }).start).toBe(0)
        expect(windowRange({ ...w, scrolledPast: 1e12 }).end).toBe(100_000)
    })

    it('0 means unlimited: everything renders', () => {
        expect(windowRange({ ...w, maxRendered: 0 })).toEqual({
            start: 0,
            end: 100_000,
            before: 0,
            after: 0,
        })
    })

    it('windows by rows in a grid', () => {
        const r = windowRange({ ...w, count: 1000, perRow: 4, rowHeight: 200 })

        expect(r.start % 4).toBe(0)
        expect(r.end - r.start).toBeLessThanOrEqual(400)
    })
})

describe('maxRenderedRows', () => {
    it('defaults, parses, and allows 0', () => {
        expect(maxRenderedRows({ getItem: () => null })).toBe(400)
        expect(maxRenderedRows({ getItem: () => '0' })).toBe(0)
        expect(maxRenderedRows({ getItem: () => '120' })).toBe(120)
        expect(maxRenderedRows({ getItem: () => 'x' })).toBe(400)
    })
})
