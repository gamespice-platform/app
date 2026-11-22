import { useCallback, useEffect, useState } from 'react'

import TextField from '~/components/text-field'
import type { LoaderVersionWireT } from '~/lib/api/contract'
import { messageOf } from '~/lib/ipc'
import { ipc } from '~/lib/ipc/commands'
import type { CatalogViewT } from '~/lib/ipc/schemas'
import {
    carryLoaderVersion,
    defaultLoaderVersion,
    findLoader,
    hasCatalog,
    provenanceNotice,
} from '~/lib/catalog/picker'
import { ct } from '~/lib/catalog/text'
import LoaderPicker from './loader-picker'
import LoaderVersionPicker from './loader-version-picker'
import VersionPicker from './version-picker'

export type VersionFields = {
    gameVersion: string | null
    loader: string | null
    loaderVersion: string | null
}

type CatalogState =
    | { kind: 'loading' }
    | { kind: 'ready'; view: CatalogViewT }
    | { kind: 'error'; message: string }

/**
 * The game's catalog through `catalog_get` (remote, cached or compiled in;
 * Rust decides). `reload(true)` skips the cache's TTL.
 */
export function useCatalog(appId: number, appSlug: string | null) {
    const [state, setState] = useState<CatalogState>({ kind: 'loading' })
    const [nonce, setNonce] = useState(0)
    const [refresh, setRefresh] = useState(false)

    useEffect(() => {
        let live = true
        void (async () => {
            if (!appSlug) {
                if (live) setState({ kind: 'error', message: '' })
                return
            }
            try {
                const view = await ipc.catalogGet(appId, appSlug, { refresh })
                if (live) setState({ kind: 'ready', view })
            } catch (err) {
                if (live) setState({ kind: 'error', message: messageOf(err) })
            }
        })()
        return () => {
            live = false
        }
    }, [appId, appSlug, nonce, refresh])

    const reload = useCallback((force: boolean) => {
        setRefresh(force)
        setNonce((n) => n + 1)
    }, [])

    return { state, reload }
}

/**
 * Game version, loader and loader version for one game, data-driven by its
 * catalog. A game with no catalog (none served, none compiled in, or an
 * empty one) gets the two free-text fields it always had.
 *
 * `onPatch` gets only the fields that change, so a game version change that
 * moves the loader version writes both in one patch.
 */
