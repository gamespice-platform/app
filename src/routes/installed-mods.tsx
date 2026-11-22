import { useMemo, useState } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { Link, useSearchParams } from 'react-router-dom'
import {
    FiAlertTriangle,
    FiArrowDownCircle,
    FiBox,
    FiExternalLink,
    FiFolder,
    FiPause,
    FiPlay,
    FiRefreshCw,
    FiSearch,
    FiTrash2,
    FiX,
} from 'react-icons/fi'

import { api } from '~/lib/api/client'
import { ipc } from '~/lib/ipc/commands'
import { messageOf } from '~/lib/ipc'
import { useLibrary } from '~/lib/library/provider'
import { useAuth } from '~/lib/auth/provider'
import { openExternally } from '~/lib/external'
import { formatBytes } from '~/lib/downloads/provider'
import { useAppIcons } from '~/lib/hooks/use-app-icons'
import {
    buildModRows,
    filterMods,
    modGames,
    MOD_SORTS,
    MOD_SOURCES,
    MOD_STATUSES,
    pick,
    type ModFilters,
    type ModRow,
    type ModSort,
    type ModSourceFilter,
    type ModStatus,
} from '~/lib/library/mods'
import Select from '~/components/select'
import { GameIcon } from '~/components/game-icon'
import { ItemThumb } from '~/components/item-thumb'

/**
 * **Library → Mods** (`/installed-mods`): every mod on this device, per game,
 * whatever put it there, and the controls to manage each one.
 *
 * The rows come from `lib/library/mods.ts` (subscriptions that are installed
 * or disabled, plus imported mods); the filters (game, status, source, sort,
 * search) are in the URL like every browse screen's.
 *
 * Disable is real, not a flag: it removes the files (`library_uninstall`) and
 * pauses the subscription so the next sync does not put them back; Enable
 * resumes it and installs again. The row stays listed while disabled so it can
 * be turned back on. An imported mod has no subscription, so it has Remove and
 * no Disable.
 */

const STATUS_LABELS: Record<ModStatus, string> = {
    all: 'Any status',
    enabled: 'Enabled',
    disabled: 'Disabled',
    updates: 'Has an update',
}

const SOURCE_LABELS: Record<ModSourceFilter, string> = {
    all: 'Any source',
    subscribed: 'Subscribed',
    local: 'Imported',
}

const SORT_LABELS: Record<ModSort, string> = {
    name: 'Name',
    game: 'Game',
    recent: 'Recently installed',
    size: 'Size',
}

const BTN =
    'flex items-center gap-1 rounded-lg border border-border px-2 py-1.5 text-xs text-muted hover:text-foreground disabled:opacity-50'

