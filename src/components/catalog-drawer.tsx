import { Link } from 'react-router-dom'
import { FiX } from 'react-icons/fi'

import { compact } from '~/lib/api/play'
import { gameHeadingStyle, gameLook } from '~/lib/gamespice/games'
import { GameIcon } from '~/components/game-icon'
import { GameCardStats } from '~/components/game-card-stats'
import { useDialog } from '~/components/use-dialog'
import {
    InstallButton,
    ManageButton,
    PlayButtons,
    TYPE_LABELS,
    catalogLinks,
    playOptions,
    type CatalogHandlers,
    type Entry,
    type InstallState,
} from '~/components/catalog-card'

/**
 * The catalogue's detail drawer: everything a card does, with the room to say
 * it. A modal side panel (focus trapped, Escape closes, focus goes back to the
 * card that opened it) rather than a page, so the grid and its filters stay put.
 */
export function CatalogDrawer({
    entry,
    listed,
    playEnabled,
    install,
    handlers,
    onClose,
}: {
    entry: Entry
    listed: boolean | null
    playEnabled: boolean
    install: InstallState
    handlers: CatalogHandlers
    onClose: () => void
}) {
    const ref = useDialog<HTMLDivElement>(onClose)
    const { app, play, gamespice, managed, installed } = entry
    const look = gamespice ? gameLook(app.slug ?? app.name) : null
    const art = app.images.banner ?? app.images.card ?? play?.bannerUrl ?? null
    const titleId = `catalog-drawer-${app.id}`

    return (
        <div className="fixed inset-0 z-50 flex justify-end">
            <button
                type="button"
                aria-label="Close details"
                tabIndex={-1}
                onClick={onClose}
                className="absolute inset-0 bg-black/50"
            />
            <div
                ref={ref}
                role="dialog"
                aria-modal="true"
                aria-labelledby={titleId}
                tabIndex={-1}
                className="relative flex h-full w-full max-w-md flex-col overflow-y-auto border-l border-border bg-surface shadow-2xl focus:outline-none"
                style={look ? { borderTopColor: look.accent, borderTopWidth: 3 } : undefined}
            >
                <div className="relative aspect-[16/7] w-full shrink-0 bg-surface-2">
                    {art && <img src={art} alt="" aria-hidden className="size-full object-cover" />}
                    <button
                        type="button"
                        onClick={onClose}
                        aria-label="Close"
                        className="absolute right-2 top-2 grid size-8 place-items-center rounded-full bg-black/60 text-white hover:bg-black/80 focus-visible:outline focus-visible:outline-2 focus-visible:outline-accent"
                    >
                        <FiX className="size-4" aria-hidden />
                    </button>
                </div>

                <div className="flex flex-col gap-4 p-4">
                    <div className="flex items-start gap-3">
                        <GameIcon app={{ id: app.id, name: app.name, icon: app.images.icon }} size="md" />
                        <div className="min-w-0 flex-1">
                            <h2 id={titleId} className="text-base font-semibold" style={look ? gameHeadingStyle(look) : undefined}>
                                {app.name}
                            </h2>
                            <p className="text-[11px] text-muted">
                                {TYPE_LABELS[app.type]}
                                {app.engine ? ` · ${app.engine.name}` : ''}
                                {gamespice ? ' · GameSpice' : ''}
                            </p>
                        </div>
                    </div>

                    <div className="flex flex-wrap items-center gap-1.5">
                        <PlayButtons entry={entry} options={playOptions(entry, listed, playEnabled)} handlers={handlers} />
                        <InstallButton entry={entry} install={install} handlers={handlers} />
                        <ManageButton entry={entry} handlers={handlers} />
                    </div>

                    {play && <GameCardStats app={play} accent={look?.accent} />}

                    {app.description && <p className="text-xs text-muted">{app.description}</p>}

                    <dl className="grid grid-cols-2 gap-2 text-xs">
                        <Fact label="Mods" value={compact(app.counts.mods)} />
                        <Fact label="Assets" value={compact(app.counts.assets)} />
                        <Fact label="Servers" value={compact(app.counts.servers)} />
                        <Fact label="Players online" value={compact(app.counts.players)} />
                        <Fact label="Managed" value={managed ? (gamespice ? 'Always (GameSpice)' : 'Yes') : 'No'} />
                        <Fact
                            label="On this device"
                            value={install?.version ? `v${install.version}` : installed ? 'Installed' : 'Not installed'}
                        />
                    </dl>

                    <nav aria-label={`${app.name} pages`} className="flex flex-wrap gap-1.5">
                        {catalogLinks(entry).map((l) => (
                            <Link
                                key={l.to}
                                to={l.to}
                                className="rounded-lg border border-border px-2.5 py-1 text-[11px] hover:border-accent/60 focus-visible:outline focus-visible:outline-2 focus-visible:outline-accent"
                            >
                                {l.label}
                            </Link>
                        ))}
                    </nav>
                </div>
            </div>
        </div>
    )
}

function Fact({ label, value }: { label: string; value: string }) {
    return (
        <div className="rounded-lg border border-border bg-surface-2 px-2.5 py-1.5">
            <dt className="text-[10px] uppercase tracking-wide text-muted">{label}</dt>
            <dd className="font-medium">{value}</dd>
        </div>
    )
}
