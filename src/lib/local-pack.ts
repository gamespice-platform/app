import type { DownloadT, PackFinalizedT } from '~/lib/ipc/schemas'

/**
 * Where a modpack's queued downloads stand.
 *
 * Rust tags every row it queues for a pack with `meta.pack = <local mod id>`
 * (and the stable id `pack<id>:<n>`), so the existing queue is the single
 * source of progress; nothing here polls.
 */
export type PackProgress = {
    total: number
    /** Finished successfully. */
    done: number
    failed: number
    cancelled: number
    /** Still queued, running or paused. */
    open: number
    bytesDone: number
    /** Sum of known totals; null while any row has not reported one. */
    bytesTotal: number | null
    /** 0..1 by bytes where known, else by file count. */
    fraction: number
}

export function packRows(downloads: readonly DownloadT[], modId: number): DownloadT[] {
    const key = String(modId)

    return downloads.filter((d) => d.meta.pack === key)
}

export function packProgress(
    downloads: readonly DownloadT[],
    modId: number
): PackProgress {
    const rows = packRows(downloads, modId)

    let done = 0
    let failed = 0
    let cancelled = 0
    let bytesDone = 0
    let bytesTotal: number | null = 0

    for (const row of rows) {
        if (row.status === 'done') done += 1
        else if (row.status === 'failed') failed += 1
        else if (row.status === 'cancelled') cancelled += 1

        bytesDone += row.status === 'done' && row.total ? row.total : row.done

        if (bytesTotal !== null) {
            bytesTotal = row.total === null ? null : bytesTotal + row.total
        }
    }

    const open = rows.length - done - failed - cancelled

    const fraction =
        rows.length === 0
            ? 0
            : bytesTotal
              ? Math.min(1, bytesDone / bytesTotal)
              : done / rows.length

    return { total: rows.length, done, failed, cancelled, open, bytesDone, bytesTotal, fraction }
}

/** Finalizing is worth asking for once every queued row has ended. */
export function canFinalize(progress: PackProgress): boolean {
    return progress.total > 0 && progress.open === 0
}

/** One line for what `local_pack_finalize` reported. */
export function finalizeSummary(result: PackFinalizedT): string {
    const parts = [`${result.placed} placed`]

    if (result.pending > 0) parts.push(`${result.pending} not downloaded yet`)

    if (result.failed.length > 0) {
        parts.push(`${result.failed.length} failed the hash check and were removed`)
    }

    return parts.join(', ')
}
