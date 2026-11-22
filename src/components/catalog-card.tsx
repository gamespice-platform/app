import { useEffect, useRef, useState, type KeyboardEvent as ReactKeyboardEvent } from 'react'
import { Link } from 'react-router-dom'
import {
    FiBox,
    FiCheck,
    FiDownload,
    FiMoreVertical,
    FiPackage,
    FiPlay,
    FiServer,
    FiStar,
    FiTrash2,
    FiUsers,
    FiZap,
} from 'react-icons/fi'

import type { AppSummaryT, AppTypeT } from '~/lib/api/contract'
import { compact } from '~/lib/api/play'
import type { CatalogEntry } from '~/lib/catalog/filter'
import { canonicalSlug, gameHeadingStyle, gameLook } from '~/lib/gamespice/games'
import { GameIcon } from '~/components/game-icon'
import { GameCardStats } from '~/components/game-card-stats'
import type { PlayAppRowT } from '~/components/play-center'

/**
 * One card in the Apps catalogue, its overflow menu, and the action list the
 * card, the menu and the detail drawer share.
 *
 * Every card is the same size as the grid's other cards. GameSpice's own games
 * stand out by their game's accent on the top edge, a GameSpice badge and the
 * website's live stats (players now, peak 24h, servers, the hourly chart) in
 * place of the description. A managed app carries an accent ring and badge.
 */

export type Entry = CatalogEntry<AppSummaryT, PlayAppRowT>

/** Device facts for a GameSpice game's native build; null for other apps. */
export type InstallState = {
    /** A build exists for this machine. */
    available: boolean
    version: string | null
    busy: boolean
} | null

export type CatalogHandlers = {
    onPlay: (entry: Entry, advanced: boolean) => void
    onQuick: (entry: Entry, advanced: boolean) => void
    onManage: (entry: Entry) => void
    onInstall: (entry: Entry) => void
    onUninstall: (entry: Entry) => void
    onDetails: (entry: Entry) => void
}

export const TYPE_LABELS: Record<AppTypeT, string> = {
    GAME: 'Game',
    GAME_SINGLEPLAYER: 'Singleplayer',
    GAME_ENGINE: 'Engine',
    VOIP: 'Voice',
    OTHER: 'Other',
}

const BTN_SM_PRIMARY =
    'flex items-center gap-1 rounded-lg bg-accent px-2.5 py-1 text-[11px] font-semibold text-accent-foreground hover:brightness-110 focus-visible:outline focus-visible:outline-2 focus-visible:outline-accent disabled:opacity-50'
const BTN_SM_GHOST =
    'flex items-center gap-1 rounded-lg border border-border px-2.5 py-1 text-[11px] hover:border-accent/60 focus-visible:outline focus-visible:outline-2 focus-visible:outline-accent disabled:opacity-50'

/** What can be started from this card right now. */
export function playOptions(entry: Entry, listed: boolean | null, playEnabled: boolean) {
    if (!playEnabled) return { quick: false, direct: false, solo: false }

    const { app, play } = entry

    return {
        quick: play !== null && listed !== false && play.serverCnt > 0,
        direct: app.play?.directPlay === true || play?.directPlay === true,
        solo: play?.solo === true || app.type === 'GAME_SINGLEPLAYER',
    }
}

export type CatalogLink = { label: string; to: string }

/** Where an app's content, tools and pages live elsewhere in the app. */
export function catalogLinks(entry: Entry): CatalogLink[] {
    const { app, gamespice } = entry
    const links: CatalogLink[] = [
        { label: 'Create sandbox', to: `/sandboxes?new=${app.id}` },
        { label: 'Workshop', to: `/workshop?game=${app.id}` },
    ]

    if (app.counts.mods > 0) links.push({ label: 'Mods', to: `/browse/mod?apps=${app.id}` })
    if (app.counts.assets > 0) links.push({ label: 'Assets', to: `/browse/asset?apps=${app.id}` })
    if (app.hasServers || (entry.play?.serverCnt ?? 0) > 0)
        links.push({ label: 'Servers', to: `/browse/server?apps=${app.id}` })
    if (gamespice && app.slug) links.push({ label: 'Game page', to: `/games/${canonicalSlug(app.slug)}` })
    links.push({ label: 'Stats', to: `/stats/${app.id}` })
    links.push({ label: 'Full page', to: `/apps/${app.id}` })

    return links
}

