import { useEffect, useMemo, useState, type KeyboardEvent as ReactKeyboardEvent } from 'react'
import { useInfiniteQuery, useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useSearchParams } from 'react-router-dom'
import { FiAlertTriangle, FiCpu, FiGlobe, FiSearch, FiX } from 'react-icons/fi'

import { api } from '~/lib/api/client'
import { ipc } from '~/lib/ipc/commands'
import { messageOf } from '~/lib/ipc'
import { AppSortVals, AppTypeVals, type AppSortT, type AppSummaryT } from '~/lib/api/contract'
import { DEFAULT_FILTERS, pageQuery } from '~/lib/api/play'
import { mergeGames } from '~/lib/gamespice/catalogue'
import {
    CATALOG_TABS,
    CATALOG_TAB_LABELS,
    PLATFORM_SLUGS,
    buildEntries,
    countTabs,
    matchesQuery,
    matchesTab,
    tabFromParam,
    type CatalogTab,
} from '~/lib/catalog/filter'
import { toggleStoredManaged, useManaged } from '~/lib/catalog/managed'
import { useAppIcons } from '~/lib/hooks/use-app-icons'
import { SkeletonGrid } from '~/components/skeleton'
import Select from '~/components/select'
import PlayDialog, { type PlayTargetT } from '~/components/play-dialog'
import { LoadError } from '~/components/states'
import { PartyBar, QuickMatchDialog, type JoinTarget } from '~/components/play-center'
import {
    CatalogCard,
    TYPE_LABELS,
    type CatalogHandlers,
    type Entry,
    type InstallState,
} from '~/components/catalog-card'
import { CatalogDrawer } from '~/components/catalog-drawer'

/**
 * The Library's catalogue of every game, engine and platform TMC tracks, and
 * what can be done with each one from this device. Mounted as the Library
 * page's Games view (`/library`); it replaced the Play page and the old Games
 * and GameSpice Games views: GameSpice's own games lead with the website's live stats, and
 * every card can Quick play, Play solo, install, manage, open a sandbox or the
 * workshop.
 *
 * DATA
 * ----
 * - `/apps`, every page (so the tabs count the whole catalogue), narrowed by the
 *   search, type, playable and moddable filters server-side.
 * - `/apps?slugs=` for GameSpice's own games, so they are always present and
 *   first whatever the sort or page.
 * - `/play/page` (games tab, every page): live players, servers, 24h history,
 *   solo; joined by app id. Its first page carries the party bar.
 * - Device: installed native builds (`games_list`), sandboxes (`sandbox_list`)
 *   and the user's managed choice (localStorage, see `~/lib/catalog/managed`).
 *
 * Every filter, the tab included, lives in the URL.
 *
 * WHAT THE CARD DOES NOT DECIDE: whether a launch is permitted. `directPlay`
 * gates the serverless case only and `/play/launch` enforces it however the
 * button was drawn.
 */

const SORT_LABELS: Record<AppSortT, string> = {
    players: 'Players online',
    servers: 'Servers',
    content: 'Mods and assets',
    name: 'Name',
}

