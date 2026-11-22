import { useEffect, useRef } from 'react'

import { EXTERNAL_LINK_EVENT } from '~/lib/external/ipc'
import { extractLinks, isEditable } from '~/lib/external/links'
import { PendingSchema } from '~/lib/external/schemas'
import { subscribe } from '~/lib/ipc'
import { DropBatchSchema } from '~/lib/ipc/schemas'

import { ConfirmExternalInstall } from './confirm-external-install'
import { FileSelectionPicker } from './file-selection-picker'
import { buttonPlain, Modal } from './modal'
import { useExternalIntake } from './use-external-intake'

/**
 * Mount once, above the router. It turns three things into the intake flow:
 *
 * - a pasted link anywhere that is not a text field: installed directly (the
 *   user did it), with Undo;
 * - a `tmc://external-link` deep link: the confirm dialog, never a silent install;
 * - when `handleDrops` is true, a dropped folder or zip (first file of a batch).
 *   Leave it false while `DropImport` owns drops, or both would act on one drop.
 */
export function ExternalIntakeHost({
    handleDrops = false,
    onChanged,
    target,
}: {
    handleDrops?: boolean
    onChanged?: () => void
    target?: { appId?: number; sandboxId?: number }
}) {
    const intake = useExternalIntake(onChanged)
    const latest = useRef(intake)
    const where = useRef(target)

    useEffect(() => {
        latest.current = intake
        where.current = target
    })

    useEffect(() => {
        const onPaste = (e: ClipboardEvent) => {
            if (isEditable(e.target)) return

            const link = extractLinks(e.clipboardData?.getData('text') ?? '')[0]

            if (link) void latest.current.submitLink(link, where.current)
        }

        window.addEventListener('paste', onPaste)

        return () => window.removeEventListener('paste', onPaste)
    }, [])

    useEffect(() => {
        let live = true
        const stops: (() => void)[] = []
        const keep = (p: Promise<() => void>) =>
            p.then((stop) => (live ? stops.push(stop) : stop())).catch(() => undefined)

        void keep(
            subscribe(EXTERNAL_LINK_EVENT, PendingSchema, (pending) => {
                if (!live) return

                if (pending.kind === 'confirm') latest.current.offer(pending)
            })
        )

        if (handleDrops) {
            void keep(
                subscribe('tmc://files-dropped', DropBatchSchema, (batch) => {
                    const first = batch.files[0]

                    if (live && first) void latest.current.submitPath(first.token, first.name, where.current)
                })
            )
        }

        return () => {
            live = false
            stops.forEach((s) => s())
        }
    }, [handleDrops])

    const { state } = intake

    return (
        <>
            {state.phase === 'confirm' && (
                <ConfirmExternalInstall
                    pending={state.pending}
                    onCancel={intake.dismiss}
                    onInstall={(t) => void intake.accept({ ...target, ...t })}
                />
            )}
            {state.phase === 'pick' && (
                <Modal title="Choose files to install" subtitle={state.name} onClose={intake.dismiss} wide>
                    <FileSelectionPicker
                        tree={state.tree}
                        onCancel={intake.dismiss}
                        onConfirm={(paths) => void intake.submitSelection(paths, target)}
                    />
                </Modal>
            )}
            {(state.phase === 'working' || state.phase === 'done' || state.phase === 'error' || state.phase === 'openPage') && (
                <div
                    role="status"
                    aria-live="polite"
                    className="fixed bottom-4 right-4 z-50 flex max-w-sm items-center gap-2 rounded-xl border border-border bg-surface p-3 text-sm shadow-lg"
                >
                    {state.phase === 'working' && <span>{state.label}...</span>}
                    {state.phase === 'done' && (
                        <>
                            <span>Added {state.name}.</span>
                            <button type="button" className={buttonPlain} onClick={() => void intake.undo()}>
                                Undo
                            </button>
                        </>
                    )}
                    {state.phase === 'error' && <span className="text-danger">{state.message}</span>}
                    {state.phase === 'openPage' && (
                        <span className="text-warning">
                            {state.reason} {state.url}
                        </span>
                    )}
                    {state.phase !== 'working' && (
                        <button type="button" aria-label="Dismiss" className="text-muted" onClick={intake.dismiss}>
                            ×
                        </button>
                    )}
                </div>
            )}
        </>
    )
}

