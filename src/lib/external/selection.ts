import type { FileTreeT } from './schemas'

/**
 * The model behind the file-selection picker: which files of a dropped folder
 * or zip are ticked. Pure, so it is tested without a DOM.
 */

export type Node = FileTreeT['nodes'][number]
export type CheckState = 'all' | 'some' | 'none'
export type SortKey = 'name' | 'size' | 'type'
export type SortDir = 'asc' | 'desc'

export type Index = {
    nodes: Node[]
    byPath: Map<string, Node>
    /** Direct children of a folder path ('' = root), in tree order. */
    children: Map<string, Node[]>
    /** Every file path under a folder path. */
    files: Map<string, string[]>
}

export function parentOf(path: string): string {
    const i = path.lastIndexOf('/')

    return i < 0 ? '' : path.slice(0, i)
}

export function extOf(name: string): string {
    const i = name.lastIndexOf('.')

    return i <= 0 ? '' : name.slice(i + 1).toLowerCase()
}

export function buildIndex(tree: FileTreeT): Index {
    const byPath = new Map<string, Node>()
    const children = new Map<string, Node[]>()
    const files = new Map<string, string[]>()

    for (const n of tree.nodes) {
        byPath.set(n.path, n)

        const p = parentOf(n.path)
        const list = children.get(p) ?? []

        list.push(n)
        children.set(p, list)

        if (!n.isDir) {
            // Every ancestor (and the root) owns this file.
            let a = p

            for (;;) {
                const l = files.get(a) ?? []

                l.push(n.path)
                files.set(a, l)

                if (a === '') break

                a = parentOf(a)
            }
        }
    }

    return { nodes: tree.nodes, byPath, children, files }
}

export function stateOf(index: Index, checked: ReadonlySet<string>, node: Node): CheckState {
    if (!node.isDir) return checked.has(node.path) ? 'all' : 'none'

    const under = index.files.get(node.path) ?? []

    if (under.length === 0) return 'none'

    let n = 0

    for (const f of under) if (checked.has(f)) n++

    return n === 0 ? 'none' : n === under.length ? 'all' : 'some'
}

/** Tick or untick a node; a folder takes everything under it. */
export function toggle(index: Index, checked: ReadonlySet<string>, node: Node): Set<string> {
    const next = new Set(checked)
    const targets = node.isDir ? (index.files.get(node.path) ?? []) : [node.path]
    const on = stateOf(index, checked, node) !== 'all'

    for (const f of targets) {
        if (on) next.add(f)
        else next.delete(f)
    }

    return next
}

export const allFiles = (index: Index): string[] => index.files.get('') ?? []

export function selectAll(index: Index): Set<string> {
    return new Set(allFiles(index))
}

export function invert(index: Index, checked: ReadonlySet<string>): Set<string> {
    return new Set(allFiles(index).filter((f) => !checked.has(f)))
}

export type Preset = {
    id: string
    label: string
    /** Lower-case extensions without the dot, or path segments (`plugins/`). */
    extensions?: string[]
    folders?: string[]
}

export const PRESETS: Preset[] = [
    { id: 'binaries', label: 'Only .dll / .jar', extensions: ['dll', 'jar'] },
    { id: 'plugins', label: 'Only plugins/', folders: ['plugins'] },
    { id: 'scripts', label: 'Only scripts', extensions: ['cs', 'lua', 'js', 'py', 'gd', 'cfg'] },
    { id: 'assets', label: 'Only textures / models', extensions: ['png', 'jpg', 'dds', 'vtf', 'vmt', 'mdl', 'obj', 'fbx', 'glb'] },
]

export function applyPreset(index: Index, preset: Preset): Set<string> {
    const exts = new Set(preset.extensions ?? [])
    const folders = preset.folders ?? []

    return new Set(
        allFiles(index).filter((f) => {
            const node = index.byPath.get(f)

            if (!node) return false

            if (exts.has(extOf(node.name))) return true

            return folders.some(
                (d) => f === d || f.startsWith(`${d}/`) || f.includes(`/${d}/`)
            )
        })
    )
}

/** Case-insensitive substring match; empty matches everything. */
export function matches(node: Node, query: string): boolean {
    const q = query.trim().toLowerCase()

    return q === '' || node.path.toLowerCase().includes(q)
}

export type Row = { node: Node; depth: number; open: boolean | null }

function compare(key: SortKey, dir: SortDir) {
    const sign = dir === 'asc' ? 1 : -1

    return (a: Node, b: Node) => {
        // Folders always lead, whatever the direction.
        if (a.isDir !== b.isDir) return a.isDir ? -1 : 1

        let c = 0

        if (key === 'size') c = a.size - b.size
        else if (key === 'type') c = extOf(a.name).localeCompare(extOf(b.name))

        if (c !== 0) return c * sign

        // Ties always fall back to name order, whatever the direction.
        return a.name.localeCompare(b.name, undefined, { numeric: true })
    }
}

/**
 * The rows to draw: depth-first, sorted, honouring which folders are open.
 * A search shows every match with its ancestors, all open.
 */
export function visibleRows(
    index: Index,
    expanded: ReadonlySet<string>,
    query: string,
    sort: { key: SortKey; dir: SortDir }
): Row[] {
    const searching = query.trim() !== ''
    const keep = new Set<string>()

    if (searching) {
        for (const n of index.nodes) {
            if (!matches(n, query)) continue

            let p = n.path

            while (p !== '') {
                keep.add(p)
                p = parentOf(p)
            }
        }
    }

    const cmp = compare(sort.key, sort.dir)
    const out: Row[] = []
    const walk = (dir: string, depth: number) => {
        const list = [...(index.children.get(dir) ?? [])].sort(cmp)

        for (const node of list) {
            if (searching && !keep.has(node.path)) continue

            const open = node.isDir ? searching || expanded.has(node.path) : null

            out.push({ node, depth, open })

            if (node.isDir && open) walk(node.path, depth + 1)
        }
    }

    walk('', 0)

    return out
}

export function allFolders(index: Index): Set<string> {
    return new Set(index.nodes.filter((n) => n.isDir).map((n) => n.path))
}

/**
 * The paths to hand to the install call: top-most fully ticked folders plus
 * loose ticked files. The core expands a folder to everything under it.
 */
export function includedPaths(index: Index, checked: ReadonlySet<string>): string[] {
    const out: string[] = []
    const walk = (dir: string) => {
        for (const n of index.children.get(dir) ?? []) {
            if (n.isDir) {
                const s = stateOf(index, checked, n)

                if (s === 'all') out.push(n.path)
                else if (s === 'some') walk(n.path)
            } else if (checked.has(n.path)) {
                out.push(n.path)
            }
        }
    }

    walk('')

    return out
}

export function summarise(index: Index, checked: ReadonlySet<string>) {
    let bytes = 0

    for (const f of checked) bytes += index.byPath.get(f)?.size ?? 0

    return { files: checked.size, bytes, total: allFiles(index).length }
}

export function formatBytes(n: number): string {
    if (n < 1024) return `${n} B`

    const u = ['KB', 'MB', 'GB', 'TB']
    let v = n / 1024
    let i = 0

    while (v >= 1024 && i < u.length - 1) {
        v /= 1024
        i++
    }

    return `${v.toFixed(v >= 10 ? 0 : 1)} ${u[i]}`
}
