import { useState } from 'react'
import { useQuery } from '@tanstack/react-query'

import { partyApi } from '~/lib/api/party'
import type { PartyReservationT } from '~/lib/api/party-schemas'
import { messageOf } from '~/lib/ipc'
import { Toggle } from '~/components/form'
import { clampMinutes } from '~/lib/party/logic'
import { BTN_DANGER, BTN_PRIMARY, ErrorLine, INPUT, Panel, usePartyAction } from './party-ui'

/**
 * Reserve the party's server for a while: ask the site what it will allow
 * (`reserve/offer`), book it, cancel a booking. Managers only; the caller
 * decides whether to render it.
 */
export default function ReservePanel({ partyId, serverId }: { partyId: string; serverId: number }) {
    const offer = useQuery({
        queryKey: ['party', 'reserve-offer', partyId, serverId],
        queryFn: () => partyApi.reserveOffer(partyId, serverId),
    })
    const [minutes, setMinutes] = useState('30')
    const [priv, setPriv] = useState(false)
    const [booked, setBooked] = useState<PartyReservationT | null>(null)

    const o = offer.data
    const reserve = usePartyAction(
        () =>
            partyApi.reserve({
                partyId,
                serverId,
                minutes: clampMinutes(Number(minutes), o?.maxMinutes ?? 0),
                private: priv,
            }),
        (r) => {
            setBooked(r)
            void offer.refetch()
        }
    )
    const cancel = usePartyAction(
        (id: string) => partyApi.cancelReservation(id),
        () => {
            setBooked(null)
            void offer.refetch()
        }
    )

    const active = booked && booked.status !== 'CANCELLED' && booked.releasedAt == null ? booked : null

    return (
        <Panel title="Reserve the server">
            <div className="flex flex-col gap-3 p-3 text-xs">
                {offer.isPending && <p className="text-muted">Checking what the server allows…</p>}
                {offer.error && <ErrorLine>{messageOf(offer.error)}</ErrorLine>}

                {o && !o.available && (
                    <p className="text-muted">
                        Not available right now{o.deny ? `: ${o.deny}` : '.'}
                        {o.opensAt ? ` Opens ${new Date(o.opensAt).toLocaleString()}.` : ''}
                    </p>
                )}

                {active && (
                    <div className="flex flex-wrap items-center gap-2 rounded-lg border border-accent/50 p-2.5">
                        <span className="min-w-0 flex-1">
                            Reserved until {new Date(active.endsAt).toLocaleTimeString()}
                            {active.private ? ' (private)' : ''}.
                        </span>
                        <button
                            type="button"
                            className={BTN_DANGER}
                            disabled={cancel.isPending}
                            onClick={() => cancel.mutate(active.id)}
                        >
                            Cancel reservation
                        </button>
                    </div>
                )}

                {o?.available && !active && (
                    <div className="flex flex-wrap items-end gap-3">
                        <label className="flex flex-col gap-1 font-medium">
                            Minutes{o.maxMinutes > 0 ? ` (up to ${o.maxMinutes})` : ''}
                            <input
                                className={`${INPUT} w-24`}
                                inputMode="numeric"
                                value={minutes}
                                onChange={(e) => setMinutes(e.target.value)}
                            />
                        </label>
                        {o.allowPrivate && (
                            <span className="flex items-center gap-2">
                                Private
                                <Toggle label="Private reservation" checked={priv} onChange={setPriv} />
                            </span>
                        )}
                        <button
                            type="button"
                            className={BTN_PRIMARY}
                            disabled={reserve.isPending}
                            onClick={() => reserve.mutate()}
                        >
                            Reserve
                        </button>
                    </div>
                )}

                {(reserve.error ?? cancel.error) && (
                    <ErrorLine>{messageOf(reserve.error ?? cancel.error)}</ErrorLine>
                )}
            </div>
        </Panel>
    )
}
