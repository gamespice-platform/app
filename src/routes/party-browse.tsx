import { useState } from 'react'
import { Link } from 'react-router-dom'
import { useInfiniteQuery, useQuery } from '@tanstack/react-query'

import { LoadError } from '~/components/states'
import {
    BTN_DANGER,
    BTN_GHOST,
    BTN_PRIMARY,
    ErrorLine,
    INPUT,
    Panel,
    UnavailableNote,
    usePartyAction,
} from '~/components/party/party-ui'
import { partyApi } from '~/lib/api/party'
import type { PartyBrowseResponseT } from '~/lib/api/party-schemas'
import { isUnavailable } from '~/lib/api/unavailable'
import { useAuth } from '~/lib/auth/provider'
import { messageOf } from '~/lib/ipc'
import { stageLabel, typeLabel } from '~/lib/party/logic'
import { usePartyMine } from '~/lib/party/use-party'

/**
 * **Find a party** — browse open parties, join one, queue your own party in the
 * matchmaker, and (for staff) moderate. Browse, search and moderation are
 * proposed routes: where the site has none, each shows a plain "not available
 * on the server yet" note instead of failing.
 */
export default function PartyBrowseRoute() {
    const { status } = useAuth()
    const [q, setQ] = useState('')
    const [applied, setApplied] = useState('')

    const browse = useInfiniteQuery({
        queryKey: ['party', 'browse', applied],
        queryFn: ({ pageParam }): Promise<PartyBrowseResponseT> =>
            partyApi.browse({ q: applied || undefined, cursor: pageParam }),
        initialPageParam: null as string | null,
        getNextPageParam: (last) => last.nextCursor,
        retry: false,
    })
    const rows = browse.data?.pages.flatMap((p) => p.items) ?? []
    const seen = new Set<string>()
    const unique = rows.filter((r) => !seen.has(r.id) && seen.add(r.id))

    return (
        <div className="flex flex-col gap-4 p-3">
            <div className="flex items-center justify-between gap-2">
                <h1 className="text-lg font-bold">Find a party</h1>
                <Link to="/party" className={BTN_GHOST}>
                    Your party
                </Link>
            </div>

            {status === 'signedIn' && <SearchPanel />}

            <Panel title="Open parties">
                <form
                    className="flex gap-2 p-3"
                    role="search"
                    onSubmit={(e) => {
                        e.preventDefault()
                        setApplied(q.trim())
                    }}
                >
                    <input
                        className={INPUT}
                        value={q}
                        onChange={(e) => setQ(e.target.value)}
                        placeholder="Search by name or game"
                        aria-label="Search parties"
                    />
                    <button type="submit" className={BTN_PRIMARY}>
                        Search
                    </button>
                </form>

                {browse.error ? (
                    isUnavailable(browse.error) ? (
                        <UnavailableNote what="Browsing parties" />
                    ) : (
                        <LoadError title="Could not load parties" detail={messageOf(browse.error)} onRetry={() => void browse.refetch()} />
                    )
                ) : browse.isPending ? (
                    <p className="p-3 text-xs text-muted">Loading…</p>
                ) : unique.length === 0 ? (
                    <p className="p-3 text-xs text-muted">No open parties right now.</p>
                ) : (
                    <ul className="flex flex-col divide-y divide-border border-t border-border">
                        {unique.map((r) => (
                            <BrowseRow key={r.id} row={r} signedIn={status === 'signedIn'} />
                        ))}
                    </ul>
                )}

                {browse.hasNextPage && (
                    <div className="border-t border-border p-2 text-center">
                        <button
                            type="button"
                            className={BTN_GHOST}
                            disabled={browse.isFetchingNextPage}
                            onClick={() => void browse.fetchNextPage()}
                        >
                            Load more
                        </button>
                    </div>
                )}
            </Panel>

            {status === 'signedIn' && <ModerationPanel />}
        </div>
    )
}

function BrowseRow({
    row,
    signedIn,
}: {
    row: PartyBrowseResponseT['items'][number]
    signedIn: boolean
}) {
    const [password, setPassword] = useState('')
    const join = usePartyAction(() => partyApi.join({ id: row.id, password: password || undefined }))
    const needsPassword = row.type === 'PUBLIC_PASSWORD'

    return (
        <li className="flex flex-wrap items-center gap-2 p-2.5">
            <div className="min-w-0 flex-1">
                <p className="truncate text-sm font-medium">{row.name ?? `Party ${row.id}`}</p>
                <p className="truncate text-[11px] text-muted">
                    {row.appName ?? `App ${row.appId}`} · {typeLabel(row.type)} · {stageLabel(row.stage)} ·{' '}
                    {row.users}
                    {row.maxUsers ? `/${row.maxUsers}` : ''} players
                    {row.hostName ? ` · host ${row.hostName}` : ''}
                    {row.mapName ? ` · ${row.mapName}` : ''}
                </p>
                {join.error && <ErrorLine>{messageOf(join.error)}</ErrorLine>}
                {join.isSuccess && (
                    <p className="text-[11px] text-success">
                        Joined. <Link to="/party" className="underline">Open your party</Link>
                    </p>
                )}
            </div>
            {needsPassword && (
                <input
                    className={`${INPUT} w-32`}
                    type="password"
                    value={password}
                    onChange={(e) => setPassword(e.target.value)}
                    placeholder="Password"
                    aria-label={`Password for ${row.name ?? row.id}`}
                />
            )}
            {signedIn && (
                <button type="button" className={BTN_PRIMARY} disabled={join.isPending} onClick={() => join.mutate()}>
                    Join
                </button>
            )}
        </li>
    )
}

