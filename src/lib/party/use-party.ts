import { useCallback } from 'react'

import { partyApi } from '~/lib/api/party'
import { usePoll, type PollStateT } from '~/lib/hooks/use-poll'
import { PARTY_POLL_MS, PARTY_READY_POLL_MS } from './poll-intervals'
import type { AppPartyInviteT, AppPartyT } from '~/lib/api/contract'
import type { PartyReadyViewT } from '~/lib/api/party-schemas'

/**
 * Live party state. Each hook is one `usePoll`, so they share its behaviour:
 * paused while the window is hidden, backed off on errors, `refresh()` after a
 * mutation so the screen does not wait a full interval to show what you did.
 */

/** The caller's live party, or `null` data when they are in none. */
export function usePartyMine(enabled = true): PollStateT<AppPartyT | null> {
    return usePoll(() => partyApi.mine(), { intervalMs: PARTY_POLL_MS, enabled, key: 'mine' })
}

export function usePartyInvites(enabled = true): PollStateT<AppPartyInviteT[]> {
    return usePoll(() => partyApi.invites(), {
        intervalMs: PARTY_POLL_MS,
        enabled,
        key: 'invites',
    })
}

/** The ready panel for `partyId`. Stops being useful once `data.started`. */
export function usePartyReady(partyId: string | null): PollStateT<PartyReadyViewT> {
    const fetcher = useCallback(() => partyApi.readyState(partyId ?? ''), [partyId])

    return usePoll(fetcher, {
        intervalMs: PARTY_READY_POLL_MS,
        enabled: partyId != null,
        key: `ready:${partyId ?? ''}`,
    })
}