export default function InstalledModsRoute() {
    const { status } = useAuth()
    const { rows: subs, loading, error: syncError, sync } = useLibrary()
    const [params, setParams] = useSearchParams()
    const [typed, setTyped] = useState<string | null>(null)

    const locals = useQuery({
        queryKey: ['local-mods'],
        queryFn: () => ipc.localList(),
    })

    const gameRaw = Number(params.get('game'))
    const filters: ModFilters = {
        q: params.get('q') ?? '',
        game: Number.isInteger(gameRaw) && gameRaw > 0 ? gameRaw : null,
        status: pick(MOD_STATUSES, params.get('status'), 'all'),
        source: pick(MOD_SOURCES, params.get('source'), 'all'),
        sort: pick(MOD_SORTS, params.get('sort'), 'game'),
    }

    const set = (key: string, value: string | null) => {
        const next = new URLSearchParams(params)

        if (value === null || value === '') next.delete(key)
        else next.set(key, value)
        setParams(next, { replace: true })
    }

    const all = useMemo(() => buildModRows(subs, locals.data ?? []), [subs, locals.data])
    const games = useMemo(() => modGames(all), [all])
    const draft = typed ?? filters.q
    const shown = useMemo(
        () => filterMods(all, { ...filters, q: draft }),
        // eslint-disable-next-line react-hooks/exhaustive-deps -- the filters are the URL's primitives
        [all, draft, filters.game, filters.status, filters.source, filters.sort]
    )
    const updates = all.filter((r) => r.update).length

    return (
        <div className="flex flex-col gap-3 p-3">
            <header className="flex flex-wrap items-start justify-between gap-2">
                <div>
                    <h1 className="text-lg font-bold">Mods</h1>
                    <p className="text-xs text-muted">
                        Every mod installed on this device, for each game: the ones you subscribed to and the ones
                        you imported.
                    </p>
                </div>

                <button
                    type="button"
                    disabled={loading || status !== 'signedIn'}
                    onClick={() => void sync(true)}
                    className="flex items-center gap-2 rounded-lg border border-border px-3 py-1.5 text-xs disabled:opacity-50"
                >
                    <FiRefreshCw className={loading ? 'size-3.5 animate-spin' : 'size-3.5'} />
                    Check for updates
                </button>
            </header>

            {syncError || locals.error ? (
                <p className="flex items-start gap-2 rounded-lg border border-danger/40 bg-danger/10 p-2 text-xs text-danger">
                    <FiAlertTriangle className="mt-0.5 size-3.5 shrink-0" />
                    {syncError ?? messageOf(locals.error)}
                </p>
            ) : null}

            <div className="flex flex-wrap items-center gap-2">
                <div className="relative min-w-48 flex-1">
                    <FiSearch className="pointer-events-none absolute left-2.5 top-1/2 size-3.5 -translate-y-1/2 text-muted" />
                    <input
                        type="search"
                        aria-label="Search installed mods"
                        value={draft}
                        onChange={(e) => setTyped(e.target.value)}
                        onBlur={() => {
                            if (typed !== null) set('q', typed.trim() || null)
                            setTyped(null)
                        }}
                        onKeyDown={(e) => {
                            if (e.key === 'Enter') e.currentTarget.blur()
                        }}
                        placeholder="Search installed mods"
                        className="w-full rounded-lg border border-border bg-surface-2 py-1.5 pl-8 pr-8 text-xs"
                    />
                    {draft && (
                        <button
                            type="button"
                            onClick={() => {
                                setTyped(null)
                                set('q', null)
                            }}
                            aria-label="Clear the search"
                            className="absolute right-2 top-1/2 -translate-y-1/2 text-muted"
                        >
                            <FiX className="size-3.5" />
                        </button>
                    )}
                </div>

                <Select
                    label="Game"
                    value={filters.game === null ? '' : String(filters.game)}
                    options={[
                        { value: '', label: 'Every game' },
                        ...games.map((g) => ({ value: String(g.id), label: g.name })),
                    ]}
                    onChange={(v) => set('game', v)}
                />
                <Select
                    label="Status"
                    value={filters.status}
                    options={MOD_STATUSES.map((v) => ({
                        value: v,
                        label: v === 'updates' && updates ? `${STATUS_LABELS[v]} (${updates})` : STATUS_LABELS[v],
                    }))}
                    onChange={(v) => set('status', v === 'all' ? null : v)}
                />
                <Select
                    label="Source"
                    value={filters.source}
                    options={MOD_SOURCES.map((v) => ({ value: v, label: SOURCE_LABELS[v] }))}
                    onChange={(v) => set('source', v === 'all' ? null : v)}
                />
                <Select
                    label="Sort"
                    value={filters.sort}
                    options={MOD_SORTS.map((v) => ({ value: v, label: SORT_LABELS[v] }))}
                    onChange={(v) => set('sort', v === 'game' ? null : v)}
                />
            </div>

            <p className="text-[11px] text-muted">
                {shown.length} of {all.length} mods
            </p>

            <div className="rounded-xl border border-border">
                {shown.length === 0 ? (
                    <div className="flex flex-col items-center gap-2 p-8 text-center">
                        <FiBox className="size-6 text-muted" />
                        <p className="text-sm">
                            {all.length === 0 ? 'No mods installed on this device yet.' : 'Nothing matches these filters.'}
                        </p>
                        {all.length === 0 ? (
                            <p className="max-w-sm text-xs text-muted">
                                Subscribe to a mod from <Link to="/browse/mod" className="text-accent">Discover</Link>,
                                or import one into a <Link to="/sandboxes" className="text-accent">sandbox</Link>.
                            </p>
                        ) : null}
                    </div>
                ) : (
                    shown.map((row) => <ModLine key={row.key} row={row} onLocalChange={() => void locals.refetch()} />)
                )}
            </div>
        </div>
    )
}

