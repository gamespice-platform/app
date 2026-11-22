import { useCallback, useState } from 'react'
import { FiAlertTriangle, FiCheck, FiDownload, FiPackage } from 'react-icons/fi'

import { useDownloads, formatBytes } from '~/lib/downloads/provider'
import { messageOf } from '~/lib/ipc'
import { ipc } from '~/lib/ipc/commands'
import { canFinalize, finalizeSummary, packProgress } from '~/lib/local-pack'
import type { PackFinalizedT, PackInfoT } from '~/lib/ipc/schemas'

/**
 * **A modpack the import recognised, and what is left to fetch.**
 *
 * The overrides are already in the mod. What the manifest listed is NOT: the
 * user starts the downloads (`local_pack_download`), watches them in the same
 * queue as everything else, and finalizes (`local_pack_finalize`) once they
 * end, which hashes each staged file and moves it in only on a match.
 *
 * CurseForge files are listed and left alone. Turning a `projectID/fileID` pair
 * into a URL needs the keyed CurseForge API, which this app does not call; the
 * honest thing is to name what is missing so the user can fetch it themselves.
 */
export default function PackPanel({ pack }: { pack: PackInfoT }) {
    const { downloads } = useDownloads()

    const [queued, setQueued] = useState<number | null>(null)
    const [result, setResult] = useState<PackFinalizedT | null>(null)
    const [busy, setBusy] = useState(false)
    const [error, setError] = useState<string | null>(null)

    const progress = packProgress(downloads, pack.modId)

    const start = useCallback(async () => {
        setBusy(true)
        setError(null)

        try {
            setQueued(await ipc.localPackDownload(pack.modId))
        } catch (err) {
            setError(messageOf(err))
        } finally {
            setBusy(false)
        }
    }, [pack.modId])

    const finalize = useCallback(async () => {
        setBusy(true)
        setError(null)

        try {
            setResult(await ipc.localPackFinalize(pack.modId))
        } catch (err) {
            setError(messageOf(err))
        } finally {
            setBusy(false)
        }
    }, [pack.modId])

    const kindLabel = pack.kind === 'mrpack' ? 'Modrinth pack' : 'CurseForge pack'

    const meta = [kindLabel, pack.version, pack.gameVersion, pack.loader].filter(
        (part): part is string => Boolean(part)
    )

    return (
        <div className="rounded-lg border border-border bg-surface-2 p-3">
            <div className="flex items-start gap-2.5">
                <FiPackage className="mt-0.5 size-4 shrink-0 text-accent" />

                <div className="min-w-0 flex-1">
                    <p className="truncate text-xs font-semibold">{pack.name}</p>
                    <p className="text-[11px] text-muted">{meta.join(' · ')}</p>
                </div>
            </div>

            {pack.files > 0 && (
                <div className="mt-3 flex flex-col gap-2">
                    <p className="text-[11px] text-muted">
                        {pack.files} file{pack.files === 1 ? '' : 's'} to download
                        {pack.skippedServerOnly > 0
                            ? `, ${pack.skippedServerOnly} server-only skipped`
                            : ''}
                        . Each is checked against the pack's SHA-512 before it goes
                        into the mod.
                    </p>

                    {progress.total > 0 && (
                        <div>
                            <div
                                className="h-1.5 overflow-hidden rounded-full bg-surface"
                                role="progressbar"
                                aria-label={`Downloading ${pack.name}`}
                                aria-valuemin={0}
                                aria-valuemax={100}
                                aria-valuenow={Math.round(progress.fraction * 100)}
                            >
                                <div
                                    className={
                                        progress.failed > 0
                                            ? 'h-full bg-danger'
                                            : 'h-full bg-accent'
                                    }
                                    style={{ width: `${progress.fraction * 100}%` }}
                                />
                            </div>

                            <p className="mt-1 text-[11px] text-muted">
                                {progress.done} of {progress.total} downloaded
                                {progress.failed > 0
                                    ? `, ${progress.failed} failed`
                                    : ''}
                                {progress.bytesTotal !== null
                                    ? ` · ${formatBytes(progress.bytesDone)} of ${formatBytes(progress.bytesTotal)}`
                                    : ''}
                            </p>
                        </div>
                    )}

                    <div className="flex flex-wrap gap-2">
                        <button
                            type="button"
                            disabled={busy || progress.open > 0}
                            onClick={() => void start()}
                            className="flex items-center gap-1.5 rounded-lg bg-accent px-3 py-1.5 text-xs text-accent-foreground disabled:opacity-50"
                        >
                            <FiDownload className="size-3.5" />
                            {progress.total > 0 ? 'Download again' : 'Download files'}
                        </button>

                        <button
                            type="button"
                            disabled={busy || !canFinalize(progress)}
                            onClick={() => void finalize()}
                            className="flex items-center gap-1.5 rounded-lg border border-border px-3 py-1.5 text-xs disabled:opacity-50"
                        >
                            <FiCheck className="size-3.5" />
                            Verify and add to the mod
                        </button>

                        <ExportSandboxSlot />
                    </div>

                    {queued === 0 && (
                        <p className="text-[11px] text-muted">
                            Nothing was queued: the pack lists no downloads.
                        </p>
                    )}
                </div>
            )}

            {pack.unresolved.length > 0 && (
                <div className="mt-3 rounded-lg border border-warning/40 bg-warning/10 p-2.5 text-[11px] text-warning">
                    <p className="flex items-start gap-2">
                        <FiAlertTriangle className="mt-0.5 size-3.5 shrink-0" />
                        <span>
                            {pack.unresolved.length} CurseForge file
                            {pack.unresolved.length === 1 ? '' : 's'} cannot be
                            downloaded by this app, which does not use the
                            CurseForge API. Fetch them yourself and add them as a
                            mod; the pack's own overrides are already imported.
                        </span>
                    </p>

                    <ul className="mt-2 max-h-28 overflow-y-auto font-mono">
                        {pack.unresolved.map((id) => (
                            <li key={id}>{id}</li>
                        ))}
                    </ul>
                </div>
            )}

            {result && (
                <p
                    className={`mt-2 text-[11px] ${result.failed.length > 0 ? 'text-danger' : 'text-muted'}`}
                >
                    {finalizeSummary(result)}
                    {result.failed.length > 0 && `: ${result.failed.join(', ')}`}
                </p>
            )}

            {error && (
                <p className="mt-2 rounded-lg border border-danger/40 bg-danger/10 p-2 text-[11px] text-danger">
                    {error}
                </p>
            )}
        </div>
    )
}

/**
 * HOOK(export-sandbox): the "export sandbox as a pack" command is being added
 * on the Rust side. Its button belongs HERE, beside the pack actions, and on the
 * sandbox editor's menu; render it from this component and add `sandboxExport`
 * to `lib/ipc/commands.ts`. Deliberately renders nothing until then.
 */
function ExportSandboxSlot() {
    return null
}