export default function Catalog() {
    const [params, setParams] = useSearchParams()
    const [target, setTarget] = useState<PlayTargetT | null>(null)
    const [detailId, setDetailId] = useState<number | null>(null)
    const qc = useQueryClient()

    const search = params.get('q') ?? ''
    // The box shows what is being typed at once (`typed`, null when idle); the URL
    // (and so the query) follows 250ms after typing stops.
    const [typed, setTyped] = useState<string | null>(null)
    const draft = typed ?? search
    // A hand-edited or stale link can carry a value the API would reject, so
    // anything outside the known sets falls back to the default.
    const typeRaw = params.get('type')
    const type = AppTypeVals.find((v) => v === typeRaw) ?? null
    const sortRaw = params.get('sort')
    const sort: AppSortT = AppSortVals.find((v) => v === sortRaw) ?? 'players'
    const playable = params.get('playable') === '1'
    // The API's `managed` filter: the game has the TMC_APP_MANAGE integration.
    // Not the user's Managed tab, hence its own name here.
    const moddable = params.get('moddable') === '1'
    const tab = tabFromParam(params.get('tab'))

    const set = (key: string, value: string | null) => {
        const next = new URLSearchParams(params)

        if (value === null || value === '') next.delete(key)
        else next.set(key, value)

        setParams(next, { replace: true })
    }

    /*
     * `gs://play/app/<slug>` lands here with `?app=&start=1`.
     *
     * The link opens the launch dialog and nothing else: a link may ask the app
     * to SHOW something, and a dialog with a Play button in it is a thing shown.
     * Resolved by a lookup rather than by searching the loaded page.
     */
    const startRef = params.get('start') === '1' ? params.get('app') : null

    const startLookup = useQuery({
        queryKey: ['apps', { start: startRef }],
        queryFn: () =>
            api.apps(
                /^\d+$/.test(startRef ?? '')
                    ? { ids: [Number(startRef)], limit: 1 }
                    : { slugs: [startRef ?? ''], limit: 1 }
            ),
        enabled: Boolean(startRef),
        staleTime: 5 * 60 * 1000,
    })

    // Open the dialog when the lookup resolves, once per result (React's
    // "adjust state when something changes").
    const startApp = startLookup.data?.apps[0]
    const [startSeen, setStartSeen] = useState<typeof startLookup.data>(undefined)

    if (startApp && startSeen !== startLookup.data) {
        const solo = params.get('solo')

        setStartSeen(startLookup.data)
        setTarget({
            appId: startApp.id,
            app: startApp,
            solo: solo ? { contentId: solo, version: params.get('soloVersion') ?? undefined } : undefined,
        })
    }

    useEffect(() => {
        if (!startApp) return

        // Cleared so a reload, or closing the dialog and pressing back, does
        // not reopen it.
        const next = new URLSearchParams(params)

        next.delete('start')
        next.delete('app')
        next.delete('solo')
        next.delete('soloVersion')

        setParams(next, { replace: true })
        // Runs when the lookup resolves only; including params re-runs it on
        // the very change it makes.
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [startLookup.data])

    useEffect(() => {
        if (typed === null) return
        const t = setTimeout(() => {
            setParams(
                (prev) => {
                    const next = new URLSearchParams(prev)
                    if (typed) next.set('q', typed)
                    else next.delete('q')
                    return next
                },
                { replace: true }
            )
            setTyped(null)
        }, 250)
        return () => clearTimeout(t)
    }, [typed, setParams])

    // ------------------------------------------------------------ Data
    const filters = useMemo(
        () => ({
            search: search || undefined,
            type: type ?? undefined,
            sort,
            playable: playable || undefined,
            managed: moddable || undefined,
            limit: 100,
        }),
        [search, type, sort, playable, moddable]
    )

    const listing = useInfiniteQuery({
        queryKey: ['apps', filters],
        initialPageParam: null as string | null,
        queryFn: ({ pageParam }) => api.apps({ ...filters, cursor: pageParam ?? undefined }),
        getNextPageParam: (last) => last.nextCursor,
        staleTime: 5 * 60 * 1000,
    })

    // The tabs and their counts describe the whole catalogue, so read it all.
    useReadAll(listing)

    const pinnedQuery = useQuery({
        queryKey: ['apps', 'gamespice', filters],
        queryFn: () => api.apps({ ...filters, search: undefined, slugs: [...PLATFORM_SLUGS], limit: 100 }),
        staleTime: 5 * 60 * 1000,
    })

    const playListing = useInfiniteQuery({
        queryKey: ['play', 'page', DEFAULT_FILTERS],
        initialPageParam: 0,
        queryFn: ({ pageParam }) => api.playPage(pageQuery(DEFAULT_FILTERS, pageParam)),
        getNextPageParam: (last) => last?.nextCursor ?? undefined,
        staleTime: 15_000,
    })

    useReadAll(playListing)

    const playable$ = useQuery({
        queryKey: ['play', 'playable'],
        queryFn: api.playPlayable,
        staleTime: 5 * 60_000,
    })

    const platform = useQuery({
        queryKey: ['games', 'platform'],
        queryFn: () => ipc.gamesPlatform(),
        staleTime: Infinity,
    })

    const installedGames = useQuery({
        queryKey: ['games', 'installed'],
        queryFn: () => ipc.gamesList(),
    })

    const sandboxes = useQuery({
        queryKey: ['sandboxes', 'list'],
        queryFn: () => ipc.sandboxList(),
        retry: false,
    })

    const managedIds = useManaged()

    const playPages = playListing.data?.pages
    const firstPlay = playPages?.[0]
    const playRows = useMemo(() => mergeGames(playPages?.flatMap((p) => p?.apps ?? []) ?? []), [playPages])

    const installedIds = useMemo(() => {
        const ids = new Set<number>()

        for (const g of installedGames.data ?? []) ids.add(g.appId)
        for (const s of sandboxes.data ?? []) ids.add(s.appId)

        return ids
    }, [installedGames.data, sandboxes.data])

    const entries: Entry[] = useMemo(() => {
        const pinned = (pinnedQuery.data?.apps ?? []).filter((a) => matchesQuery(a, search, type))
        const apps = listing.data?.pages.flatMap((p) => p.apps) ?? []

        return buildEntries<AppSummaryT, (typeof playRows)[number]>(pinned, apps, playRows, managedIds, installedIds)
    }, [pinnedQuery.data, listing.data, playRows, managedIds, installedIds, search, type])

    const counts = useMemo(() => countTabs(entries), [entries])
    const shown = useMemo(() => entries.filter((e) => matchesTab(e, tab)), [entries, tab])

    const listed = useMemo(
        () => (playable$.data?.enabled ? new Set(playable$.data.appIds) : null),
        [playable$.data]
    )
    const playEnabled = listing.data?.pages[0]?.playEnabled !== false

    // ------------------------------------------------------------ Install
    const [busyId, setBusyId] = useState<number | null>(null)
    const [actionError, setActionError] = useState<string | null>(null)

    const installState = (entry: Entry): InstallState => {
        if (!entry.gamespice) return null

        const target = platform.data?.installable ? platform.data.platform : null
        const have = installedGames.data?.find((g) => g.appId === entry.app.id)

        return {
            available: target !== null && Boolean(entry.app.install?.builds.some((b) => b.platform === target)),
            version: have?.version ?? null,
            busy: busyId === entry.app.id,
        }
    }

    const act = async (appId: number, fn: () => Promise<unknown>) => {
        setBusyId(appId)
        setActionError(null)

        try {
            await fn()
        } catch (err) {
            setActionError(messageOf(err))
        } finally {
            setBusyId(null)
            await qc.invalidateQueries({ queryKey: ['games', 'installed'] })
        }
    }

    // ------------------------------------------------------------ Launching
    const [quick, setQuick] = useState<{ id: number; name: string } | null>(null)
    const [finding, setFinding] = useState(false)
    const facetIcon = useAppIcons()
    const iconOf = (id: number) => entries.find((e) => e.app.id === id)?.app.images.icon ?? facetIcon(id)

    const join = useMutation({
        mutationFn: async (t: JoinTarget) => {
            // The launcher wants the server's full row; the page row is thin.
            const detail = await api.content('server', String(t.serverId))

            return { t, summary: detail.summary }
        },
        onSuccess: ({ t, summary }) => {
            setQuick(null)
            setTarget({
                quickStart: t.quick === true,
                appId: t.appId,
                server: summary,
                fallback: { name: t.appName, icon: iconOf(t.appId), slug: null },
            })
        },
    })

    const handlers: CatalogHandlers = {
        /*
         * Left click: find the best server and start it natively, with the web
         * build as the warned fallback. Right click, or a search that finds
         * nothing: the chooser, which is where "no open server" is explained.
         */
        onQuick: ({ app }, advanced) => {
            const asDialog = () => setQuick({ id: app.id, name: app.name })

            if (advanced) return asDialog()

            setFinding(true)
            api.playQuickMatch(app.id, false)
                .then((res) => {
                    if (res?.best)
                        join.mutate({ serverId: res.best.id, appId: app.id, appName: res.appName ?? app.name, quick: true })
                    else asDialog()
                })
                .catch(asDialog)
                .finally(() => setFinding(false))
        },
        onPlay: ({ app }, advanced) => setTarget({ quickStart: !advanced, appId: app.id, app }),
        onManage: ({ app, gamespice }) => {
            if (!gamespice) toggleStoredManaged(app.id)
        },
        onInstall: ({ app }) =>
            void act(app.id, () => ipc.gameInstall({ appId: app.id, name: app.name, slug: app.slug ?? undefined })),
        onUninstall: ({ app }) => void act(app.id, () => ipc.gameUninstall(app.id)),
        onDetails: ({ app }) => setDetailId(app.id),
    }

    const detail = detailId === null ? null : (entries.find((e) => e.app.id === detailId) ?? null)
    const loading = listing.isPending && pinnedQuery.isPending

    return (
        <div className="flex flex-col">
            <div className="flex flex-col gap-3 border-b border-border pb-3">
                <div className="flex flex-wrap items-center gap-2">
                    <div className="relative min-w-48 flex-1">
                        <FiSearch className="pointer-events-none absolute left-2.5 top-1/2 size-3.5 -translate-y-1/2 text-muted" />
                        <input
                            type="search"
                            aria-label="Search games and apps"
                            value={draft}
                            onChange={(e) => setTyped(e.target.value)}
                            placeholder="Search games and apps"
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
                        label="Type"
                        value={type ?? ''}
                        options={[
                            { value: '', label: 'Any type' },
                            ...AppTypeVals.map((v) => ({ value: v, label: TYPE_LABELS[v] })),
                        ]}
                        onChange={(next) => set('type', next)}
                    />
                    <Select
                        label="Sort"
                        value={sort}
                        options={AppSortVals.map((v) => ({ value: v, label: SORT_LABELS[v] }))}
                        onChange={(next) => set('sort', next)}
                    />
                    <Chip
                        active={playable}
                        onClick={() => set('playable', playable ? null : '1')}
                        label="Playable"
                        icon={FiGlobe}
                    />
                    <Chip
                        active={moddable}
                        onClick={() => set('moddable', moddable ? null : '1')}
                        label="Moddable here"
                        icon={FiCpu}
                    />
                </div>

                <Tabs tab={tab} counts={counts} onChange={(t) => set('tab', t === 'all' ? null : t)} />

                {!playEnabled && (
                    <p className="rounded-lg border border-border bg-surface-2 px-3 py-2 text-[11px] text-muted">
                        Playing games from the app is switched off site-wide at the moment, so no Play buttons are
                        shown. Everything else on this screen still works.
                    </p>
                )}
            </div>

            <div
                id="catalog-panel"
                role="tabpanel"
                aria-labelledby={`catalog-tab-${tab}`}
                className="pt-3"
            >
                <div className="flex flex-col gap-4">
                    {firstPlay?.party && (
                        <PartyBar party={firstPlay.party} busy={join.isPending} onJoin={(t) => join.mutate(t)} />
                    )}

                    {(join.isError || actionError) && (
                        <p
                            role="alert"
                            className="flex items-center gap-2 rounded-lg border border-danger/40 bg-danger/10 px-3 py-2 text-xs text-danger"
                        >
                            <FiAlertTriangle className="size-3.5 shrink-0" aria-hidden />
                            {actionError ?? messageOf(join.error)}
                        </p>
                    )}

                    {loading ? (
                        <SkeletonGrid count={9} />
                    ) : listing.isError && entries.length < 1 ? (
                        <LoadError
                            title="The catalogue could not be loaded."
                            detail={messageOf(listing.error)}
                            onRetry={() => void listing.refetch()}
                            retrying={listing.isFetching}
                        />
                    ) : shown.length < 1 ? (
                        <p className="text-xs text-muted">
                            {tab === 'managed'
                                ? 'Nothing managed yet. Press Manage on a card to keep it here.'
                                : tab === 'installed'
                                  ? 'Nothing installed on this device yet.'
                                  : 'Nothing matched. Try a broader filter.'}
                        </p>
                    ) : (
                        <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4">
                            {shown.map((entry) => (
                                <CatalogCard
                                    key={entry.app.id}
                                    entry={entry}
                                    listed={listed ? listed.has(entry.app.id) : null}
                                    playEnabled={playEnabled}
                                    install={installState(entry)}
                                    handlers={handlers}
                                />
                            ))}
                        </div>
                    )}

                    {listing.isFetchingNextPage && <p className="text-center text-[11px] text-muted">Loading more…</p>}
                </div>
            </div>

            {detail && (
                <CatalogDrawer
                    entry={detail}
                    listed={listed ? listed.has(detail.app.id) : null}
                    playEnabled={playEnabled}
                    install={installState(detail)}
                    handlers={handlers}
                    onClose={() => setDetailId(null)}
                />
            )}

            {quick && (
                <QuickMatchDialog
                    app={quick}
                    busy={join.isPending}
                    onJoin={(t) => join.mutate(t)}
                    onClose={() => setQuick(null)}
                />
            )}

            {finding && (
                <div
                    role="status"
                    className="fixed bottom-4 right-4 z-50 rounded-lg border border-border bg-surface px-3 py-2 text-xs shadow-lg"
                >
                    Finding the best server…
                </div>
            )}

            {target && <PlayDialog target={target} onClose={() => setTarget(null)} />}
        </div>
    )
}

/** Fetch every page of an infinite query, one after another. */
function useReadAll({
    hasNextPage,
    isFetchingNextPage,
    isError,
    fetchNextPage,
}: {
    hasNextPage: boolean
    isFetchingNextPage: boolean
    isError: boolean
    fetchNextPage: () => Promise<unknown>
}) {
    useEffect(() => {
        if (hasNextPage && !isFetchingNextPage && !isError) void fetchNextPage()
    }, [hasNextPage, isFetchingNextPage, isError, fetchNextPage])
}

/** WAI-ARIA tabs: arrows, Home and End move between them, selection follows focus. */
function Tabs({
    tab,
    counts,
    onChange,
}: {
    tab: CatalogTab
    counts: Record<CatalogTab, number>
    onChange: (tab: CatalogTab) => void
}) {
    const onKey = (e: ReactKeyboardEvent<HTMLDivElement>) => {
        const at = CATALOG_TABS.indexOf(tab)
        const n = CATALOG_TABS.length
        const next =
            e.key === 'ArrowRight'
                ? CATALOG_TABS[(at + 1) % n]
                : e.key === 'ArrowLeft'
                  ? CATALOG_TABS[(at - 1 + n) % n]
                  : e.key === 'Home'
                    ? CATALOG_TABS[0]
                    : e.key === 'End'
                      ? CATALOG_TABS[n - 1]
                      : undefined

        if (!next) return

        e.preventDefault()
        onChange(next)
        document.getElementById(`catalog-tab-${next}`)?.focus()
    }

    return (
        <div role="tablist" aria-label="Catalogue" onKeyDown={onKey} className="flex flex-wrap gap-1">
            {CATALOG_TABS.map((t) => {
                const active = t === tab

                return (
                    <button
                        key={t}
                        id={`catalog-tab-${t}`}
                        type="button"
                        role="tab"
                        aria-selected={active}
                        aria-controls="catalog-panel"
                        tabIndex={active ? 0 : -1}
                        onClick={() => onChange(t)}
                        className={`flex items-center gap-1.5 rounded-lg px-3 py-1.5 text-xs font-medium transition focus-visible:outline focus-visible:outline-2 focus-visible:outline-accent ${
                            active ? 'bg-accent text-accent-foreground' : 'text-muted hover:bg-surface-2 hover:text-foreground'
                        }`}
                    >
                        {CATALOG_TAB_LABELS[t]}
                        <span
                            className={`rounded-full px-1.5 text-[10px] ${active ? 'bg-black/20' : 'bg-surface-2'}`}
                        >
                            {counts[t]}
                        </span>
                    </button>
                )
            })}
        </div>
    )
}

function Chip({
    active,
    onClick,
    label,
    icon: Icon,
}: {
    active: boolean
    onClick: () => void
    label: string
    icon?: typeof FiGlobe
}) {
    return (
        <button
            type="button"
            onClick={onClick}
            aria-pressed={active}
            className={`flex items-center gap-1 rounded-full border px-2.5 py-1 text-[11px] transition ${
                active ? 'border-accent bg-accent/10 text-accent' : 'border-border text-muted'
            }`}
        >
            {Icon && <Icon className="size-2.5" />}
            {label}
        </button>
    )
}