/** Matchmaking for your own party: queue, watch, cancel. */
function SearchPanel() {
    const mine = usePartyMine()
    const party = mine.data ?? null
    const state = useQuery({
        queryKey: ['party', 'search', party?.id],
        queryFn: () => partyApi.searchState(party?.id ?? ''),
        enabled: party != null,
        refetchInterval: (q) => (q.state.data ? 3000 : false),
        retry: false,
    })
    const start = usePartyAction(() => partyApi.searchStart(party?.id ?? ''), () => void state.refetch())
    const cancel = usePartyAction(() => partyApi.searchCancel(party?.id ?? ''), () => void state.refetch())
    const err = start.error ?? cancel.error ?? state.error
    const unavailable = err != null && isUnavailable(err)
    const searching = state.data != null

    return (
        <Panel title="Matchmaking">
            <div className="flex flex-col gap-2 p-3 text-xs">
                {!party ? (
                    <p className="text-muted">
                        Be in a party to search for a match. <Link to="/party" className="text-accent underline">Your party</Link>
                    </p>
                ) : unavailable ? (
                    <UnavailableNote what="Matchmaking" />
                ) : (
                    <div className="flex flex-wrap items-center gap-2">
                        <span className="min-w-0 flex-1 text-muted">
                            {searching ? `Searching (${state.data?.state.toLowerCase()})…` : 'Not searching.'}
                        </span>
                        {searching ? (
                            <button type="button" className={BTN_DANGER} disabled={cancel.isPending} onClick={() => cancel.mutate()}>
                                Stop searching
                            </button>
                        ) : (
                            <button type="button" className={BTN_PRIMARY} disabled={start.isPending} onClick={() => start.mutate()}>
                                Search for a match
                            </button>
                        )}
                    </div>
                )}
                {err != null && !unavailable && <ErrorLine>{messageOf(err)}</ErrorLine>}
            </div>
        </Panel>
    )
}

/** Staff tools for a party by id. A member's token is refused by the site, which we show as is. */
function ModerationPanel() {
    const [id, setId] = useState('')
    const [hidden, setHidden] = useState(false)
    const [nsfw, setNsfw] = useState(false)
    const [chatLocked, setChatLocked] = useState(false)
    const act = usePartyAction((end: boolean) =>
        partyApi.moderate({ id: id.trim(), hidden, nsfw, chatLocked, end: end || undefined })
    )
    const unavailable = act.error != null && isUnavailable(act.error)
    const valid = /^\d+$/.test(id.trim())

    return (
        <Panel title="Moderation (staff)">
            <div className="flex flex-col gap-2 p-3 text-xs">
                {unavailable ? (
                    <UnavailableNote what="Party moderation" />
                ) : (
                    <>
                        <label className="flex flex-col gap-1 font-medium">
                            Party id
                            <input
                                className={`${INPUT} w-40`}
                                inputMode="numeric"
                                value={id}
                                onChange={(e) => setId(e.target.value)}
                            />
                        </label>
                        <div className="flex flex-wrap gap-4">
                            {(
                                [
                                    ['Hidden', hidden, setHidden],
                                    ['NSFW', nsfw, setNsfw],
                                    ['Chat locked', chatLocked, setChatLocked],
                                ] as const
                            ).map(([label, value, set]) => (
                                <label key={label} className="flex items-center gap-1.5">
                                    <input type="checkbox" checked={value} onChange={(e) => set(e.target.checked)} />
                                    {label}
                                </label>
                            ))}
                        </div>
                        <div className="flex gap-2">
                            <button type="button" className={BTN_PRIMARY} disabled={!valid || act.isPending} onClick={() => act.mutate(false)}>
                                Apply flags
                            </button>
                            <button type="button" className={BTN_DANGER} disabled={!valid || act.isPending} onClick={() => act.mutate(true)}>
                                End party
                            </button>
                        </div>
                        {act.isSuccess && <p className="text-success">Done.</p>}
                        {act.error != null && <ErrorLine>{messageOf(act.error)}</ErrorLine>}
                    </>
                )}
            </div>
        </Panel>
    )
}