function Stat({ icon: Icon, value, label }: { icon: typeof FiServer; value: number; label: string }) {
    if (value <= 0) return null

    return (
        <span className="flex items-center gap-1" title={`${value} ${label}`}>
            <Icon className="size-3" aria-hidden />
            {compact(value)}
            <span className="sr-only"> {label}</span>
        </span>
    )
}

export function PlayButtons({
    entry,
    options,
    handlers,
}: {
    entry: Entry
    options: ReturnType<typeof playOptions>
    handlers: CatalogHandlers
}) {
    return (
        <>
            {options.quick && (
                <button
                    type="button"
                    title="Join the best open server. Right-click for options."
                    onClick={() => handlers.onQuick(entry, false)}
                    onContextMenu={(e) => {
                        e.preventDefault()
                        handlers.onQuick(entry, true)
                    }}
                    className={BTN_SM_PRIMARY}
                >
                    <FiZap className="size-3" aria-hidden />
                    Quick play
                </button>
            )}
            {options.direct && (
                <button
                    type="button"
                    title="Right-click for options"
                    onClick={() => handlers.onPlay(entry, false)}
                    onContextMenu={(e) => {
                        e.preventDefault()
                        handlers.onPlay(entry, true)
                    }}
                    className={options.quick ? BTN_SM_GHOST : BTN_SM_PRIMARY}
                >
                    <FiPlay className="size-3" aria-hidden />
                    {options.solo ? 'Play solo' : 'Play'}
                </button>
            )}
        </>
    )
}

export function InstallButton({
    entry,
    install,
    handlers,
}: {
    entry: Entry
    install: InstallState
    handlers: CatalogHandlers
}) {
    if (!install?.available && !install?.version) return null

    return install.version ? (
        <button
            type="button"
            disabled={install.busy}
            onClick={() => handlers.onUninstall(entry)}
            title={`Installed ${install.version}. Uninstall from this device.`}
            className={BTN_SM_GHOST}
        >
            <FiTrash2 className="size-3" aria-hidden />
            {install.busy ? 'Working…' : 'Uninstall'}
        </button>
    ) : (
        <button
            type="button"
            disabled={install.busy}
            onClick={() => handlers.onInstall(entry)}
            className={BTN_SM_GHOST}
        >
            <FiDownload className="size-3" aria-hidden />
            {install.busy ? 'Installing…' : 'Install'}
        </button>
    )
}

export function ManageButton({ entry, handlers }: { entry: Entry; handlers: CatalogHandlers }) {
    const { managed, gamespice } = entry

    return (
        <button
            type="button"
            aria-pressed={managed}
            disabled={gamespice}
            onClick={() => handlers.onManage(entry)}
            title={
                gamespice
                    ? 'GameSpice games are always managed.'
                    : managed
                      ? 'Stop managing this app here.'
                      : 'Manage this app from the app: keep it in Managed, with its sandboxes and content.'
            }
            className={`flex items-center gap-1 rounded-lg border px-2.5 py-1 text-[11px] focus-visible:outline focus-visible:outline-2 focus-visible:outline-accent disabled:cursor-default ${
                managed ? 'border-accent bg-accent/10 text-accent' : 'border-border text-muted hover:text-foreground'
            }`}
        >
            {managed && <FiCheck className="size-3" aria-hidden />}
            {managed ? 'Managed' : 'Manage'}
        </button>
    )
}

