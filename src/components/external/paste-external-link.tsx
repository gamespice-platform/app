import { useId, useState } from 'react'
import { FiLink } from 'react-icons/fi'

import { STEP_TEXT, type IntakeState } from '~/lib/external/intake'
import { extractLinks } from '~/lib/external/links'

import { buttonPlain, buttonPrimary } from './modal'

/**
 * A field for a pasted mod link. Presentational: the caller passes the intake
 * state and callbacks (from `useExternalIntake`), so it mounts anywhere.
 */
export function PasteExternalLink({
    state,
    onSubmit,
    onUndo,
    onDismiss,
    initial = '',
}: {
    state: IntakeState
    onSubmit: (link: string) => void
    onUndo: () => void
    onDismiss: () => void
    initial?: string
}) {
    const id = useId()
    const [value, setValue] = useState(initial)
    const busy = state.phase === 'working'
    const link = extractLinks(value)[0] ?? (value.trim() || null)

    return (
        <form
            className="flex flex-col gap-2"
            onSubmit={(e) => {
                e.preventDefault()

                if (link && !busy) onSubmit(link)
            }}
        >
            <label htmlFor={id} className="text-sm font-medium">
                Add a mod from a link
            </label>
            <div className="flex gap-2">
                <div className="relative flex-1">
                    <FiLink aria-hidden className="absolute left-2.5 top-2.5 size-4 text-muted" />
                    <input
                        id={id}
                        type="text"
                        value={value}
                        disabled={busy}
                        onChange={(e) => setValue(e.target.value)}
                        placeholder="Paste a Modrinth, Thunderstore, CurseForge, Nexus or GitHub link"
                        className="w-full rounded-lg border border-border bg-surface-2 py-2 pl-8 pr-2 text-sm"
                    />
                </div>
                <button type="submit" disabled={!link || busy} className={buttonPrimary}>
                    Add
                </button>
            </div>

            <div aria-live="polite" className="text-xs">
                {state.phase === 'working' && (
                    <div className="flex flex-col gap-1">
                        <span className="text-muted">{state.label || STEP_TEXT[state.step]}</span>
                        <div
                            role="progressbar"
                            aria-label={state.label}
                            aria-valuetext={state.label}
                            className="h-1.5 overflow-hidden rounded-full bg-surface-2"
                        >
                            <div className="h-full w-1/3 animate-pulse rounded-full bg-accent" />
                        </div>
                    </div>
                )}
                {state.phase === 'done' && (
                    <div className="flex items-center gap-2">
                        <span>
                            Added {state.name} ({state.files} {state.files === 1 ? 'file' : 'files'}).
                        </span>
                        <button type="button" onClick={onUndo} className={buttonPlain}>
                            Undo
                        </button>
                        <button type="button" onClick={onDismiss} className="text-muted underline">
                            Dismiss
                        </button>
                    </div>
                )}
                {state.phase === 'openPage' && (
                    <span className="text-warning">
                        {state.reason} Open it in your browser: {state.url}
                    </span>
                )}
                {state.phase === 'error' && (
                    <span role="alert" className="text-danger">
                        {state.message}
                    </span>
                )}
            </div>
        </form>
    )
}
