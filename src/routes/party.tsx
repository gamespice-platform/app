import { useEffect, useState } from 'react'
import { Link, useLocation } from 'react-router-dom'
import { FiLogOut, FiPlus, FiSearch, FiUserPlus, FiUsers } from 'react-icons/fi'

import { LiveChat } from '~/components/chat/live-chat'
import CreatePartyDialog from '~/components/party/create-party-dialog'
import InviteFriendsDialog from '~/components/party/invite-friends-dialog'
import InvitesList from '~/components/party/invites-list'
import MembersPanel from '~/components/party/members-panel'
import ReadyPanel from '~/components/party/ready-panel'
import ReservePanel from '~/components/party/reserve-panel'
import StatsPanel from '~/components/party/stats-panel'
import {
    BTN_DANGER,
    BTN_GHOST,
    BTN_PRIMARY,
    ErrorLine,
    Panel,
    usePartyAction,
} from '~/components/party/party-ui'
import { LoadError } from '~/components/states'
import { partyApi } from '~/lib/api/party'
import type { AppPartyT } from '~/lib/api/contract'
import { useAuth } from '~/lib/auth/provider'
import { messageOf } from '~/lib/ipc'
import { PARTY_HEARTBEAT_MS, joinedMembers, myPartyRole, stageLabel, typeLabel } from '~/lib/party/logic'
import { usePartyInvites, usePartyMine, usePartyReady } from '~/lib/party/use-party'

/**
 * **Party** — the party you are in, the invites waiting for you, and a way to
 * start one. The same routes the website's party page uses, so a party made
 * here shows there and the other way round.
 */
export default function PartyRoute() {
    const { status } = useAuth()

    if (status !== 'signedIn')
        return (
            <div className="flex flex-col gap-3 p-3">
                <h1 className="text-lg font-bold">Party</h1>
                <div className="rounded-xl border border-border p-6 text-center">
                    <p className="text-sm text-muted">Sign in to join or start a party.</p>
                    <Link to="/account" className={`${BTN_PRIMARY} mt-3`}>
                        Sign in
                    </Link>
                </div>
            </div>
        )

    return <SignedIn />
}

function SignedIn() {
    const mine = usePartyMine()
    const invites = usePartyInvites()
    const [creating, setCreating] = useState(false)
    const party = mine.data ?? null
    // The create dialog hands a refused invite over here: the party stands, say so.
    const inviteFailure = (useLocation().state as { inviteFailure?: string } | null)?.inviteFailure

    const refresh = () => {
        void mine.refresh()
        void invites.refresh()
    }

    return (
        <div className="flex flex-col gap-4 p-3">
            <div className="flex flex-wrap items-center justify-between gap-2">
                <h1 className="flex items-center gap-2 text-lg font-bold">
                    <FiUsers className="size-4" aria-hidden />
                    Party
                </h1>
                <div className="flex items-center gap-2">
                    <Link to="/party/browse" className={BTN_GHOST}>
                        <FiSearch className="size-3.5" aria-hidden />
                        Find a party
                    </Link>
                    {!party && (
                        <button type="button" className={BTN_PRIMARY} onClick={() => setCreating(true)}>
                            <FiPlus className="size-3.5" aria-hidden />
                            Create party
                        </button>
                    )}
                </div>
            </div>

            {inviteFailure && (
                <p role="alert" className="rounded-lg border border-warning/50 p-2.5 text-xs text-warning">
                    Your party was created, but the invites were not sent: {inviteFailure}
                </p>
            )}

            {mine.error && !mine.data && <LoadError title="Could not load your party" detail={messageOf(mine.error)} onRetry={mine.refresh} />}

            <InvitesList invites={invites.data ?? []} onChanged={refresh} />

            {party ? (
                <CurrentParty party={party} onChanged={refresh} />
            ) : (
                !mine.loading &&
                !mine.error && (
                    <p className="text-xs text-muted">
                        You are not in a party. Create one, accept an invite, or find one to join.
                    </p>
                )
            )}

            <StatsPanel />

            {creating && <CreatePartyDialog onClose={() => setCreating(false)} />}
        </div>
    )
}

