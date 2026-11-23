import { useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { useNavigate } from 'react-router-dom'

import { api } from '~/lib/api/client'
import { partyApi } from '~/lib/api/party'
import { PartyTypeVals } from '~/lib/api/party-schemas'
import { messageOf } from '~/lib/ipc'
import { Select, Toggle } from '~/components/form'
import {
    BLANK_CREATE,
    buildCreateRequest,
    typeLabel,
    type CreateFormT,
} from '~/lib/party/logic'
import { BTN_GHOST, BTN_PRIMARY, ErrorLine, INPUT, Modal, usePartyAction } from './party-ui'

/**
 * Make a party. Opened from the party screen, from a server's page ("start a
 * party for this server") and from a friend ("start one with them"): the last
 * two pass `appId`/`serverId`/`inviteUserIds` so the form arrives filled in.
 */
export default function CreatePartyDialog({
    appId,
    appName,
    serverId,
    inviteUserIds,
    onClose,
}: {
    appId?: number
    appName?: string
    serverId?: number
    /** Invited straight after the party exists. */
    inviteUserIds?: string[]
    onClose: () => void
}) {
    const navigate = useNavigate()
    const [form, setForm] = useState<CreateFormT>({
        ...BLANK_CREATE,
        appId: appId ?? null,
        serverId: serverId ?? null,
    })
    const [error, setError] = useState<string | null>(null)
    const [search, setSearch] = useState('')
    const set = <K extends keyof CreateFormT>(key: K, value: CreateFormT[K]) =>
        setForm((f) => ({ ...f, [key]: value }))

    // Only when the game is not already decided by where the dialog was opened.
    const apps = useQuery({
        queryKey: ['apps', 'party-picker', search],
        queryFn: () => api.apps({ search: search || undefined, limit: 25 }),
        enabled: appId == null,
    })

    const create = usePartyAction(
        async () => {
            const built = buildCreateRequest(form)

            if (!built.ok) throw new Error(built.error)

            const party = await partyApi.create(built.request)

            // The party stands even if an invite is refused; say so rather than undo it.
            if (inviteUserIds?.length)
                await partyApi.invite(party.id, inviteUserIds).catch(() => undefined)

            return party
        },
        () => {
            onClose()
            void navigate('/party')
        }
    )

    const typeOptions = PartyTypeVals.map((t) => ({ value: t, label: typeLabel(t) }))

    return (
        <Modal title="Start a party" onClose={onClose}>
            <form
                className="mt-3 flex flex-col gap-3"
                onSubmit={(e) => {
                    e.preventDefault()
                    const built = buildCreateRequest(form)

                    if (!built.ok) return setError(built.error)

                    setError(null)
                    create.mutate()
                }}
            >
                {appId != null ? (
                    <p className="text-xs text-muted">
                        Game: <span className="text-foreground">{appName ?? `#${appId}`}</span>
                        {serverId != null ? ' · on this server' : ''}
                    </p>
                ) : (
                    <div className="flex flex-col gap-1.5">
                        <label className="text-xs font-medium" htmlFor="party-game-search">
                            Game
                        </label>
                        <input
                            id="party-game-search"
                            className={INPUT}
                            placeholder="Search for a game"
                            value={search}
                            onChange={(e) => setSearch(e.target.value)}
                        />
                        <Select
                            label="Game"
                            fullWidth
                            placeholder={apps.isPending ? 'Loading games…' : 'Choose a game'}
                            value={form.appId == null ? '' : String(form.appId)}
                            options={(apps.data?.apps ?? []).map((a) => ({
                                value: String(a.id),
                                label: a.name,
                            }))}
                            onChange={(v) => set('appId', Number(v))}
                        />
                        {apps.error && <ErrorLine>{messageOf(apps.error)}</ErrorLine>}
                    </div>
                )}

                <label className="flex flex-col gap-1 text-xs font-medium">
                    Name (optional)
                    <input
                        className={INPUT}
                        value={form.name}
                        onChange={(e) => set('name', e.target.value)}
                    />
                </label>

                <label className="flex flex-col gap-1 text-xs font-medium">
                    Description (optional)
                    <textarea
                        className={INPUT}
                        rows={2}
                        value={form.description}
                        onChange={(e) => set('description', e.target.value)}
                    />
                </label>

                <div className="flex flex-col gap-1 text-xs font-medium">
                    Who can join
                    <Select
                        label="Who can join"
                        fullWidth
                        value={form.type}
                        options={typeOptions}
                        onChange={(v) => set('type', v)}
                    />
                </div>

                {form.type === 'PUBLIC_PASSWORD' && (
                    <label className="flex flex-col gap-1 text-xs font-medium">
                        Password
                        <input
                            className={INPUT}
                            type="password"
                            autoComplete="off"
                            value={form.password}
                            onChange={(e) => set('password', e.target.value)}
                        />
                    </label>
                )}

                <label className="flex flex-col gap-1 text-xs font-medium">
                    Players (blank for the site&rsquo;s default, 0 for no limit)
                    <input
                        className={INPUT}
                        inputMode="numeric"
                        value={form.maxUsers}
                        onChange={(e) => set('maxUsers', e.target.value)}
                    />
                </label>

                <div className="flex flex-col gap-2 text-xs">
                    <span className="flex items-center justify-between gap-3">
                        Party chat
                        <Toggle
                            label="Party chat"
                            checked={form.chatEnabled}
                            onChange={(v) => set('chatEnabled', v)}
                        />
                    </span>
                    <span className="flex items-center justify-between gap-3">
                        Friends can join without an invite
                        <Toggle
                            label="Friends can join without an invite"
                            checked={form.friendsCanJoin}
                            onChange={(v) => set('friendsCanJoin', v)}
                        />
                    </span>
                    <span className="flex items-center justify-between gap-3">
                        Show who is in the party
                        <Toggle
                            label="Show who is in the party"
                            checked={form.showUsers}
                            onChange={(v) => set('showUsers', v)}
                        />
                    </span>
                </div>

                {(error ?? create.error) && (
                    <ErrorLine>{error ?? messageOf(create.error)}</ErrorLine>
                )}

                <div className="flex justify-end gap-2">
                    <button type="button" className={BTN_GHOST} onClick={onClose}>
                        Cancel
                    </button>
                    <button type="submit" className={BTN_PRIMARY} disabled={create.isPending}>
                        {create.isPending ? 'Creating…' : 'Create party'}
                    </button>
                </div>
            </form>
        </Modal>
    )
}
