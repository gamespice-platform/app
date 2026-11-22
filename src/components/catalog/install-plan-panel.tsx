import { useEffect, useState } from 'react'

import { messageOf } from '~/lib/ipc'
import { ipc } from '~/lib/ipc/commands'
import type { InstallPlanPreviewT, InstallPlanT } from '~/lib/ipc/schemas'
import { planSteps, type Piece, type StepView } from '~/lib/catalog/picker'
import { ct } from '~/lib/catalog/text'
import { StepLabel } from './step-label'

/**
 * "What installing this does": the install plan of a sandbox's current game
 * version and loader, as `install_plan_preview` resolves it, step by step
 * with the full argv of any program it runs. The same list the consent
 * dialog shows, so nothing in the dialog is a surprise.
 *
 * `watch` re-asks whenever it changes (the fields the plan depends on).
 */
export default function InstallPlanPanel({
    sandboxId,
    watch,
}: {
    sandboxId: number
    watch: string
}) {
    const [state, setState] = useState<
        | { kind: 'loading' }
        | { kind: 'ready'; preview: InstallPlanPreviewT }
        | { kind: 'error'; message: string }
    >({ kind: 'loading' })

    useEffect(() => {
        let live = true
        void (async () => {
            try {
                const preview = await ipc.installPlanPreview(sandboxId)
                if (live) setState({ kind: 'ready', preview })
            } catch (err) {
                if (live) setState({ kind: 'error', message: messageOf(err) })
            }
        })()
        return () => {
            live = false
        }
    }, [sandboxId, watch])

    return (
        <details
            data-testid="install-plan"
            className="rounded-lg border border-border bg-surface-2 px-3 py-2 text-xs"
        >
            <summary className="cursor-pointer select-none text-sm font-medium">
                {ct('catalog.plan.title')}
            </summary>
            <div className="mt-2">
                {state.kind === 'loading' && (
                    <p className="text-muted">{ct('catalog.plan.loading')}</p>
                )}
                {state.kind === 'error' && (
                    <p role="alert" className="text-danger">
                        {state.message}
                    </p>
                )}
                {state.kind === 'ready' &&
                    (state.preview.plan && state.preview.plan.layers.length > 0 ? (
                        <PlanSteps plan={state.preview.plan} />
                    ) : (
                        <p className="text-muted">{ct('catalog.plan.nothing')}</p>
                    ))}
            </div>
        </details>
    )
}

/** Every step of `plan`, in run order, grouped by layer; also the consent dialog's body. */
export function PlanSteps({ plan }: { plan: InstallPlanT }) {
    return (
        <div className="flex flex-col gap-3">
            {planSteps(plan).map(({ layer, steps }) => (
                <section key={layer.manifestKey} className="flex flex-col gap-1.5">
                    <h4 className="text-xs font-semibold">
                        {layer.title}{' '}
                        <span className="font-normal text-muted">
                            {layer.manifestKey} r{layer.revision}
                        </span>
                    </h4>
                    {layer.downloadHosts.length > 0 && (
                        <p className="text-muted">
                            {ct('catalog.plan.hosts', {
                                hosts: layer.downloadHosts.join(', '),
                            })}
                        </p>
                    )}
                    <ol className="flex flex-col gap-1.5">
                        {steps.map(({ n, view }) => (
                            <Step key={n} n={n} view={view} />
                        ))}
                    </ol>
                </section>
            ))}
            {plan.runtimes.length > 0 && (
                <ul className="flex flex-col gap-1 text-muted">
                    {plan.runtimes.map((r) => (
                        <li key={`${r.kind}-${r.major}-${r.for}`}>
                            {ct('catalog.plan.runtime', {
                                kind: r.kind,
                                major: r.major,
                                purpose:
                                    r.for === 'install'
                                        ? ct('catalog.plan.runtimeInstall')
                                        : ct('catalog.plan.runtimeLaunch'),
                            })}
                        </li>
                    ))}
                </ul>
            )}
        </div>
    )
}

function Step({ n, view }: { n: number; view: StepView }) {
    return (
        <li
            data-testid="plan-step"
            className="rounded-md border border-border bg-surface px-2 py-1.5"
        >
            <div className="flex flex-wrap items-baseline gap-x-2">
                <span className="text-muted">{ct('catalog.plan.step', { n })}</span>
                <StepLabel action={view.action} />
                {view.target && (
                    <code className="selectable break-all font-mono">
                        <Pieces pieces={view.target} />
                    </code>
                )}
            </div>
            {view.argv && (
                <div className="mt-1">
                    <p className="text-muted">{ct('catalog.plan.argv')}</p>
                    <ol
                        data-testid="plan-argv"
                        className="selectable mt-0.5 rounded bg-background px-2 py-1 font-mono"
                    >
                        {view.argv.map((arg, i) => (
                            <li key={i} className="break-all">
                                <Pieces pieces={arg} />
                            </li>
                        ))}
                    </ol>
                    {view.cwd && (
                        <p className="mt-0.5 font-mono text-muted">
                            cwd <Pieces pieces={view.cwd} />
                        </p>
                    )}
                </div>
            )}
            <details className="mt-1">
                <summary className="cursor-pointer text-muted">
                    {ct('catalog.plan.details')}
                </summary>
                <pre className="selectable mt-1 overflow-x-auto rounded bg-background p-2 font-mono text-[0.7rem]">
                    {JSON.stringify(view.raw, null, 2)}
                </pre>
            </details>
        </li>
    )
}

/** Plain text as is; a placeholder the install fills in, set apart. */
function Pieces({ pieces }: { pieces: Piece[] }) {
    return (
        <>
            {pieces.map((p, i) =>
                p.token ? (
                    <span key={i} className="text-accent">
                        {p.text}
                    </span>
                ) : (
                    <span key={i}>{p.text}</span>
                )
            )}
        </>
    )
}
