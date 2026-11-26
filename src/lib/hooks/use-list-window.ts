import { useEffect, useLayoutEffect, useRef, useState, type RefObject } from 'react'

import { maxRenderedRows, windowRange, type WindowRange } from '~/lib/paging'

/** The nearest ancestor that scrolls vertically, or null for the page itself. */
export function scrollParent(node: HTMLElement | null): HTMLElement | null {
    let el = node?.parentElement ?? null

    while (el) {
        const oy = getComputedStyle(el).overflowY

        if (oy === 'auto' || oy === 'scroll') return el

        el = el.parentElement
    }

    return null
}

/**
 * Which slice of a long list to render, so the DOM stays bounded however many
 * rows have been fetched. `box` wraps the rendered rows; `rowSelector` matches
 * the elements whose measured height gives the row height (the first guess is
 * `estimate`). `perRow` is the grid's column count (1 for a table).
 */
export function useListWindow(
    box: RefObject<HTMLElement | null>,
    count: number,
    opts: {
        estimate: number
        perRow?: number
        overscan?: number
        rowSelector: string
        gap?: number
    }
): WindowRange {
    const limit = maxRenderedRows()
    const perRow = opts.perRow ?? 1
    const overscan = opts.overscan ?? 6
    const [rowHeight, setRowHeight] = useState(opts.estimate)
    const [range, setRange] = useState<WindowRange>({
        start: 0,
        end: count,
        before: 0,
        after: 0,
    })
    const frame = useRef(0)
    const latest = useRef({ count, perRow, rowHeight })

    useEffect(() => {
        latest.current = { count, perRow, rowHeight }
    }, [count, perRow, rowHeight])

    const { rowSelector, gap = 0 } = opts
    const run = useRef<() => void>(() => {})

    useLayoutEffect(() => {
        if (limit === 0) return

        const compute = () => {
            frame.current = 0

            const node = box.current

            if (!node) return

            const root = scrollParent(node)
            const rootBox = root
                ? root.getBoundingClientRect()
                : { top: 0, height: window.innerHeight }
            const past = rootBox.top - node.getBoundingClientRect().top
            const { count: n, perRow: per, rowHeight: h } = latest.current

            const next = windowRange({
                count: n,
                rowHeight: h,
                perRow: per,
                scrolledPast: past,
                viewport: rootBox.height,
                overscan,
                maxRendered: limit,
            })

            setRange((old) =>
                old.start === next.start &&
                old.end === next.end &&
                old.before === next.before &&
                old.after === next.after
                    ? old
                    : next
            )

            // Measure real rows so spacers track the true height (expanded rows, wrapped cards).
            const rows = node.querySelectorAll<HTMLElement>(rowSelector)

            if (rows.length >= 3) {
                let total = 0

                for (const r of rows) total += r.offsetHeight

                const avg = Math.round((total / rows.length + gap) / 4) * 4 || h

                if (avg > 0 && Math.abs(avg - h) >= 8) setRowHeight(avg)
            }
        }
        const schedule = () => {
            if (!frame.current) frame.current = requestAnimationFrame(compute)
        }
        const target: HTMLElement | Window = scrollParent(box.current) ?? window

        run.current = compute
        target.addEventListener('scroll', schedule, { passive: true })
        window.addEventListener('resize', schedule)

        return () => {
            target.removeEventListener('scroll', schedule)
            window.removeEventListener('resize', schedule)

            if (frame.current) cancelAnimationFrame(frame.current)

            frame.current = 0
        }
    }, [box, limit, overscan, rowSelector, gap])

    // Re-window whenever the data or the measured row height changes.
    useLayoutEffect(() => {
        run.current()
    }, [count, perRow, rowHeight, limit])

    return limit === 0
        ? { start: 0, end: count, before: 0, after: 0 }
        : { ...range, end: Math.min(range.end, count) }
}
