import { partyApi } from '~/lib/api/party'
import type { AppPartyInviteT } from '~/lib/api/contract'
import { messageOf } from '~/lib/ipc'
import { BTN_GHOST, BTN_PRIMARY, ErrorLine, Panel, usePartyAction } from './party-ui'

/** Pending invites to the viewer: accept (which joins, so capacity still decides) or decline. */
export default function InvitesList({
    invites,
    onChanged,
}: {
    invites: AppPartyInviteT[]
    onChanged: () => void
}) {
    if (invites.length === 0) return null

    return (
        <Panel title={`Invites (${invites.length})`}>
            <ul className="flex flex-col divide-y divide-border">
                {invites.map((i) => (
                    <InviteItem key={i.id} invite={i} onChanged={onChanged} />
                ))}
            </ul>
        </Panel>
    )
}

function InviteItem({ invite, onChanged }: { invite: AppPartyInviteT; onChanged: () => void }) {
    const accept = usePartyAction(() => partyApi.respondToInvite(invite.id, true), onChanged)
    const decline = usePartyAction(() => partyApi.respondToInvite(invite.id, false), onChanged)
    const busy = accept.isPending || decline.isPending
    const error = accept.error ?? decline.error

    return (
        <li className="flex items-center gap-3 p-2.5">
            <div className="min-w-0 flex-1">
                <p className="truncate text-sm font-medium">
                    {invite.partyName ?? `Party ${invite.partyId}`}
                </p>
                {invite.message && <p className="truncate text-[11px] text-muted">{invite.message}</p>}
                {error && <ErrorLine>{messageOf(error)}</ErrorLine>}
            </div>
            <button type="button" className={BTN_PRIMARY} disabled={busy} onClick={() => accept.mutate()}>
                Accept
            </button>
            <button type="button" className={BTN_GHOST} disabled={busy} onClick={() => decline.mutate()}>
                Decline
            </button>
        </li>
    )
}
