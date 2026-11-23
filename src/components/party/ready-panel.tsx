import { useEffect, useState } from 'react'
import { openUrl } from '@tauri-apps/plugin-opener'
import { FiCheck, FiExternalLink, FiPlay } from 'react-icons/fi'

import { partyApi } from '~/lib/api/party'
import type { PartyReadyViewT } from '~/lib/api/party-schemas'
import { messageOf } from '~/lib/ipc'
import { ipc } from '~/lib/ipc/commands'
import { clock, connectRoute, countdownRemaining, readyPercent, stageLabel } from '~/lib/party/logic'
import { BTN_DANGER, BTN_GHOST, BTN_PRIMARY, ErrorLine, Panel, usePartyAction } from './party-ui'

/**
 * The ready check. Everyone toggles ready; managers open the check, force the
 * start, or cancel it. The countdown ticks locally from the site's figure and
 * is re-anchored by every poll (`answeredAt`).
 */
export default function ReadyPanel({
    partyId,
    view,
    serverId,
    onChanged,
}: {
    partyId: string
    view: PartyReadyViewT
    serverId: number | null
    onChanged: () => void
}) {
    // Milliseconds since `view` arrived, so the countdown advances between polls.
    // Tied to the view it was counted for: a fresh answer restarts from zero.
    const [tick, setTick] = useState<{ view: PartyReadyViewT; ms: number } | null>(null)
    const checking = view.stage === 'READY'

    useEffect(() => {
        const total = view.countdownSec

        if (total == null) return

        const began = performance.now()
        let t: ReturnType<typeof setInterval> | undefined
        const stop = () => {
            clearInterval(t)
            t = undefined
        }
        const step = () => {
            const ms = performance.now() - began

            setTick({ view, ms })
            // Nothing left to count: stop rather than re-render twice a second at 0:00.
            if (ms / 1000 >= total) stop()
        }
        const start = () => {
            if (t === undefined && !document.hidden) t = setInterval(step, 500)
        }
        // Hidden windows do not tick; coming back re-syncs at once.
        const onVisibility = () => {
            if (document.hidden) return stop()

            step()
            start()
        }

        document.addEventListener('visibilitychange', onVisibility)
        start()

        return () => {
            document.removeEventListener('visibilitychange', onVisibility)
            stop()
        }
    }, [view])

    const ready = usePartyAction((next: boolean) => partyApi.setReady(partyId, next), onChanged)
    const start = usePartyAction(() => partyApi.start(partyId, serverId ?? undefined), onChanged)
    const force = usePartyAction(() => partyApi.forceStart(partyId), onChanged)
    const cancel = usePartyAction(() => partyApi.cancelStart(partyId), onChanged)
    const busy = ready.isPending || start.isPending || force.isPending || cancel.isPending
    // https opens in the system browser; any other scheme goes through Rust, which
    // checks it against LAUNCH_SCHEMES (the webview's opener refuses it silently).
    const connect = usePartyAction(async () => {
        const c = view.connect

        if (!c?.href) return

        await partyApi.connected(partyId).catch(() => undefined)

        if (connectRoute(c.href) === 'web') await openUrl(c.href)
        else await ipc.playConnect(c.serverId)
    }, onChanged)
    const error = ready.error ?? start.error ?? force.error ?? cancel.error ?? connect.error
    const left = countdownRemaining(view, tick?.view === view ? tick.ms : 0)
    const pct = readyPercent(view)

    return (
        <Panel title="Ready check">
            <div className="flex flex-col gap-3 p-3">
                <p className="text-xs text-muted">
                    {stageLabel(view.stage)} · {view.ready} of {view.total} ready
                    {left != null ? ` · starts in ${clock(left)}` : ''}
                </p>

                <div
                    role="progressbar"
                    aria-label="Players ready"
                    aria-valuemin={0}
                    aria-valuemax={100}
                    aria-valuenow={pct}
                    className="h-1.5 overflow-hidden rounded-full bg-surface-2"
                >
                    <div className="h-full bg-accent transition-all" style={{ width: `${pct}%` }} />
                </div>

                {view.membersHidden && (
                    <p className="text-[11px] text-muted">
                        This party keeps its roster private, so only totals are shown.
                    </p>
                )}

                <div className="flex flex-wrap items-center gap-2">
                    {view.me.isMember && !view.started && (
                        <button
                            type="button"
                            disabled={busy}
                            aria-pressed={view.me.ready}
                            className={view.me.ready ? BTN_GHOST : BTN_PRIMARY}
                            onClick={() => ready.mutate(!view.me.ready)}
                        >
                            <FiCheck className="size-3.5" aria-hidden />
                            {view.me.ready ? 'Not ready' : 'Ready'}
                        </button>
                    )}

                    {view.me.canManage && !view.started && !checking && (
                        <button type="button" disabled={busy} className={BTN_PRIMARY} onClick={() => start.mutate()}>
                            <FiPlay className="size-3.5" aria-hidden />
                            Start ready check
                        </button>
                    )}
                    {view.me.canManage && checking && (
                        <>
                            <button type="button" disabled={busy} className={BTN_PRIMARY} onClick={() => force.mutate()}>
                                Start now
                            </button>
                            <button type="button" disabled={busy} className={BTN_DANGER} onClick={() => cancel.mutate()}>
                                Cancel start
                            </button>
                        </>
                    )}
                </div>

                {view.started && view.connect && (
                    <div className="flex flex-wrap items-center gap-2 rounded-lg border border-accent/50 p-2.5 text-xs">
                        <span className="min-w-0 flex-1">
                            Started on {view.connect.serverName ?? `server ${view.connect.serverId}`}.
                        </span>
                        {view.connect.href && (
                            <button
                                type="button"
                                className={BTN_PRIMARY}
                                disabled={connect.isPending}
                                onClick={() => connect.mutate()}
                            >
                                <FiExternalLink className="size-3.5" aria-hidden />
                                Connect
                            </button>
                        )}
                    </div>
                )}

                {error && <ErrorLine>{messageOf(error)}</ErrorLine>}
            </div>
        </Panel>
    )
}
