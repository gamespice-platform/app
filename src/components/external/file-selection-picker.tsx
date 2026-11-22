import { useCallback, useMemo, useState, type KeyboardEvent } from 'react'
import { FiChevronDown, FiChevronRight, FiFile, FiFolder } from 'react-icons/fi'

import { loadLayout, saveLayout, type LayoutPrefsT } from '~/lib/external/layout-prefs'
import type { FileTreeT } from '~/lib/external/schemas'
import {
    allFolders,
    applyPreset,
    buildIndex,
    extOf,
    formatBytes,
    includedPaths,
    invert,
    PRESETS,
    selectAll,
    stateOf,
    summarise,
    toggle,
    visibleRows,
    type SortKey,
} from '~/lib/external/selection'

import { buttonPlain, buttonPrimary } from './modal'

/**
 * Pick which files of a dropped folder or zip to install, when it carries no
 * metadata saying what it is. Folders are tri-state; the result is the minimal
 * list of paths (`onConfirm`) the core expands and copies.
 */
export function FileSelectionPicker({
    tree,
    onConfirm,
    onCancel,
    busy = false,
    initialLayout,
}: {
    tree: FileTreeT
    onConfirm: (paths: string[]) => void
    onCancel: () => void
    busy?: boolean
    /** Tests supply a layout; the app reads the remembered one. */
    initialLayout?: LayoutPrefsT
}) {
    const index = useMemo(() => buildIndex(tree), [tree])
    const [layout, setLayoutState] = useState<LayoutPrefsT>(() => initialLayout ?? loadLayout())
    const [checked, setChecked] = useState<Set<string>>(
        () => new Set(tree.suggested.length > 0 ? tree.suggested.filter((p) => index.byPath.get(p)?.isDir === false) : [])
    )
    const [expanded, setExpanded] = useState<Set<string>>(() => new Set())
    const [query, setQuery] = useState('')
    const [focus, setFocus] = useState(0)

    const setLayout = useCallback((next: LayoutPrefsT) => {
        setLayoutState(next)
        saveLayout(next)
    }, [])

    const rows = useMemo(() => visibleRows(index, expanded, query, layout.sort), [index, expanded, query, layout.sort])
    const sum = summarise(index, checked)
    const at = Math.min(focus, Math.max(0, rows.length - 1))

    const open = (path: string, on: boolean) =>
        setExpanded((prev) => {
            const next = new Set(prev)

            if (on) next.add(path)
            else next.delete(path)

            return next
        })

    const onKey = (e: KeyboardEvent) => {
        const row = rows[at]

        if (!row) return

        const move = (to: number) => {
            e.preventDefault()
            setFocus(Math.max(0, Math.min(rows.length - 1, to)))
        }

        if (e.key === 'ArrowDown') move(at + 1)
        else if (e.key === 'ArrowUp') move(at - 1)
        else if (e.key === 'Home') move(0)
        else if (e.key === 'End') move(rows.length - 1)
        else if (e.key === 'ArrowRight' && row.node.isDir) {
            e.preventDefault()

            if (row.open) move(at + 1)
            else open(row.node.path, true)
        } else if (e.key === 'ArrowLeft') {
            e.preventDefault()

            if (row.node.isDir && row.open) open(row.node.path, false)
            else {
                const parent = rows.findIndex((r) => r.node.isDir && row.node.path.startsWith(`${r.node.path}/`) && r.depth === row.depth - 1)

                if (parent >= 0) setFocus(parent)
            }
        } else if (e.key === ' ' || e.key === 'Enter') {
            e.preventDefault()
            setChecked(toggle(index, checked, row.node))
        }
    }

    const sortBy = (key: SortKey) =>
        setLayout({
            ...layout,
            sort: { key, dir: layout.sort.key === key && layout.sort.dir === 'asc' ? 'desc' : 'asc' },
        })

    return (
        <div className="flex flex-col gap-3">
            <div className="flex flex-wrap items-center gap-2">
                <label className="sr-only" htmlFor="picker-search">
                    Filter files
                </label>
                <input
                    id="picker-search"
                    type="search"
                    value={query}
                    onChange={(e) => setQuery(e.target.value)}
                    placeholder="Filter files"
                    className="min-w-0 flex-1 rounded-lg border border-border bg-surface-2 px-2 py-1.5 text-sm"
                />
                <button type="button" className={buttonPlain} onClick={() => setExpanded(allFolders(index))}>
                    Expand all
                </button>
                <button type="button" className={buttonPlain} onClick={() => setExpanded(new Set())}>
                    Collapse all
                </button>
            </div>

            <div className="flex flex-wrap items-center gap-1.5 text-xs" role="group" aria-label="Include rules">
                <button type="button" className={buttonPlain} onClick={() => setChecked(selectAll(index))}>
                    All
                </button>
                <button type="button" className={buttonPlain} onClick={() => setChecked(new Set())}>
                    None
                </button>
                <button type="button" className={buttonPlain} onClick={() => setChecked(invert(index, checked))}>
                    Invert
                </button>
                {PRESETS.map((p) => (
                    <button key={p.id} type="button" className={buttonPlain} onClick={() => setChecked(applyPreset(index, p))}>
                        {p.label}
                    </button>
                ))}
            </div>

            <div className="flex flex-wrap items-center gap-3 text-xs text-muted">
                <span>Columns:</span>
                {(['size', 'type'] as const).map((c) => (
                    <label key={c} className="flex items-center gap-1">
                        <input
                            type="checkbox"
                            checked={layout.columns[c]}
                            onChange={(e) => setLayout({ ...layout, columns: { ...layout.columns, [c]: e.target.checked } })}
                        />
                        {c === 'size' ? 'Size' : 'Type'}
                    </label>
                ))}
                <span className="ml-auto">Sort:</span>
                {(['name', 'size', 'type'] as const).map((k) => (
                    <button
                        key={k}
                        type="button"
                        onClick={() => sortBy(k)}
                        aria-pressed={layout.sort.key === k}
                        className={layout.sort.key === k ? 'font-semibold text-foreground' : ''}
                    >
                        {k}
                        {layout.sort.key === k ? (layout.sort.dir === 'asc' ? ' ▲' : ' ▼') : ''}
                    </button>
                ))}
            </div>

            <div
                role="tree"
                aria-label="Files"
                aria-multiselectable="true"
                tabIndex={0}
                onKeyDown={onKey}
                className="max-h-80 overflow-y-auto rounded-xl border border-border bg-surface-2 py-1 text-sm"
            >
                {rows.length === 0 && <p className="p-3 text-xs text-muted">Nothing matches.</p>}
                {rows.map((row, i) => {
                    const state = stateOf(index, checked, row.node)

                    return (
                        <div
                            key={row.node.path}
                            role="treeitem"
                            aria-level={row.depth + 1}
                            aria-selected={state === 'all'}
                            aria-checked={state === 'some' ? 'mixed' : state === 'all'}
                            aria-expanded={row.open ?? undefined}
                            data-focused={i === at || undefined}
                            style={{ paddingLeft: `${row.depth * 16 + 8}px` }}
                            className={`flex items-center gap-2 py-0.5 pr-2 ${i === at ? 'bg-surface' : ''}`}
                            onClick={() => setFocus(i)}
                        >
                            {row.node.isDir ? (
                                <button
                                    type="button"
                                    tabIndex={-1}
                                    aria-label={row.open ? 'Collapse' : 'Expand'}
                                    onClick={() => open(row.node.path, !row.open)}
                                >
                                    {row.open ? <FiChevronDown className="size-3.5" /> : <FiChevronRight className="size-3.5" />}
                                </button>
                            ) : (
                                <span className="w-3.5" />
                            )}
                            <input
                                type="checkbox"
                                tabIndex={-1}
                                aria-label={`Include ${row.node.name}`}
                                checked={state === 'all'}
                                ref={(el) => {
                                    if (el) el.indeterminate = state === 'some'
                                }}
                                onChange={() => setChecked(toggle(index, checked, row.node))}
                            />
                            {row.node.isDir ? <FiFolder aria-hidden className="size-3.5 text-muted" /> : <FiFile aria-hidden className="size-3.5 text-muted" />}
                            <span className="min-w-0 flex-1 truncate">{row.node.name}</span>
                            {layout.columns.type && <span className="w-12 text-xs text-muted">{row.node.isDir ? '' : extOf(row.node.name)}</span>}
                            {layout.columns.size && (
                                <span className="w-16 text-right text-xs text-muted">
                                    {row.node.isDir ? '' : formatBytes(row.node.size)}
                                </span>
                            )}
                        </div>
                    )
                })}
            </div>

            {tree.truncated && (
                <p className="text-xs text-warning">
                    This folder is large; only the first {tree.nodes.length} entries are listed.
                </p>
            )}

            <div className="flex items-center justify-between gap-2">
                <p aria-live="polite" className="text-xs text-muted">
                    {sum.files} of {sum.total} files selected ({formatBytes(sum.bytes)})
                </p>
                <div className="flex gap-2">
                    <button type="button" className={buttonPlain} onClick={onCancel}>
                        Cancel
                    </button>
                    <button
                        type="button"
                        disabled={busy || sum.files === 0}
                        className={buttonPrimary}
                        onClick={() => onConfirm(includedPaths(index, checked))}
                    >
                        Install {sum.files} {sum.files === 1 ? 'file' : 'files'}
                    </button>
                </div>
            </div>
        </div>
    )
}
