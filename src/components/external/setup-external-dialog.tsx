import { useMemo, useState } from 'react'

import { adviseScheme, planSchemeChanges, recommendedSchemes, SCHEME_APPS, type SchemeChange } from '~/lib/external/scheme-advice'
import type { SchemeReportSchema } from '~/lib/external/schemas'
import type { z } from 'zod'

import { buttonPlain, buttonPrimary, Modal } from './modal'

type Report = z.infer<typeof SchemeReportSchema>

/**
 * Which links should open in GameSpice. `mode="setup"` is the first-run
 * version: it pre-ticks only schemes nothing else handles. `mode="edit"`
 * starts from what is registered now. Either way a scheme another app owns is
 * opt-in, with a warning, and a second explicit tick to replace it. Nothing is
 * ever overwritten silently.
 */
export function SetupExternalDialog({
    reports,
    mode,
    onApply,
    onClose,
    busy = false,
    error,
}: {
    reports: Report[]
    mode: 'setup' | 'edit'
    onApply: (changes: SchemeChange[]) => void | Promise<void>
    onClose: () => void
    busy?: boolean
    error?: string | null
}) {
    const initial = useMemo(
        () =>
            new Set(
                mode === 'setup'
                    ? recommendedSchemes(reports)
                    : reports.filter((r) => adviseScheme(r.scheme, r.status).kind === 'ours').map((r) => r.scheme)
            ),
        [reports, mode]
    )
    const [wanted, setWanted] = useState<Set<string>>(initial)
    const [overrides, setOverrides] = useState<Set<string>>(new Set())
    const flip = (set: Set<string>, key: string, on: boolean) => {
        const next = new Set(set)

        if (on) next.add(key)
        else next.delete(key)

        return next
    }
    const changes = planSchemeChanges(reports, wanted, overrides)

    return (
        <Modal
            title={mode === 'setup' ? 'Open mod links in GameSpice' : 'Link handling'}
            subtitle="Choose which website links open here. You can change this later."
            onClose={onClose}
            footer={
                <>
                    <button type="button" onClick={onClose} className={buttonPlain}>
                        {mode === 'setup' ? 'Not now' : 'Cancel'}
                    </button>
                    <button
                        type="button"
                        disabled={busy || changes.length === 0}
                        onClick={() => void onApply(changes)}
                        className={buttonPrimary}
                    >
                        Apply
                    </button>
                </>
            }
        >
            {error && (
                <p role="alert" className="mb-2 text-xs text-danger">
                    {error}
                </p>
            )}
            <ul className="flex flex-col divide-y divide-border rounded-xl border border-border">
                {reports.map(({ scheme, status }) => {
                    const advice = adviseScheme(scheme, status)
                    const name = SCHEME_APPS[scheme]?.label ?? `${scheme}:// links`
                    const id = `scheme-${scheme}`

                    return (
                        <li key={scheme} className="flex flex-col gap-1 p-3">
                            <label htmlFor={id} className="flex items-center gap-2 text-sm font-medium">
                                <input
                                    id={id}
                                    type="checkbox"
                                    checked={wanted.has(scheme)}
                                    disabled={advice.kind === 'unavailable'}
                                    onChange={(e) => setWanted(flip(wanted, scheme, e.target.checked))}
                                />
                                {name}
                                {advice.kind === 'recommend' && <span className="text-xs font-normal text-accent">Recommended</span>}
                                {advice.kind === 'ours' && <span className="text-xs font-normal text-muted">On</span>}
                            </label>
                            {advice.kind === 'recommend' && (
                                <p className="text-xs text-muted">No other app handles these, so nothing is replaced.</p>
                            )}
                            {advice.kind === 'unavailable' && <p className="text-xs text-muted">{advice.note}</p>}
                            {advice.kind === 'optIn' && (
                                <div className="rounded-lg bg-surface-2 p-2 text-xs">
                                    <p className="text-warning">{advice.warning}</p>
                                    <label className="mt-1 flex items-center gap-2">
                                        <input
                                            type="checkbox"
                                            checked={overrides.has(scheme)}
                                            disabled={!wanted.has(scheme)}
                                            onChange={(e) => setOverrides(flip(overrides, scheme, e.target.checked))}
                                        />
                                        Replace {advice.by} for {scheme}:// links
                                    </label>
                                </div>
                            )}
                        </li>
                    )
                })}
            </ul>
        </Modal>
    )
}

export const EditExternalDialog = (props: Omit<Parameters<typeof SetupExternalDialog>[0], 'mode'>) => (
    <SetupExternalDialog {...props} mode="edit" />
)
