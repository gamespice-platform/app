import { useMemo, useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { Link } from 'react-router-dom'
import {
    FiAlertTriangle,
    FiCheck,
    FiFolder,
    FiSearch,
    FiSettings,
    FiX,
} from 'react-icons/fi'

import { api } from '~/lib/api/client'
import { appLabel } from '~/lib/api/labels'
import { ipc } from '~/lib/ipc/commands'
import { isIpcError, messageOf } from '~/lib/ipc'
import type { DetectedGameT } from '~/lib/ipc/schemas'
import { useSettings } from '~/lib/settings/provider'
import {
    buildManageRows,
    unmatchedCount,
    type ManageRow,
} from '~/lib/app-manage/rows'
import { GameIcon } from '~/components/game-icon'
import { Skeleton } from '~/components/skeleton'
import { useFolderPicker } from '~/components/folder-picker'
import { useDialog } from '~/components/use-dialog'

/**
 * **Library -> Apps**: the games on this device and where each one lives.
 *
 * This is where the per-game settings used to be, in Settings -> Games. They
 * moved because "where is this game installed" is something you do while
 * looking at the game, not in a global settings list of every game that has
 * content.
 *
 * Install folders anchor the plugin file jail: an installer plugin's `gameDir`
 * root is whatever is set here, and it can never write outside it. So the path
 * is always chosen by BROWSING, never typed, and what enforces the policy is
 * Rust (`settings_set_game_dir` validates; `settings_patch` refuses the field),
 * not this screen. The picker is an affordance, not a control.
 *
 * "Find my games" reads what Steam, Epic, GOG and the rest already wrote down.
 * It changes nothing by itself: applying a result is a separate click that goes
 * through the same validator as a hand-picked folder.
 */
export default function MyAppsRoute() {
    const { app, reloadApp } = useSettings()
    const [search, setSearch] = useState('')
    const [found, setFound] = useState<DetectedGameT[] | null>(null)
    const [scanning, setScanning] = useState(false)
    const [error, setError] = useState<string | null>(null)
    const [managing, setManaging] = useState<number | null>(null)

    const catalogue = useQuery({
        queryKey: ['facets', 'mod'],
        queryFn: () => api.facets('mod'),
        staleTime: 10 * 60 * 1000,
    })

    const sandboxes = useQuery({
        queryKey: ['sandboxes'],
        queryFn: () => ipc.sandboxList(),
        staleTime: 10 * 1000,
    })

    const gameDirs = app?.gameDirs
    const sandboxRows = sandboxes.data

    const rows = useMemo(
        () =>
            buildManageRows({
                catalogue: catalogue.data?.apps ?? [],
                gameDirs: gameDirs ?? {},
                sandboxAppIds: (sandboxRows ?? []).map((s) => s.appId),
                detected: found ?? [],
                search,
            }),
        [catalogue.data, gameDirs, sandboxRows, found, search]
    )

    if (!app) return <Skeleton className="m-3 h-40 w-full" />

    const scan = async () => {
        setScanning(true)
        setError(null)

        try {
            setFound(await ipc.detectGames())
        } catch (err) {
            setError(isIpcError(err) ? err.message : 'The scan failed.')
        } finally {
            setScanning(false)
        }
    }

    const managed = rows.find((r) => r.appId === managing) ?? null
    const matched = (found ?? []).filter((g) => g.slug).length

    return (
        <div className="flex flex-col gap-3 p-3">
            <header>
                <h1 className="text-lg font-bold">Apps</h1>
                <p className="text-xs text-muted">
                    Your games and where they are installed. Choose Manage to set
                    a game&rsquo;s install folder.
                </p>
            </header>

            <div className="flex flex-wrap items-center gap-2">
                <div className="relative min-w-0 flex-1 basis-48">
                    <FiSearch className="pointer-events-none absolute left-2.5 top-1/2 size-3.5 -translate-y-1/2 text-muted" />
                    <input
                        value={search}
                        onChange={(e) => setSearch(e.target.value)}
                        placeholder="Search games"
                        aria-label="Search games"
                        className="w-full rounded-lg border border-border bg-surface-2 py-1.5 pl-8 pr-2 text-xs"
                    />
                </div>

                <button
                    type="button"
                    disabled={scanning}
                    onClick={() => void scan()}
                    className="flex shrink-0 items-center gap-1.5 rounded-lg border border-border px-3 py-1.5 text-xs hover:border-accent disabled:opacity-50"
                >
                    <FiSearch className="size-3" />
                    {scanning ? 'Scanning…' : found === null ? 'Find my games' : 'Scan again'}
                </button>
            </div>

            {found !== null && (
                <p className="text-xs text-muted">
                    {matched} game{matched === 1 ? '' : 's'} the app can manage,{' '}
                    {unmatchedCount(found)} other
                    {unmatchedCount(found) === 1 ? '' : 's'} found. Nothing is
                    changed until you apply a result in Manage.
                </p>
            )}

            {error && <ErrorNote message={error} />}

            {catalogue.isPending && <Skeleton className="h-40 w-full" />}

            {catalogue.isError && (
                <div className="flex items-center justify-between gap-3 rounded-xl border border-border p-3">
                    <p className="min-w-0 text-sm text-danger">
                        The game list could not be loaded: {messageOf(catalogue.error)}
                    </p>
                    <button
                        type="button"
                        onClick={() => void catalogue.refetch()}
                        className="shrink-0 rounded-md border border-border px-3 py-1 text-xs hover:bg-surface-hover"
                    >
                        Try again
                    </button>
                </div>
            )}

            {catalogue.isSuccess && rows.length < 1 && (
                <p className="rounded-xl border border-border p-6 text-center text-sm text-muted">
                    No games match.
                </p>
            )}

            <ul className="flex flex-col gap-2">
                {rows.map((row) => (
                    <li
                        key={row.appId}
                        className="flex items-center gap-3 rounded-xl border border-border bg-surface p-3"
                    >
                        <GameIcon
                            app={{ id: row.appId, name: row.name, icon: row.icon }}
                            size="lg"
                        />

                        <div className="min-w-0 flex-1">
                            <p className="truncate text-sm font-semibold">
                                {appLabel({ id: row.appId, name: row.name })}
                            </p>
                            <p className="flex items-center gap-1 truncate text-[11px] text-muted">
                                <FiFolder className="size-3 shrink-0" />
                                <span className="selectable truncate">
                                    {row.dir ??
                                        (row.status === 'detected'
                                            ? 'Found by the scan, not applied'
                                            : 'No folder set')}
                                </span>
                            </p>
                        </div>

                        <button
                            type="button"
                            onClick={() => setManaging(row.appId)}
                            aria-label={`Manage ${row.name}`}
                            className="flex shrink-0 items-center gap-1.5 rounded-lg border border-border px-2.5 py-1.5 text-xs hover:border-accent"
                        >
                            <FiSettings className="size-3.5" />
                            Manage
                        </button>
                    </li>
                ))}
            </ul>

            {managed && (
                <ManageDialog
                    row={managed}
                    scanning={scanning}
                    scanned={found !== null}
                    onScan={() => void scan()}
                    onApplied={() => {
                        void reloadApp()
                        setError(null)
                    }}
                    onClose={() => setManaging(null)}
                />
            )}
        </div>
    )
}

function ErrorNote({ message }: { message: string }) {
    return (
        <p className="flex items-start gap-2 rounded-lg border border-danger p-3 text-xs text-danger">
            <FiAlertTriangle className="mt-0.5 size-3.5 shrink-0" />
            {message}
        </p>
    )
}

/**
 * One game's settings.
 *
 * Rust's refusal is shown, not swallowed: "nothing happened when I chose a
 * folder" is the worst reading of a security check firing, and the message
 * names the reason (the folder encloses the app's own files, or is a system
 * directory) because the fix is to pick a different one.
 */
function ManageDialog({
    row,
    scanning,
    scanned,
    onScan,
    onApplied,
    onClose,
}: {
    row: ManageRow
    scanning: boolean
    scanned: boolean
    onScan: () => void
    onApplied: () => void
    onClose: () => void
}) {
    const dialogRef = useDialog<HTMLDivElement>(onClose)
    const { setGameDir } = useSettings()
    const { pick, element: picker } = useFolderPicker()
    const [error, setError] = useState<string | null>(null)
    const [applied, setApplied] = useState<Set<string>>(new Set())

    const guard = async (write: () => Promise<void>) => {
        setError(null)

        try {
            await write()
            onApplied()
        } catch (err) {
            setError(isIpcError(err) ? err.message : 'That folder cannot be used.')
        }
    }

    const choose = async () => {
        const dir = await pick({
            title: `Where is ${row.name} installed?`,
            hint: 'Installer plugins for this game can only write inside the folder you choose.',
            initial: row.dir,
        })

        if (dir === null) return

        await guard(() => setGameDir(row.appId, dir))
    }

    const use = async (game: DetectedGameT) => {
        if (!game.slug) return

        const slug = game.slug

        await guard(async () => {
            await ipc.detectApply(slug, game.path)
            setApplied((was) => new Set(was).add(game.path))
        })
    }

    return (
        <div
            className="fixed inset-0 z-50 flex items-end justify-center bg-black/50 p-0 md:items-center md:p-4"
            onClick={(e) => e.target === e.currentTarget && onClose()}
        >
            <div
                ref={dialogRef}
                tabIndex={-1}
                role="dialog"
                aria-modal="true"
                aria-label={`Manage ${row.name}`}
                className="flex max-h-[90vh] w-full max-w-lg flex-col gap-4 overflow-y-auto rounded-t-xl border border-border bg-surface p-4 focus:outline-none md:rounded-xl"
            >
                <div className="flex items-center justify-between gap-3">
                    <div className="flex min-w-0 items-center gap-2.5">
                        <GameIcon
                            app={{ id: row.appId, name: row.name, icon: row.icon }}
                        />
                        <h2 className="truncate text-sm font-semibold">{row.name}</h2>
                    </div>

                    <button
                        type="button"
                        onClick={onClose}
                        aria-label="Close"
                        className="shrink-0 rounded-lg border border-border p-2 text-muted hover:text-foreground"
                    >
                        <FiX className="size-4" />
                    </button>
                </div>

                {error && <ErrorNote message={error} />}

                <section className="flex flex-col gap-2">
                    <div>
                        <h3 className="text-xs font-semibold">Install folder</h3>
                        <p className="text-[11px] text-muted">
                            Installer plugins can only write inside the folder
                            you pick here, and only for this game.
                        </p>
                    </div>

                    <div className="flex items-center justify-between gap-3 rounded-lg border border-border px-3 py-2">
                        <p className="selectable min-w-0 truncate text-xs text-muted">
                            {row.dir ?? 'Not set'}
                        </p>

                        <div className="flex shrink-0 items-center gap-1">
                            <button
                                type="button"
                                onClick={() => void choose()}
                                aria-label={`Choose folder for ${row.name}`}
                                className="rounded-lg border border-border p-2 text-muted hover:text-foreground"
                            >
                                <FiFolder className="size-4" />
                            </button>

                            {row.dir && (
                                <button
                                    type="button"
                                    onClick={() =>
                                        void guard(() => setGameDir(row.appId, null))
                                    }
                                    aria-label={`Clear folder for ${row.name}`}
                                    className="rounded-lg border border-border p-2 text-muted hover:text-danger"
                                >
                                    <FiX className="size-4" />
                                </button>
                            )}
                        </div>
                    </div>
                </section>

                <section className="flex flex-col gap-2">
                    <div className="flex flex-wrap items-center justify-between gap-2">
                        <div className="min-w-0 flex-1 basis-48">
                            <h3 className="text-xs font-semibold">Find my games</h3>
                            <p className="text-[11px] text-muted">
                                Reads what Steam, Epic, GOG and the rest already
                                know. Nothing changes until you apply a result.
                            </p>
                        </div>

                        <button
                            type="button"
                            disabled={scanning}
                            onClick={onScan}
                            className="flex shrink-0 items-center gap-1.5 rounded-lg border border-border px-3 py-1.5 text-xs hover:border-accent disabled:opacity-50"
                        >
                            <FiSearch className="size-3" />
                            {scanning ? 'Scanning…' : scanned ? 'Scan again' : 'Scan'}
                        </button>
                    </div>

                    {scanned && row.detected.length === 0 && (
                        <p className="text-xs text-muted">
                            The scan did not find this game.
                        </p>
                    )}

                    {row.detected.map((game) => (
                        <div
                            key={`${game.source}:${game.path}`}
                            className="flex items-center justify-between gap-3 rounded-lg border border-border px-3 py-2"
                        >
                            <div className="min-w-0">
                                <p className="text-sm">
                                    {game.name}
                                    <span className="ml-2 rounded bg-surface-tertiary px-1.5 py-0.5 text-[0.6rem] uppercase text-muted">
                                        {game.source}
                                    </span>
                                </p>
                                <p className="selectable truncate text-xs text-muted">
                                    {game.path}
                                </p>
                                {game.replaces && (
                                    <p className="truncate text-[0.7rem] text-warning">
                                        Replaces {game.replaces}
                                    </p>
                                )}
                            </div>

                            {game.alreadySet ||
                            applied.has(game.path) ||
                            row.dir === game.path ? (
                                <span className="flex shrink-0 items-center gap-1 text-xs text-success">
                                    <FiCheck className="size-3.5" />
                                    In use
                                </span>
                            ) : (
                                <button
                                    type="button"
                                    onClick={() => void use(game)}
                                    className="shrink-0 rounded-lg bg-accent px-3 py-1.5 text-xs text-accent-foreground"
                                >
                                    Use this
                                </button>
                            )}
                        </div>
                    ))}
                </section>

                <Link
                    to="/sandboxes"
                    className="text-xs text-muted underline hover:text-foreground"
                    onClick={onClose}
                >
                    Mod profiles for this game are in Sandboxes
                </Link>
            </div>

            {picker}
        </div>
    )
}