function ModLine({ row, onLocalChange }: { row: ModRow; onLocalChange: () => void }) {
    const iconFor = useAppIcons()
    const { install, uninstall, busy, sync } = useLibrary()
    const client = useQueryClient()
    const [working, setWorking] = useState(false)
    const [error, setError] = useState<string | null>(null)

    const sub = row.sub
    const pending =
        working || (sub !== null && (busy.has(sub.id) || sub.state === 'installing' || sub.state === 'removing'))

    const run = async (task: () => Promise<unknown>) => {
        setWorking(true)
        setError(null)

        try {
            await task()
        } catch (e) {
            setError(messageOf(e))
        } finally {
            setWorking(false)
        }
    }

    const disable = () =>
        run(async () => {
            if (!sub) return
            if (sub.installedReleaseId !== null) await uninstall(sub.id)
            await api.subscriptionPrefs(sub.kind, sub.itemId, { paused: true })
            await sync()
        })

    const enable = () =>
        run(async () => {
            if (!sub) return
            await api.subscriptionPrefs(sub.kind, sub.itemId, { paused: false })
            await sync()
            await install(sub.id)
        })

    const removeLocal = () =>
        run(async () => {
            if (!row.local) return
            await ipc.localDelete(row.local.id)
            await client.invalidateQueries({ queryKey: ['local-mods'] })
            onLocalChange()
        })

    const reveal = () =>
        run(() => (sub ? ipc.libraryReveal(sub.id) : row.local ? ipc.localReveal(row.local.id) : Promise.resolve()))

    const detail = [
        row.source === 'subscribed' ? 'Subscribed' : (row.local?.originLabel ?? 'Imported'),
        row.version ? `v${row.version}` : null,
        row.bytes !== null ? formatBytes(row.bytes) : null,
    ].filter(Boolean)

    return (
        <div className="flex flex-wrap items-start gap-3 border-b border-border px-3 py-3 last:border-b-0">
            <ItemThumb image={sub?.image ?? null} kind={sub ? row.kind : 'local'} />

            <div className="min-w-0 flex-1">
                <div className="flex flex-wrap items-center gap-2">
                    {sub ? (
                        <Link
                            to={`/view/${sub.kind}/${sub.itemId}`}
                            className="truncate text-sm font-medium hover:text-accent"
                        >
                            {row.name}
                        </Link>
                    ) : (
                        <span className="truncate text-sm font-medium">{row.name}</span>
                    )}

                    {row.appName ? (
                        <span className="flex items-center gap-1.5 text-[11px] text-muted">
                            <GameIcon
                                app={{ id: row.appId ?? 0, name: row.appName, icon: iconFor(row.appId) }}
                                size="sm"
                            />
                            {row.appName}
                        </span>
                    ) : null}

                    <span
                        className={`rounded border px-1.5 py-0.5 text-[10px] uppercase tracking-wide ${
                            row.enabled ? 'border-success/40 text-success' : 'border-border text-muted'
                        }`}
                    >
                        {row.enabled ? 'Enabled' : 'Disabled'}
                    </span>

                    {row.update ? (
                        <span className="rounded border border-accent/40 px-1.5 py-0.5 text-[10px] uppercase tracking-wide text-accent">
                            Update {sub?.latestVersion ?? ''}
                        </span>
                    ) : null}
                </div>

                <p className="mt-0.5 text-xs text-muted">{detail.join(' · ')}</p>
                {sub?.state === 'failed' && sub.lastError ? (
                    <p className="mt-0.5 text-xs text-danger">{sub.lastError}</p>
                ) : null}
                {error ? (
                    <p role="alert" className="mt-0.5 text-xs text-danger">
                        {error}
                    </p>
                ) : null}
            </div>

            <div className="flex shrink-0 flex-wrap items-center gap-1">
                {sub && row.update && row.enabled ? (
                    <button type="button" disabled={pending} onClick={() => void install(sub.id)} className={BTN}>
                        <FiArrowDownCircle className="size-3.5" />
                        Update
                    </button>
                ) : null}

                {sub ? (
                    <button
                        type="button"
                        disabled={pending}
                        onClick={() => void (row.enabled ? disable() : enable())}
                        title={
                            row.enabled
                                ? 'Remove its files and stop installing it; stay subscribed'
                                : 'Install it again and keep it updated'
                        }
                        className={BTN}
                    >
                        {row.enabled ? <FiPause className="size-3.5" /> : <FiPlay className="size-3.5" />}
                        {row.enabled ? 'Disable' : 'Enable'}
                    </button>
                ) : null}

                {(sub?.installedReleaseId ?? null) !== null || row.local ? (
                    <button
                        type="button"
                        disabled={pending}
                        onClick={() => void reveal()}
                        aria-label="Open folder"
                        title="Open folder"
                        className={BTN}
                    >
                        <FiFolder className="size-3.5" />
                    </button>
                ) : null}

                {sub ? (
                    <button
                        type="button"
                        onClick={() => void openExternally(sub)}
                        aria-label="Open its workshop page"
                        title="Open its workshop page"
                        className={BTN}
                    >
                        <FiExternalLink className="size-3.5" />
                    </button>
                ) : null}

                {sub?.installedReleaseId != null ? (
                    <button
                        type="button"
                        disabled={pending}
                        onClick={() => void uninstall(sub.id)}
                        aria-label="Uninstall"
                        title="Uninstall from this device (stays subscribed; Sync installs it again)"
                        className={`${BTN} hover:text-danger`}
                    >
                        <FiTrash2 className="size-3.5" />
                    </button>
                ) : null}

                {row.local ? (
                    <button
                        type="button"
                        disabled={pending}
                        onClick={() => void removeLocal()}
                        aria-label="Remove"
                        title="Delete this imported mod from the app and every sandbox"
                        className={`${BTN} hover:text-danger`}
                    >
                        <FiTrash2 className="size-3.5" />
                    </button>
                ) : null}
            </div>
        </div>
    )
}
