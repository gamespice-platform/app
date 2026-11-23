import { useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { FiUserPlus } from 'react-icons/fi'

import { api } from '~/lib/api/client'
import { partyApi } from '~/lib/api/party'
import { messageOf } from '~/lib/ipc'
import CreatePartyDialog from './create-party-dialog'
import { BTN_GHOST, usePartyAction } from './party-ui'

/**
 * The Friends row's "Invite to party". In a party: invites that friend. In
 * none: opens the create dialog and invites them once it exists. The site
 * refuses an invite from a plain member, and says so; nothing is hidden.
 */
export default function InviteToPartyButton({
    userId,
    displayName,
}: {
    userId: string
    displayName: string
}) {
    const mine = useQuery({ queryKey: ['social', 'party'], queryFn: api.partyMine })
    const [creating, setCreating] = useState(false)
    const [note, setNote] = useState<string | null>(null)
    const invite = usePartyAction(
        (id: string) => partyApi.invite(id, [userId]),
        (made) => setNote(made.length ? 'Invited' : 'Could not invite')
    )

    const party = mine.data ?? null

    return (
        <>
            <button
                type="button"
                className={`${BTN_GHOST} !px-2 !py-1 !text-[11px]`}
                disabled={mine.isPending || invite.isPending}
                title={party ? `Invite ${displayName} to your party` : `Start a party with ${displayName}`}
                onClick={() => (party ? invite.mutate(party.id) : setCreating(true))}
            >
                <FiUserPlus className="size-3" aria-hidden />
                {note ?? 'Invite to party'}
            </button>
            {invite.error && (
                <span role="alert" className="text-[11px] text-danger">
                    {messageOf(invite.error)}
                </span>
            )}
            {creating && (
                <CreatePartyDialog
                    // The game is chosen in the dialog; the friend is invited after create.
                    inviteUserIds={[userId]}
                    onClose={() => setCreating(false)}
                />
            )}
        </>
    )
}
