/**
 * What a game card charts: the last hours of players, the peak and when it
 * was. The mirror of the website's `cardstats.ts` (website-gs), pure so the
 * numbers on a card are tested rather than eyeballed.
 */

export type CardHourT = { ts: string; players: number; servers: number }

export type CardStatsT = {
    /** Oldest first; the last is the current (partial) hour. */
    hours: CardHourT[]
    /** Highest hourly players in `hours` (0 when none). */
    peak: number
    /** The hour of `peak`, or null when it is 0. */
    peakAt: string | null
}

/** A card's numbers from the 24 h history; `null` or empty history gives no hours. */
export function cardStats(
    history: readonly { ts: string; players: number; serversOnline: number }[] | null | undefined
): CardStatsT {
    const hours = (history ?? []).map((p) => ({
        ts: p.ts,
        players: p.players,
        servers: p.serversOnline,
    }))
    let peak = 0
    let peakAt: string | null = null

    for (const h of hours) {
        if (h.players > peak) {
            peak = h.players
            peakAt = h.ts
        }
    }

    return { hours, peak, peakAt }
}

/** A bar's height in percent of the chart: 0 for nobody, else at least 8. */
export function barHeight(players: number, peak: number): number {
    if (players <= 0 || peak <= 0) return 0

    return Math.max(8, Math.round((players / peak) * 1000) / 10)
}
