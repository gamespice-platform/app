import { useMemo } from 'react'

import { barHeight, cardStats } from '~/lib/gamespice/card-stats'
import { compact } from '~/lib/api/play'
import type { PlayAppRowT } from '~/components/play-center'

/**
 * A game card's live panel, after the website's /games cards: players now,
 * the 24 h peak and servers up as tiles, then players per hour as bars (the
 * current hour in the accent, quiet hours a hairline, every bar titled with
 * its hour and count). Without history from the site the tiles stay and the
 * chart slot says so, so a card never changes height.
 */

const TILE = 'flex min-w-0 flex-col rounded-lg bg-surface-2/60 px-2.5 py-1.5'
const LABEL = 'truncate text-[10px] font-semibold uppercase tracking-wide text-muted'
const VALUE = 'text-base font-extrabold leading-tight tabular-nums'

export function GameCardStats({
    app,
    accent = 'var(--accent)',
}: {
    app: PlayAppRowT
    accent?: string
}) {
    const stats = useMemo(() => cardStats(app.history24h), [app.history24h])
    const n = stats.hours.length
    const hasChart = n > 0

    return (
        <div
            className="rounded-xl border border-border/70 p-2.5"
            style={{ background: `linear-gradient(135deg, color-mix(in srgb, ${accent} 14%, transparent), transparent 70%)` }}
        >
            <dl className="grid grid-cols-3 gap-2">
                <div className={TILE}>
                    <dt className={LABEL}>Now</dt>
                    <dd className={VALUE}>{compact(app.playerCnt)}</dd>
                </div>
                <div className={TILE}>
                    <dt className={LABEL}>Peak 24h</dt>
                    <dd className={VALUE}>{hasChart ? compact(stats.peak) : '–'}</dd>
                </div>
                <div className={TILE}>
                    <dt className={LABEL}>Servers</dt>
                    <dd className={VALUE}>{compact(app.serverCnt)}</dd>
                </div>
            </dl>
            {hasChart ? (
                <>
                    <div
                        role="img"
                        aria-label={`Players per hour over the last ${n} hours, peak ${stats.peak}`}
                        className="mt-2.5 flex h-12 items-end gap-px"
                    >
                        {stats.hours.map((hr, i) => {
                            const pct = barHeight(hr.players, stats.peak)
                            const last = i === n - 1

                            return (
                                <div
                                    key={hr.ts}
                                    title={`${hr.ts.slice(11, 16)} UTC: ${hr.players} players, ${hr.servers} servers`}
                                    className="flex h-full min-w-0 flex-1 items-end"
                                >
                                    <div
                                        className="w-full rounded-t-[2px]"
                                        style={{
                                            height: pct === 0 ? 2 : `${pct}%`,
                                            background: pct === 0 ? 'var(--color-border, #333)' : accent,
                                            opacity: pct === 0 || last ? 1 : 0.55,
                                        }}
                                    />
                                </div>
                            )
                        })}
                    </div>
                    <p className="mt-1 flex justify-between text-[10px] text-muted" aria-hidden>
                        <span>{n}h ago</span>
                        <span>now</span>
                    </p>
                </>
            ) : (
                <p className="mt-2.5 flex h-12 items-center justify-center rounded-md border border-dashed border-border/60 text-[11px] text-muted">
                    {app.playerCnt > 0 ? 'Players per hour: not reported yet' : 'Quiet right now'}
                </p>
            )}
        </div>
    )
}