function CurrentParty({ party, onChanged }: { party: AppPartyT; onChanged: () => void }) {
    const { user } = useAuth()
    const ready = usePartyReady(party.id)
    const view = ready.data
    const [inviting, setInviting] = useState(false)
    const [confirmEnd, setConfirmEnd] = useState(false)

    // Keep the party alive while this screen is open. Deliberately not paused when
    // the window is hidden (unlike the polls): a minimised app must not let the
    // party time out under the people still in it.
    useEffect(() => {
        const beat = () => void partyApi.heartbeat(party.id).catch(() => undefined)

        beat()
        const t = setInterval(beat, PARTY_HEARTBEAT_MS)

        return () => clearInterval(t)
    }, [party.id])

    const changed = () => {
        onChanged()
        void ready.refresh()
    }
    const leave = usePartyAction(() => partyApi.leave(party.id), onChanged)
    const end = usePartyAction(() => partyApi.end(party.id), onChanged)

    // A private roster leaves the ready view without members, so the role comes
    // from the party payload by user id; only then is it guessed.
    const myRole = myPartyRole(user?.id, party.members, view?.members, view?.me.canManage)
    const myUserIds = new Set(
        (view?.members ?? []).filter((m) => m.isMe && m.userId).map((m) => m.userId as string)
    )
    const readyIds = new Set(
        (view?.members ?? []).filter((m) => m.ready && m.userId).map((m) => m.userId as string)
    )
    const canManage = view?.me.canManage ?? false
    const error = leave.error ?? end.error

    return (
        <div className="flex flex-col gap-4">
            <Panel
                title="Your party"
                action={
                    <div className="flex items-center gap-1.5">
                        {canManage && (
                            <button type="button" className={BTN_GHOST} onClick={() => setInviting(true)}>
                                <FiUserPlus className="size-3.5" aria-hidden />
                                Invite friends
                            </button>
                        )}
                        <button
                            type="button"
                            className={BTN_GHOST}
                            disabled={leave.isPending}
                            onClick={() => leave.mutate()}
                        >
                            <FiLogOut className="size-3.5" aria-hidden />
                            Leave
                        </button>
                        {myRole === 'HOST' &&
                            (confirmEnd ? (
                                <>
                                    <button
                                        type="button"
                                        className={BTN_DANGER}
                                        disabled={end.isPending}
                                        onClick={() => end.mutate()}
                                    >
                                        End for everyone
                                    </button>
                                    <button type="button" className={BTN_GHOST} onClick={() => setConfirmEnd(false)}>
                                        Keep
                                    </button>
                                </>
                            ) : (
                                <button type="button" className={BTN_DANGER} onClick={() => setConfirmEnd(true)}>
                                    End party
                                </button>
                            ))}
                    </div>
                }
            >
                <div className="flex flex-col gap-0.5 p-3">
                    <p className="text-sm font-semibold">{party.name ?? `Party ${party.id}`}</p>
                    <p className="text-[11px] text-muted">
                        {typeLabel(party.type)} · {stageLabel(party.stage)} · {party.users}
                        {party.maxUsers ? `/${party.maxUsers}` : ''} players
                        {party.mapName ? ` · ${party.mapName}` : ''}
                        {party.gameMode ? ` · ${party.gameMode}` : ''}
                    </p>
                    {party.serverId != null && (
                        <Link
                            to={`/view/server/${party.serverId}`}
                            className="text-[11px] text-accent hover:underline"
                        >
                            Open the server
                        </Link>
                    )}
                    {error && <ErrorLine>{messageOf(error)}</ErrorLine>}
                </div>
            </Panel>

            {ready.error && !view && <LoadError title="Could not load the ready check" detail={messageOf(ready.error)} onRetry={ready.refresh} />}

            {view && (
                <ReadyPanel
                    partyId={party.id}
                    view={view}
                    serverId={party.serverId}
                    onChanged={changed}
                />
            )}

            <MembersPanel
                partyId={party.id}
                members={party.members}
                myUserIds={myUserIds}
                myRole={myRole}
                readyUserIds={readyIds}
                onChanged={changed}
            />

            {canManage && party.serverId != null && (
                <ReservePanel partyId={party.id} serverId={party.serverId} />
            )}

            <section aria-label="Party chat" className="h-80 overflow-hidden rounded-xl border border-border bg-surface">
                <LiveChat target={{ scope: 'PARTY', partyId: party.id }} label="Party chat" className="h-full" />
            </section>

            {inviting && (
                <InviteFriendsDialog
                    partyId={party.id}
                    memberIds={joinedMembers(party.members).map((m) => m.userId)}
                    onClose={() => setInviting(false)}
                    onSent={changed}
                />
            )}
        </div>
    )
}