export function CatalogCard({
    entry,
    listed,
    playEnabled,
    install,
    handlers,
}: {
    entry: Entry
    listed: boolean | null
    playEnabled: boolean
    install: InstallState
    handlers: CatalogHandlers
}) {
    const { app, play, gamespice, managed, installed } = entry
    const art = app.images.card ?? app.images.banner ?? play?.bannerUrl ?? null
    const look = gamespice ? gameLook(app.slug ?? app.name) : null
    const options = playOptions(entry, listed, playEnabled)

    return (
        <article
            aria-label={app.name}
            className={`group relative flex flex-col rounded-xl border bg-surface transition focus-within:z-10 hover:z-10 hover:-translate-y-0.5 hover:shadow-lg hover:shadow-black/30 ${
                managed ? 'border-accent/50 ring-1 ring-accent/40' : 'border-border'
            }`}
            style={look ? { borderTopColor: look.accent, borderTopWidth: 3 } : undefined}
        >
            <div className="relative aspect-[16/7] w-full overflow-hidden rounded-t-[11px] bg-surface-2">
                {art ? (
                    <img
                        src={art}
                        alt=""
                        aria-hidden
                        loading="lazy"
                        className="size-full object-cover transition duration-300 group-hover:scale-105"
                    />
                ) : null}

                <div className="absolute left-2 top-2 flex flex-wrap gap-1">
                    {gamespice && (
                        <span className="flex items-center gap-1 rounded-full bg-black/70 px-2 py-0.5 text-[10px] font-semibold text-white backdrop-blur">
                            <FiZap className="size-2.5 text-accent" aria-hidden />
                            GameSpice
                        </span>
                    )}
                    {managed && (
                        <span className="rounded-full bg-accent px-2 py-0.5 text-[10px] font-semibold text-accent-foreground">
                            Managed
                        </span>
                    )}
                    {installed && (
                        <span className="flex items-center gap-1 rounded-full bg-black/70 px-2 py-0.5 text-[10px] text-success backdrop-blur">
                            <FiCheck className="size-2.5" aria-hidden />
                            Installed
                        </span>
                    )}
                    {app.isOfficial && !gamespice && (
                        <span className="flex items-center gap-1 rounded-full bg-black/60 px-2 py-0.5 text-[10px] font-medium text-white backdrop-blur">
                            <FiStar className="size-2.5" aria-hidden />
                            Official
                        </span>
                    )}
                </div>
            </div>

            {/* Outside the art, which clips: the menu drops below it. */}
            <div className="absolute right-2 top-2">
                <CardMenu entry={entry} install={install} handlers={handlers} />
            </div>

            <div className="flex flex-1 flex-col gap-2 p-3">
                <div className="flex items-start gap-2.5">
                    <GameIcon app={{ id: app.id, name: app.name, icon: app.images.icon }} size="md" />

                    <div className="min-w-0 flex-1">
                        <h3 className="truncate text-sm font-semibold" style={look ? gameHeadingStyle(look) : undefined}>
                            <button
                                type="button"
                                onClick={() => handlers.onDetails(entry)}
                                className="max-w-full truncate text-left hover:underline focus-visible:outline focus-visible:outline-2 focus-visible:outline-accent"
                            >
                                {app.name}
                            </button>
                        </h3>
                        <p className="truncate text-[11px] text-muted">
                            {TYPE_LABELS[app.type]}
                            {app.engine ? ` · ${app.engine.name}` : ''}
                            {install?.version ? ` · v${install.version}` : ''}
                        </p>
                    </div>
                </div>

                {gamespice && play ? (
                    <GameCardStats app={play} accent={look?.accent} />
                ) : (
                    <>
                        {app.description && <p className="line-clamp-2 text-[11px] text-muted">{app.description}</p>}
                        <div className="mt-auto flex flex-wrap items-center gap-3 text-[11px] text-muted">
                            <Stat icon={FiPackage} value={app.counts.mods} label="mods" />
                            <Stat icon={FiBox} value={app.counts.assets} label="assets" />
                            <Stat icon={FiServer} value={app.counts.servers} label="servers" />
                            <Stat icon={FiUsers} value={app.counts.players} label="players online" />
                        </div>
                    </>
                )}

                <div className="mt-auto flex flex-wrap items-center gap-1.5 pt-1">
                    <PlayButtons entry={entry} options={options} handlers={handlers} />
                    <InstallButton entry={entry} install={install} handlers={handlers} />
                    <span className="ml-auto">
                        <ManageButton entry={entry} handlers={handlers} />
                    </span>
                </div>
            </div>
        </article>
    )
}

/**
 * The card's overflow menu: a menu button with arrow-key navigation, Escape
 * and outside clicks to close, and focus returned to the button.
 */
