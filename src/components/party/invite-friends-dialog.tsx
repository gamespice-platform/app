import { useState } from 'react'
import { useQuery } from '@tanstack/react-query'

import { api } from '~/lib/api/client'
import { partyApi } from '~/lib/api/party'
import { messageOf } from '~/lib/ipc'
import { invitableFriends } from '~/lib/party/logic'
import { BTN_GHOST, BTN_PRIMARY, ErrorLine, INPUT, Initial, Modal, usePartyAction } from './party-ui'

/** Invite friends who are not in the party yet. Skipped ones are reported, not hidden. */
export default function InviteFriendsDialog({
    partyId,
    memberIds,
    onClose,
    onSent,
}: {
    partyId: string
    memberIds: string[]
    onClose: () => void
    onSent?: () => void
}) {
    const friends = useQuery({ queryKey: ['social', 'friends'], queryFn: api.friends })
    const [picked, setPicked] = useState<Set<string>>(new Set())
    const [message, setMessage] = useState('')
    const [result, setResult] = useState<string | null>(null)

    const list = invitableFriends(friends.data ?? [], memberIds.map((userId) => ({ userId })))

    const send = usePartyAction(
        () => partyApi.invite(partyId, [...picked], message.trim() || undefined),
        (made) => {
            onSent?.()

            if (made.length >= picked.size) return onClose()

            setResult(
                `Invited ${made.length} of ${picked.size}. The rest could not be invited (already invited, banned, or not accepting).`
            )
        }
    )

    const toggle = (id: string) =>
        setPicked((p) => {
            const next = new Set(p)

            if (!next.delete(id)) next.add(id)

            return next
        })

    return (
        <Modal title="Invite friends" onClose={onClose}>
            <div className="mt-3 flex flex-col gap-3">
                {friends.isPending && <p className="text-xs text-muted">Loading friends…</p>}
                {friends.error && <ErrorLine>{messageOf(friends.error)}</ErrorLine>}
                {friends.data && list.length === 0 && (
                    <p className="text-xs text-muted">
                        No friends left to invite. Everyone you know is already in the party.
                    </p>
                )}

                <ul className="flex max-h-64 flex-col divide-y divide-border overflow-y-auto rounded-lg border border-border">
                    {list.map((f) => (
                        <li key={f.userId}>
                            <label className="flex cursor-pointer items-center gap-2.5 p-2 hover:bg-surface-hover">
                                <input
                                    type="checkbox"
                                    checked={picked.has(f.userId)}
                                    onChange={() => toggle(f.userId)}
                                />
                                <Initial name={f.displayName} />
                                <span className="min-w-0 flex-1 truncate text-sm">
                                    {f.displayName}
                                </span>
                                <span className="text-[11px] text-muted">{f.presence.status}</span>
                            </label>
                        </li>
                    ))}
                </ul>

                <label className="flex flex-col gap-1 text-xs font-medium">
                    Message (optional)
                    <input
                        className={INPUT}
                        value={message}
                        onChange={(e) => setMessage(e.target.value)}
                    />
                </label>

                {result && <p role="status" className="text-xs text-muted">{result}</p>}
                {send.error && <ErrorLine>{messageOf(send.error)}</ErrorLine>}

                <div className="flex justify-end gap-2">
                    <button type="button" className={BTN_GHOST} onClick={onClose}>
                        {result ? 'Close' : 'Cancel'}
                    </button>
                    <button
                        type="button"
                        className={BTN_PRIMARY}
                        disabled={picked.size === 0 || send.isPending}
                        onClick={() => send.mutate()}
                    >
                        {send.isPending ? 'Inviting…' : `Invite ${picked.size || ''}`.trim()}
                    </button>
                </div>
            </div>
        </Modal>
    )
}
