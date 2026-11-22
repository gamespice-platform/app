import { useState } from 'react'

import { useDialog } from '~/components/use-dialog'
import { openExternally } from '~/lib/external'
import { messageOf } from '~/lib/ipc'
import type { InstallPlanT } from '~/lib/ipc/schemas'
import { checkboxConsents, execHost, trustLine } from '~/lib/catalog/picker'
import { ct } from '~/lib/catalog/text'
import { PlanSteps } from './install-plan-panel'

/**
 * Asked before a deploy whose plan runs an installer or needs a licence
 * accepted (`install_plan_consent`). It says who published the plan (the
 * trust line and plan hash), what will run and from where, lists every step,
 * and gives each licence its own checkbox. Install stays disabled until
 * every box is ticked. Rust records consent against the plan hash, so a plan
 * that changed while this was open is refused there, not trusted here.
 */
export default function InstallConsentDialog({
    plan,
    onConsent,
    onCancel,
}: {
    plan: InstallPlanT
    onConsent: () => Promise<void>
    onCancel: () => void
}) {
    const ref = useDialog<HTMLDivElement>(onCancel)
    const boxes = checkboxConsents(plan.consents)
    const statements = plan.consents.filter((c) => !boxes.includes(c))
    const [ticked, setTicked] = useState<Set<string>>(new Set())
    const [busy, setBusy] = useState(false)
    const [error, setError] = useState<string | null>(null)

    const title = plan.layers.map((l) => l.title).join(' + ') || plan.appSlug
    const ready = boxes.every((c) => ticked.has(c.key))

    const install = async () => {
        setBusy(true)
        setError(null)
        try {
            await onConsent()
        } catch (err) {
            setError(messageOf(err))
            setBusy(false)
        }
    }

    return (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4">
            <div
                ref={ref}
                role="dialog"
                aria-modal="true"
                aria-label={ct('catalog.consent.title', { title })}
                tabIndex={-1}
                data-testid="install-consent"
                className="flex max-h-[90vh] w-full max-w-2xl flex-col gap-3 rounded-xl border border-border bg-surface p-4 focus:outline-none"
            >
                <h2 className="text-sm font-semibold">
                    {ct('catalog.consent.title', { title })}
                </h2>

                <p
                    data-testid="consent-trust"
                    className={`rounded-lg border px-3 py-2 text-xs ${
                        plan.trust === 'local'
                            ? 'border-warning/50 text-warning'
                            : 'border-border text-muted'
                    }`}
                >
                    {trustLine(plan)}
                </p>

                {plan.exec && (
                    <p
                        data-testid="consent-exec"
                        className="rounded-lg border border-warning/50 bg-surface-2 px-3 py-2 text-xs"
                    >
                        {ct('catalog.consent.exec', { host: execHost(plan) })}
                    </p>
                )}

                {statements
                    .filter((c) => !c.key.startsWith('exec:'))
                    .map((c) => (
                        <p key={c.key} className="text-xs text-muted">
                            {c.text}
                        </p>
                    ))}

                <p className="text-xs text-muted">{ct('catalog.consent.intro')}</p>
                <div className="min-h-0 overflow-y-auto text-xs">
                    <PlanSteps plan={plan} />
                </div>

                {boxes.map((c) => (
                    <label
                        key={c.key}
                        data-testid={`consent-${c.key}`}
                        className="flex items-start gap-2 text-xs"
                    >
                        <input
                            type="checkbox"
                            className="mt-0.5"
                            checked={ticked.has(c.key)}
                            onChange={(e) => {
                                const next = new Set(ticked)
                                if (e.target.checked) next.add(c.key)
                                else next.delete(c.key)
                                setTicked(next)
                            }}
                        />
                        <span>
                            {c.text}{' '}
                            {c.url?.startsWith('https://') && (
                                <button
                                    type="button"
                                    className="text-accent underline"
                                    onClick={() =>
                                        void openExternally({ webUrl: c.url ?? '' })
                                    }
                                >
                                    {ct('catalog.consent.read')}
                                </button>
                            )}
                        </span>
                    </label>
                ))}

                {error && (
                    <p role="alert" className="text-xs text-danger">
                        {error}
                    </p>
                )}

                <div className="flex justify-end gap-2">
                    <button
                        type="button"
                        onClick={onCancel}
                        className="rounded-lg border border-border px-3 py-1.5 text-sm hover:border-accent"
                    >
                        {ct('catalog.consent.cancel')}
                    </button>
                    <button
                        type="button"
                        disabled={!ready || busy}
                        onClick={() => void install()}
                        className="rounded-lg bg-accent px-3 py-1.5 text-sm font-medium text-accent-foreground disabled:opacity-40"
                    >
                        {ct('catalog.consent.install')}
                    </button>
                </div>
            </div>
        </div>
    )
}
