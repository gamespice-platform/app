import { Link, useSearchParams } from 'react-router-dom'
import { FiGrid, FiHardDrive, FiLayers, FiPackage } from 'react-icons/fi'

import { useAuth } from '~/lib/auth/provider'
import { useLibrary } from '~/lib/library/provider'
import LibraryGames from '~/components/library-games'
import LibraryContent from '~/components/library-content'
import LibraryTmcGames from '~/components/library-tmc'
import Catalog from '~/components/catalog'
import SectionCards from '~/components/section-cards'
import { LIBRARY, type NavNode } from '~/lib/nav'

/**
 * **Library**: the app's front door. Its own page IS the games catalogue;
 * there is no separate Apps or Play page any more (`/apps`, `/play` and
 * `/games` redirect here with their query, so `gs://play/app` deep links and
 * old bookmarks still work).
 *
 * The cards on top link to every child of the group (Mods, Sandboxes,
 * Workshop, Downloads, Stores, Installs); the bottom bar cannot nest, so on a
 * phone they are the only way in. Below them, three views:
 *
 *   * **Games** (default): the unified catalogue (`components/catalog.tsx`),
 *     with its own All / Managed / Unmanaged / GameSpice tabs, search and
 *     sort. It replaced the old "Games" and "GameSpice Games" views.
 *   * **On this device**: the two device lists those views used to be,
 *     sandboxes and play time for other launchers' games, and the GameSpice
 *     builds this app installed with their update and auto-update controls.
 *     The catalogue says what a game IS; this says what this machine holds.
 *   * **Subscribed**: what the ACCOUNT subscribed to, mirrored down.
 *
 * The view is in the URL (`view=`) so a link to one half survives a reload;
 * the retired `view=tmc` lands on the device view.
 */

/*
 * Installs is not in the rail: it is a list, not a place to work, so it is
 * reached from here rather than given a row beside Sandboxes.
 */
const INSTALLS: NavNode = {
    id: 'installs',
    to: '/installs',
    label: 'Installs',
    icon: FiPackage,
    description: 'Sandbox installs saved to your account.',
}

type View = 'games' | 'device' | 'content'

function viewOf(raw: string | null): View {
    if (raw === 'content') return 'content'
    if (raw === 'device' || raw === 'tmc') return 'device'

    return 'games'
}

export default function LibraryRoute() {
    const { status } = useAuth()
    const { rows } = useLibrary()
    const [params, setParams] = useSearchParams()
    const view = viewOf(params.get('view'))

    const setView = (next: View) => {
        // The catalogue's own filters belong to the games view only.
        const updated = next === 'games' ? new URLSearchParams() : new URLSearchParams({ view: next })

        setParams(updated, { replace: true })
    }

    /*
     * The catalogue and the device lists work signed out: game folders,
     * sandboxes and play time are facts about this machine. Only the
     * subscription list needs an account, so only that view asks for one.
     */
    if (status !== 'signedIn' && view === 'content')
        return (
            <div className="flex flex-col gap-3 p-3">
                <Landing />
                <Header view={view} onChange={setView} count={0} />

                <div className="rounded-xl border border-border p-6 text-center">
                    <p className="text-sm text-muted">
                        Sign in to see the mods and assets you subscribed to.
                    </p>
                    <Link
                        to="/account"
                        className="mt-3 inline-block rounded-lg bg-accent px-4 py-2 text-sm text-accent-foreground"
                    >
                        Sign in
                    </Link>
                </div>
            </div>
        )

    return (
        <div className="flex flex-col gap-3 p-3">
            <Landing />
            <Header view={view} onChange={setView} count={rows.length} />

            {view === 'games' && <Catalog />}
            {view === 'device' && (
                <div className="flex flex-col gap-6">
                    <LibraryGames />
                    <section className="flex flex-col gap-2">
                        <h2 className="text-sm font-semibold">GameSpice games on this device</h2>
                        <LibraryTmcGames />
                    </section>
                </div>
            )}
            {view === 'content' && <LibraryContent />}
        </div>
    )
}

function Landing() {
    return (
        <>
            <header>
                <h1 className="text-lg font-bold">Library</h1>
                <p className="text-xs text-muted">
                    Every game, and what this device does with it: play,
                    install, mod and manage.
                </p>
            </header>

            <SectionCards node={LIBRARY} extra={[INSTALLS]} />
        </>
    )
}

function Header({
    view,
    onChange,
    count,
}: {
    view: View
    onChange: (next: View) => void
    count: number
}) {
    return (
        <div className="flex flex-wrap items-center justify-between gap-2">
            <div className="flex items-center gap-1 rounded-lg border border-border p-0.5">
                <Tab
                    active={view === 'games'}
                    onClick={() => onChange('games')}
                    icon={FiGrid}
                    label="Games"
                />
                <Tab
                    active={view === 'device'}
                    onClick={() => onChange('device')}
                    icon={FiHardDrive}
                    label="On this device"
                />
                <Tab
                    active={view === 'content'}
                    onClick={() => onChange('content')}
                    icon={FiPackage}
                    label="Subscribed"
                    badge={count || undefined}
                />
            </div>
        </div>
    )
}

function Tab({
    active,
    onClick,
    icon: Icon,
    label,
    badge,
}: {
    active: boolean
    onClick: () => void
    icon: typeof FiLayers
    label: string
    badge?: number
}) {
    return (
        <button
            type="button"
            onClick={onClick}
            aria-pressed={active}
            className={`flex items-center gap-1.5 rounded-md px-3 py-1.5 text-xs transition ${
                active ? 'bg-accent text-accent-foreground' : 'text-muted'
            }`}
        >
            <Icon className="size-3.5" />
            {label}
            {badge !== undefined && <span className="opacity-70">{badge}</span>}
        </button>
    )
}
