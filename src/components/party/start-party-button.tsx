import { useState } from 'react'
import { FiUsers } from 'react-icons/fi'

import { useAuth } from '~/lib/auth/provider'
import CreatePartyDialog from './create-party-dialog'
import { BTN_GHOST } from './party-ui'

/**
 * "Start a party" for a server or a game: the entry point on a server's page
 * and in Play/Join. Renders nothing when signed out, because a party needs an
 * account and the Play button next to it already works without one.
 */
export default function StartPartyButton({
    appId,
    appName,
    serverId,
    className = BTN_GHOST,
}: {
    appId: number
    appName?: string
    serverId?: number
    className?: string
}) {
    const { status } = useAuth()
    const [open, setOpen] = useState(false)

    if (status !== 'signedIn') return null

    return (
        <>
            <button type="button" className={className} onClick={() => setOpen(true)}>
                <FiUsers className="size-3.5" aria-hidden />
                Start a party
            </button>
            {open && (
                <CreatePartyDialog
                    appId={appId}
                    appName={appName}
                    serverId={serverId}
                    onClose={() => setOpen(false)}
                />
            )}
        </>
    )
}
