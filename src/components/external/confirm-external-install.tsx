import { useEffect, useState } from 'react'

import { externalIpc } from '~/lib/external/ipc'
import { confirmFacts } from '~/lib/external/intake'
import { describeSize } from '~/lib/external/links'
import type { PendingT, ResolvedExternalSchema } from '~/lib/external/schemas'
import { messageOf } from '~/lib/ipc'
import type { z } from 'zod'

import { buttonPlain, buttonPrimary, Modal } from './modal'

type Lookup = z.infer<typeof ResolvedExternalSchema>
type Target = { appId?: number; sandboxId?: number }

/**
 * Shown for a link that arrived from outside the app (`tmc://external-link`).
 * Nothing is downloaded until the user presses Install: an OS link may SHOW,
 * never DO. The lookup here only reads metadata.
 */
export function ConfirmExternalInstall({
    pending,
    onInstall,
    onCancel,
    preview,
}: {
    pending: Extract<PendingT, { kind: 'confirm' }>
    onInstall: (target: Target) => void
    onCancel: () => void
    /** Tests and storybooks supply the lookup; the app leaves it out. */
    preview?: Lookup
}) {
    const [lookup, setLookup] = useState<Lookup | null>(preview ?? null)
    const [error, setError] = useState<string | null>(null)
    const [choice, setChoice] = useState(0)

    useEffect(() => {
        if (preview) return

        let live = true

        externalIpc
            .resolve(pending.url)
            .then((r) => live && setLookup(r))
            .catch((e) => live && setError(messageOf(e)))

        return () => {
            live = false
        }
    }, [pending.url, preview])

    const facts = confirmFacts(pending)
    const resolved = lookup?.resolved
    const candidates = lookup?.candidates ?? []
    const picked = candidates[choice]
    const bytes = resolved?.files.reduce((n, f) => n + (f.size ?? 0), 0)

    if (resolved) {
        facts[1] = { label: 'Project', value: resolved.title }
        facts[2] = { label: 'Version', value: resolved.version }
    }

    return (
        <Modal
            title="Install this mod?"
            subtitle="A link opened GameSpice. Nothing is downloaded until you choose Install."
            onClose={onCancel}
            footer={
                <>
                    <button type="button" onClick={onCancel} className={buttonPlain}>
                        Cancel
                    </button>
                    <button
                        type="button"
                        autoFocus
                        disabled={!resolved || resolved.manualOnly !== null}
                        onClick={() => onInstall({ appId: picked?.appId, sandboxId: picked?.sandboxId ?? undefined })}
                        className={buttonPrimary}
                    >
                        Install
                    </button>
                </>
            }
        >
            <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-sm">
                {facts.map((f) => (
                    <div key={f.label} className="contents">
                        <dt className="text-muted">{f.label}</dt>
                        <dd className="break-words">{f.value}</dd>
                    </div>
                ))}
                {resolved && (
                    <>
                        <dt className="text-muted">Files</dt>
                        <dd>
                            {resolved.files.map((f) => f.name).join(', ') || 'None listed'}
                            {' · '}
                            {describeSize(bytes || null)}
                        </dd>
                    </>
                )}
            </dl>

            {!resolved && !error && <p className="mt-3 text-xs text-muted">Looking it up...</p>}
            {error && (
                <p role="alert" className="mt-3 text-xs text-danger">
                    {error}
                </p>
            )}
            {resolved?.manualOnly && (
                <p className="mt-3 text-xs text-warning">
                    This provider does not allow the app to download it. Open {resolved.manualOnly} in your browser.
                </p>
            )}

            {candidates.length > 0 && (
                <fieldset className="mt-4">
                    <legend className="mb-1 text-xs font-medium">Install into</legend>
                    {candidates.map((c, i) => (
                        <label key={`${c.appId}:${c.sandboxId ?? 'none'}`} className="flex items-start gap-2 py-1 text-sm">
                            <input
                                type="radio"
                                name="external-target"
                                checked={choice === i}
                                onChange={() => setChoice(i)}
                            />
                            <span>
                                {c.gameName}
                                {c.sandboxName ? ` / ${c.sandboxName}` : ' / this device'}
                                {c.warnings.map((w) => (
                                    <span key={w} className="block text-xs text-warning">
                                        {w}
                                    </span>
                                ))}
                            </span>
                        </label>
                    ))}
                </fieldset>
            )}
        </Modal>
    )
}