export default function CatalogFields({
    appId,
    appSlug,
    value,
    onPatch,
    disabled,
}: {
    appId: number
    appSlug: string | null
    value: VersionFields
    onPatch: (next: Partial<VersionFields>) => void | Promise<void>
    disabled?: boolean
}) {
    const { state, reload } = useCatalog(appId, appSlug)
    const [builds, setBuilds] = useState<LoaderVersionWireT[]>([])
    const [buildsLoading, setBuildsLoading] = useState(false)
    const [notice, setNotice] = useState<string | null>(null)

    const view = state.kind === 'ready' ? state.view : null
    const loader = view ? findLoader(view.loaders, value.loader) : undefined
    const loaderKey = loader?.key ?? null

    const fetchBuilds = useCallback(
        async (key: string, gameVersion: string | null) => {
            if (!appSlug) return []
            const got = await ipc.catalogLoaderVersions(appId, appSlug, key, {
                gameVersion: gameVersion ?? undefined,
            })
            return got.items
        },
        [appId, appSlug]
    )

    // The builds list follows the stored loader and game version.
    useEffect(() => {
        let live = true
        void (async () => {
            if (!loaderKey) {
                if (live) setBuilds([])
                return
            }
            if (live) setBuildsLoading(true)
            const items = await fetchBuilds(loaderKey, value.gameVersion).catch(
                () => []
            )
            if (live) {
                setBuilds(items)
                setBuildsLoading(false)
            }
        })()
        return () => {
            live = false
        }
    }, [loaderKey, value.gameVersion, fetchBuilds])

    if (state.kind === 'loading') {
        return (
            <p className="text-xs text-muted sm:col-span-2">
                {ct('catalog.loaderVersion.loading')}
            </p>
        )
    }

    if (!view || !hasCatalog(view)) {
        return (
            <FreeText
                value={value}
                onPatch={onPatch}
                error={state.kind === 'error' && state.message ? state.message : null}
                onRetry={() => reload(true)}
            />
        )
    }

    const offline = provenanceNotice(view)

    const pickVersion = async (gameVersion: string | null) => {
        setNotice(null)
        if (!loaderKey || !loader) {
            await onPatch({ gameVersion })
            return
        }
        const items = await fetchBuilds(loaderKey, gameVersion).catch(() => [])
        const carry = carryLoaderVersion(items, value.loaderVersion, {
            loader: loader.name,
            gameVersion: gameVersion ?? ct('catalog.version.any'),
        })
        setNotice(carry.notice)
        await onPatch({ gameVersion, loaderVersion: carry.value })
    }

    const pickLoader = async (next: string | null) => {
        setNotice(null)
        if (!next) {
            await onPatch({ loader: null, loaderVersion: null })
            return
        }
        const items = findLoader(view.loaders, next)
            ? await fetchBuilds(next, value.gameVersion).catch(() => [])
            : []
        await onPatch({ loader: next, loaderVersion: defaultLoaderVersion(items) })
    }

    return (
        <>
            {offline && (
                <p
                    role="status"
                    data-testid="catalog-offline"
                    className="flex items-center justify-between gap-2 rounded-lg border border-warning/50 px-3 py-2 text-xs text-warning sm:col-span-2"
                >
                    <span>{offline}</span>
                    <button
                        type="button"
                        onClick={() => reload(true)}
                        className="rounded-md border border-border px-2 py-0.5 text-foreground hover:border-accent"
                    >
                        {ct('catalog.source.refresh')}
                    </button>
                </p>
            )}

            <Field label={ct('catalog.version.label')}>
                <VersionPicker
                    items={view.versions.items}
                    value={value.gameVersion}
                    onChange={(next) => void pickVersion(next)}
                    disabled={disabled}
                />
            </Field>

            <Field label={ct('catalog.loader.label')}>
                <LoaderPicker
                    loaders={view.loaders}
                    value={value.loader}
                    onChange={(next) => void pickLoader(next)}
                    disabled={disabled}
                />
            </Field>

            {loader && (
                <Field label={ct('catalog.loaderVersion.label')} hint={notice}>
                    <LoaderVersionPicker
                        items={builds}
                        value={value.loaderVersion}
                        loading={buildsLoading}
                        onChange={(next) => {
                            setNotice(null)
                            void onPatch({ loaderVersion: next })
                        }}
                        disabled={disabled}
                    />
                </Field>
            )}
        </>
    )
}

function Field({
    label,
    hint,
    children,
}: {
    label: string
    hint?: string | null
    children: React.ReactNode
}) {
    return (
        <div className="min-w-0">
            <span className="mb-1 block text-xs text-muted">{label}</span>
            {children}
            {hint && (
                <p role="status" className="mt-1 text-[0.7rem] text-warning">
                    {hint}
                </p>
            )}
        </div>
    )
}

/** The fields as they were before catalogs: typed by hand, stored as typed. */
function FreeText({
    value,
    onPatch,
    error,
    onRetry,
}: {
    value: VersionFields
    onPatch: (next: Partial<VersionFields>) => void | Promise<void>
    error: string | null
    onRetry: () => void
}) {
    return (
        <>
            <p
                data-testid="catalog-free-text"
                className="flex items-center justify-between gap-2 text-xs text-muted sm:col-span-2"
            >
                <span>
                    {error
                        ? ct('catalog.source.failed', { error })
                        : ct('catalog.free.hint')}
                </span>
                {error && (
                    <button
                        type="button"
                        onClick={onRetry}
                        className="rounded-md border border-border px-2 py-0.5 text-foreground hover:border-accent"
                    >
                        {ct('catalog.source.refresh')}
                    </button>
                )}
            </p>
            <Field label={ct('catalog.version.label')}>
                <TextField
                    label={ct('catalog.version.label')}
                    value={value.gameVersion ?? ''}
                    placeholder={ct('catalog.free.version')}
                    onCommit={(next) => void onPatch({ gameVersion: next || null })}
                />
            </Field>
            <Field label={ct('catalog.loader.label')}>
                <TextField
                    label={ct('catalog.loader.label')}
                    value={value.loader ?? ''}
                    placeholder={ct('catalog.free.loader')}
                    onCommit={(next) => void onPatch({ loader: next || null })}
                />
            </Field>
        </>
    )
}
