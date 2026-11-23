import { useState } from 'react'
import { Link } from 'react-router-dom'

import { partyApi } from '~/lib/api/party'
import type { AppPartyMemberT } from '~/lib/api/contract'
import { messageOf } from '~/lib/ipc'
import { joinedMembers, memberName, memberPowers, roleLabel } from '~/lib/party/logic'
import { BTN_DANGER, BTN_GHOST, ErrorLine, Initial, Panel, usePartyAction } from './party-ui'

/**
 * Who is in the party, with the host's tools: make co-host, hand over the
 * host role, kick (and ban). Each destructive step asks first.
 */
export default function MembersPanel({
    partyId,
    members,
    myUserIds,
    myRole,
    readyUserIds,
    onChanged,
}: {
    partyId: string
    members: AppPartyMemberT[]
    /** The viewer's own member ids, from the ready panel. */
    myUserIds: Set<string>
    myRole: string
    readyUserIds: Set<string>
    onChanged: () => void
}) {
    const list = joinedMembers(members)

    return (
        <Panel title={`Members (${list.length})`}>
            <ul className="flex flex-col divide-y divide-border">
                {list.map((m) => (
                    <MemberRow
                        key={m.userId}
                        partyId={partyId}
                        member={m}
                        isMe={myUserIds.has(m.userId)}
                        myRole={myRole}
                        ready={readyUserIds.has(m.userId) || m.readyAt != null}
                        onChanged={onChanged}
                    />
                ))}
                {list.length === 0 && (
                    <li className="p-3 text-xs text-muted">
                        The roster is hidden by this party.
                    </li>
                )}
            </ul>
        </Panel>
    )
}

type PendingT = 'kick' | 'transfer' | null

function MemberRow({
    partyId,
    member,
    isMe,
    myRole,
    ready,
    onChanged,
}: {
    partyId: string
    member: AppPartyMemberT
    isMe: boolean
    myRole: string
    ready: boolean
    onChanged: () => void
}) {
    const [confirm, setConfirm] = useState<PendingT>(null)
    const powers = memberPowers(myRole, { role: member.role, isMe })
    const done = () => {
        setConfirm(null)
        onChanged()
    }
    const kick = usePartyAction((ban: boolean) => partyApi.kick(partyId, member.userId, ban), done)
    const transfer = usePartyAction(() => partyApi.transferHost(partyId, member.userId), done)
    const role = usePartyAction(
        () =>
            partyApi.setRole(partyId, member.userId, member.role === 'CO_HOST' ? 'MEMBER' : 'CO_HOST'),
        done
    )
    const busy = kick.isPending || transfer.isPending || role.isPending
    const error = kick.error ?? transfer.error ?? role.error
    const name = memberName(member)

    return (
        <li className="flex flex-wrap items-center gap-2.5 p-2.5">
            <Initial name={name} />
            <div className="min-w-0 flex-1">
                <Link
                    to={`/view/user/${encodeURIComponent(member.userId)}`}
                    className="block truncate text-sm font-medium hover:underline"
                >
                    {name}
                    {isMe ? ' (you)' : ''}
                </Link>
                <p className="text-[11px] text-muted">
                    {roleLabel(member.role)}
                    {ready ? ' · ready' : ''}
                    {member.connectedAt ? ' · connected' : ''}
                </p>
                {error && <ErrorLine>{messageOf(error)}</ErrorLine>}
            </div>

            {confirm === 'kick' && (
                <span className="flex items-center gap-1" role="group" aria-label={`Remove ${name}`}>
                    <button type="button" disabled={busy} className={BTN_DANGER} onClick={() => kick.mutate(false)}>
                        Kick
                    </button>
                    <button type="button" disabled={busy} className={BTN_DANGER} onClick={() => kick.mutate(true)}>
                        Kick and ban
                    </button>
                    <button type="button" className={BTN_GHOST} onClick={() => setConfirm(null)}>
                        Keep
                    </button>
                </span>
            )}

            {confirm === 'transfer' && (
                <span className="flex items-center gap-1" role="group" aria-label={`Make ${name} host`}>
                    <button type="button" disabled={busy} className={BTN_DANGER} onClick={() => transfer.mutate()}>
                        Make {name} host
                    </button>
                    <button type="button" className={BTN_GHOST} onClick={() => setConfirm(null)}>
                        Cancel
                    </button>
                </span>
            )}

            {confirm === null && (powers.kick || powers.makeCoHost || powers.transfer) && (
                <span className="flex items-center gap-1">
                    {(powers.makeCoHost || member.role === 'CO_HOST') && myRole === 'HOST' && (
                        <button type="button" disabled={busy} className={BTN_GHOST} onClick={() => role.mutate()}>
                            {member.role === 'CO_HOST' ? 'Remove co-host' : 'Make co-host'}
                        </button>
                    )}
                    {powers.transfer && (
                        <button type="button" disabled={busy} className={BTN_GHOST} onClick={() => setConfirm('transfer')}>
                            Make host
                        </button>
                    )}
                    {powers.kick && (
                        <button type="button" disabled={busy} className={BTN_GHOST} onClick={() => setConfirm('kick')}>
                            Kick
                        </button>
                    )}
                </span>
            )}
        </li>
    )
}
