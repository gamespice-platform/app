import { useCallback, useEffect, useRef, useState } from 'react'

import { isUnavailable } from '~/lib/api/unavailable'
import { pollDelay } from './poll'

/**
 * A reconnecting subscription, in the only transport the site offers: polling.
 *
 * website-city has no websocket or SSE channel for parties or chat. Its pages
 * use react-query `refetchInterval` (chat 4 s, an open DM 3 s, the DM badge
 * 15 s, a party about half its heartbeat), so the app asks at the same pace and
 * stays inside the same rate-limit buckets.
 *
 *   * **Visibility-aware.** Nothing is requested while the window is hidden,
 *     and the poll fires at once when it comes back.
 *   * **Reconnecting.** A failure backs the wait off (`pollDelay`) and the
 *     next success resets it; `error` clears itself when it recovers.
 *   * **Stops on `unavailable`.** A route the site does not serve will not
 *     start serving it in a minute; the hook reports it and goes quiet.
 *
 * `fetcher` may change identity freely; the latest one is used. Change `key`
 * to restart (a different thread, a different room).
 */

export type PollStateT<T> = {
    data: T | undefined
    error: Error | null
    /** The site has no app route for this yet. Polling has stopped. */
    unavailable: boolean
    /** First answer still outstanding. */
    loading: boolean
    /** Poll now, e.g. after a send. Resolves when that poll lands. */
    refresh: () => Promise<void>
}

export type PollOptionsT<T> = {
    intervalMs: number
    /** False pauses everything (signed out, panel closed). */
    enabled?: boolean
    key?: string
    /** Called with each answer, after `data` is set; for folding into a list. */
    onData?: (data: T) => void
}

type SlotT<T> = {
    key: string
    data: T | undefined
    error: Error | null
    unavailable: boolean
    loading: boolean
}

const BLANK = { data: undefined, error: null, unavailable: false, loading: true }

export function usePoll<T>(
    fetcher: () => Promise<T>,
    { intervalMs, enabled = true, key = '', onData }: PollOptionsT<T>
): PollStateT<T> {
    /*
     * One object tagged with the key it answers for. A new key makes the old
     * tag mismatch, so a different thread or room starts blank without an
     * effect having to reset state.
     */
    const [slot, setSlot] = useState<SlotT<T>>({ key, ...BLANK })
    const current = slot.key === key ? slot : { key, ...BLANK }
    const { data, error, unavailable, loading } = current

    const fetcherRef = useRef(fetcher)
    const onDataRef = useRef(onData)
    const runRef = useRef<() => Promise<void>>(() => Promise.resolve())

    useEffect(() => {
        fetcherRef.current = fetcher
        onDataRef.current = onData
    })

    useEffect(() => {
        if (!enabled) return

        const patch = (p: Partial<SlotT<T>>) =>
            setSlot((cur) =>
                cur.key === key ? { ...cur, ...p } : { key, ...BLANK, ...p }
            )

        let alive = true
        let failures = 0
        let timer: ReturnType<typeof setTimeout> | undefined
        let inFlight: Promise<void> | undefined

        const visible = () =>
            typeof document === 'undefined' || document.visibilityState !== 'hidden'

        const schedule = () => {
            if (!alive) return

            clearTimeout(timer)
            timer = setTimeout(() => void tick(), pollDelay(intervalMs, failures))
        }

        const run = async () => {
            try {
                const next = await fetcherRef.current()

                if (!alive) return

                failures = 0
                patch({ error: null, data: next, loading: false })
                onDataRef.current?.(next)
            } catch (e) {
                if (!alive) return

                if (isUnavailable(e)) {
                    patch({ unavailable: true, error: e as Error, loading: false })
                    alive = false

                    return
                }

                failures += 1
                patch({
                    error: e instanceof Error ? e : new Error(String(e)),
                    loading: false,
                })
            }
        }

        const tick = async () => {
            if (!alive) return

            // Hidden: skip the request, keep the clock; visibility wakes it.
            if (visible()) {
                inFlight ??= run().finally(() => {
                    inFlight = undefined
                })

                await inFlight
            }

            schedule()
        }

        runRef.current = async () => {
            clearTimeout(timer)
            inFlight ??= run().finally(() => {
                inFlight = undefined
            })
            await inFlight
            schedule()
        }

        const onVisible = () => {
            if (visible() && alive) {
                clearTimeout(timer)
                void tick()
            }
        }

        document.addEventListener('visibilitychange', onVisible)
        void tick()

        return () => {
            alive = false
            clearTimeout(timer)
            document.removeEventListener('visibilitychange', onVisible)
        }
    }, [enabled, intervalMs, key])

    const refresh = useCallback(() => runRef.current(), [])

    return { data, error, unavailable, loading, refresh }
}
