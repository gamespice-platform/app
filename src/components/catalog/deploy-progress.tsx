import { useEffect, useRef, useState } from 'react'

import { useDialog } from '~/components/use-dialog'
import { messageOf, subscribe } from '~/lib/ipc'
import {
    PlanStepEventSchema,
    type DeployReportT,
    type InstallPlanT,
} from '~/lib/ipc/schemas'
import { planSteps } from '~/lib/catalog/picker'
import { ct } from '~/lib/catalog/text'
import { StepLabel } from './step-label'

type Status = 'waiting' | 'running' | 'done' | 'skipped' | 'failed'

/** `deploy://plan-step` names a step by its layer and its index in that layer. */
const keyOf = (layer: string, index: number) => `${layer}#${index}`

const STATUS_TEXT: Record<Status, string> = {
    waiting: ct('catalog.progress.waiting'),
    running: ct('catalog.progress.running'),
    done: ct('catalog.progress.stepDone'),
    skipped: ct('catalog.progress.skipped'),
    failed: ct('catalog.progress.stepFailed'),
}

const STATUS_CLASS: Record<Status, string> = {
    waiting: 'text-muted',
    running: 'text-accent',
    done: 'text-foreground',
    skipped: 'text-muted',
    failed: 'text-danger',
}

/**
 * The deploy of a sandbox with an install plan: every step of the plan,
 * marked as `deploy://plan-step` reports it, then done or failed. A failed
 * step keeps its log tail, openable under it.
 *
 * It starts the deploy itself (`deploy`) once it is listening, so no event
 * is missed.
 */
export default function DeployProgress({
    sandboxId,
    plan,
    deploy,
    onClose,
}: {
    sandboxId: number
    plan: InstallPlanT
    deploy: () => Promise<DeployReportT>
    onClose: (report: DeployReportT | null) => void
}) {
    const [steps, setSteps] = useState<Record<string, { status: Status; log: string | null }>>({})
    const [outcome, setOutcome] = useState<'done' | 'failed' | null>(null)
    const [error, setError] = useState<string | null>(null)
    const report = useRef<DeployReportT | null>(null)
    const started = useRef(false)
    const groups = planSteps(plan)
    const ref = useDialog<HTMLDivElement>(() => {
        if (outcome) onClose(report.current)
    })

    useEffect(() => {
        let live = true
        let unlisten: (() => void) | null = null
        void (async () => {
            unlisten = await subscribe('deploy://plan-step', PlanStepEventSchema, (e) => {
                if (!live || e.sandboxId !== sandboxId) return
                setSteps((prev) => ({
                    ...prev,
                    [keyOf(e.layer, e.index)]: { status: e.status, log: e.logTail ?? null },
                }))
            })
            if (!live) {
                unlisten()
                return
            }
            if (started.current) return
            started.current = true
            try {
                report.current = await deploy()
                if (live) setOutcome('done')
            } catch (err) {
                if (!live) return
                setError(messageOf(err))
                setOutcome('failed')
            }
        })()
        return () => {
            live = false
            unlisten?.()
        }
        // The deploy runs once per dialog; `deploy` is a new closure each render.
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [sandboxId])

    const statusOf = (layer: string, index: number): Status => {
        const s = steps[keyOf(layer, index)]?.status
        if (s) return s
        if (outcome === 'done') return 'done'
        return 'waiting'
    }

    // A failure with no step event of its own lands on the first step that
    // never finished, so the list still says where it stopped.
    const failedAt = (() => {
        if (outcome !== 'failed') return null
        for (const g of groups) {
            for (let i = 0; i < g.steps.length; i++) {
                const s = steps[keyOf(g.layer.manifestKey, i)]?.status
                if (s === 'failed') return keyOf(g.layer.manifestKey, i)
                if (s !== 'done' && s !== 'skipped') return keyOf(g.layer.manifestKey, i)
            }
        }
        return null
    })()

    return (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4">
            <div
                ref={ref}
                role="dialog"
                aria-modal="true"
                aria-label={ct('catalog.progress.title')}
                tabIndex={-1}
                data-testid="deploy-progress"
                className="flex max-h-[90vh] w-full max-w-xl flex-col gap-3 rounded-xl border border-border bg-surface p-4 focus:outline-none"
            >
                <h2 className="text-sm font-semibold">{ct('catalog.progress.title')}</h2>
                <ol className="flex min-h-0 flex-col gap-1 overflow-y-auto text-xs">
                    {groups.flatMap(({ layer, steps: list }) =>
                        list.map(({ n, view }, i) => {
                            const key = keyOf(layer.manifestKey, i)
                            const status: Status = key === failedAt ? 'failed' : statusOf(layer.manifestKey, i)
                            const log = steps[key]?.log ?? (status === 'failed' ? error : null)
                            return (
                                <li
                                    key={key}
                                    data-testid="progress-step"
                                    data-status={status}
                                    className="rounded-md border border-border bg-surface-2 px-2 py-1"
                                >
                                    <div className="flex items-baseline gap-2">
                                        <span className="text-muted">{n}.</span>
                                        <StepLabel action={view.action} />
                                        <span className="text-muted">{layer.title}</span>
                                        <span className={`ml-auto ${STATUS_CLASS[status]}`}>
                                            {STATUS_TEXT[status]}
                                        </span>
                                    </div>
                                    {status === 'failed' && (
                                        <details className="mt-1" data-testid="progress-log">
                                            <summary className="cursor-pointer text-danger">
                                                {ct('catalog.progress.log')}
                                            </summary>
                                            <pre className="selectable mt-1 max-h-48 overflow-auto whitespace-pre-wrap rounded bg-background p-2 font-mono text-[0.7rem]">
                                                {log ?? ct('catalog.progress.noLog')}
                                            </pre>
                                        </details>
                                    )}
                                </li>
                            )
                        })
                    )}
                </ol>

                {outcome === 'done' && (
                    <p role="status" className="text-xs">
                        {ct('catalog.progress.done')}
                    </p>
                )}
                {outcome === 'failed' && (
                    <p role="alert" className="text-xs text-danger">
                        {ct('catalog.progress.failed')} {error}
                    </p>
                )}

                <div className="flex justify-end">
                    <button
                        type="button"
                        disabled={outcome === null}
                        onClick={() => onClose(report.current)}
                        className="rounded-lg border border-border px-3 py-1.5 text-sm hover:border-accent disabled:opacity-40"
                    >
                        {ct('catalog.progress.close')}
                    </button>
                </div>
            </div>
        </div>
    )
}