function CardMenu({
    entry,
    install,
    handlers,
}: {
    entry: Entry
    install: InstallState
    handlers: CatalogHandlers
}) {
    const [open, setOpen] = useState(false)
    const button = useRef<HTMLButtonElement>(null)
    const menu = useRef<HTMLDivElement>(null)

    const items = () => [...(menu.current?.querySelectorAll<HTMLElement>('[role="menuitem"]') ?? [])]

    const close = (refocus: boolean) => {
        setOpen(false)
        if (refocus) button.current?.focus()
    }

    useEffect(() => {
        if (!open) return

        items()[0]?.focus()

        const onDown = (e: MouseEvent) => {
            const t = e.target

            if (t instanceof Node && !menu.current?.contains(t) && !button.current?.contains(t)) setOpen(false)
        }

        document.addEventListener('mousedown', onDown)

        return () => document.removeEventListener('mousedown', onDown)
    }, [open])

    const onKey = (e: ReactKeyboardEvent) => {
        const list = items()
        const at = list.indexOf(document.activeElement as HTMLElement)

        if (e.key === 'Escape') {
            e.preventDefault()
            e.stopPropagation()
            close(true)
        } else if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
            e.preventDefault()
            const step = e.key === 'ArrowDown' ? 1 : -1

            list[(at + step + list.length) % list.length]?.focus()
        } else if (e.key === 'Home' || e.key === 'End') {
            e.preventDefault()
            ;(e.key === 'Home' ? list[0] : list[list.length - 1])?.focus()
        } else if (e.key === 'Tab') {
            close(false)
        }
    }

    // Built during render, so it must not touch a ref: closing without refocus.
    const run = (fn: () => void) => () => {
        setOpen(false)
        fn()
    }

    const ITEM =
        'block w-full px-3 py-1.5 text-left text-xs hover:bg-surface-2 focus:bg-surface-2 focus:outline-none disabled:opacity-50'

    return (
        <>
            <button
                ref={button}
                type="button"
                aria-label={`More for ${entry.app.name}`}
                aria-haspopup="menu"
                aria-expanded={open}
                onClick={() => setOpen((o) => !o)}
                onKeyDown={(e) => {
                    if (e.key === 'ArrowDown' && !open) {
                        e.preventDefault()
                        setOpen(true)
                    }
                }}
                className="grid size-7 place-items-center rounded-full bg-black/60 text-white backdrop-blur hover:bg-black/80 focus-visible:outline focus-visible:outline-2 focus-visible:outline-accent"
            >
                <FiMoreVertical className="size-3.5" aria-hidden />
            </button>

            {open && (
                <div
                    ref={menu}
                    role="menu"
                    aria-label={`${entry.app.name} actions`}
                    onKeyDown={onKey}
                    className="absolute right-0 z-20 mt-1 w-48 overflow-hidden rounded-lg border border-border bg-surface py-1 shadow-xl"
                >
                    <button type="button" role="menuitem" className={ITEM} onClick={run(() => handlers.onDetails(entry))}>
                        Details
                    </button>
                    {!entry.gamespice && (
                        <button type="button" role="menuitem" className={ITEM} onClick={run(() => handlers.onManage(entry))}>
                            {entry.managed ? 'Stop managing' : 'Manage'}
                        </button>
                    )}
                    {install?.version ? (
                        <button
                            type="button"
                            role="menuitem"
                            disabled={install.busy}
                            className={ITEM}
                            onClick={run(() => handlers.onUninstall(entry))}
                        >
                            Uninstall
                        </button>
                    ) : install?.available ? (
                        <button
                            type="button"
                            role="menuitem"
                            disabled={install.busy}
                            className={ITEM}
                            onClick={run(() => handlers.onInstall(entry))}
                        >
                            Install
                        </button>
                    ) : null}
                    <div role="separator" className="my-1 h-px bg-border" />
                    {catalogLinks(entry).map((l) => (
                        <Link key={l.to} to={l.to} role="menuitem" className={ITEM} onClick={() => setOpen(false)}>
                            {l.label}
                        </Link>
                    ))}
                </div>
            )}
        </>
    )
}
