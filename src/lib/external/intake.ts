import type { FileTreeT, ParsedExternalT, PendingT } from './schemas'

/**
 * The phases of bringing one external mod in, as a plain reducer so the order
 * of events is tested without a window. The hook in
 * `components/external/use-external-intake.ts` drives it.
 */

export type IntakeState =
    | { phase: 'idle' }
    | { phase: 'working'; step: 'parsing' | 'resolving' | 'installing' | 'inspecting'; label: string }
    /** A link from outside the app: nothing happens until the user says Install. */
    | { phase: 'confirm'; pending: Extract<PendingT, { kind: 'confirm' }> }
    | { phase: 'pick'; token: string; name: string; tree: FileTreeT }
    | { phase: 'done'; localId: number; name: string; files: number; sandboxId: number | null }
    | { phase: 'openPage'; url: string; reason: string }
    | { phase: 'error'; message: string }

export type IntakeEvent =
    | { type: 'start'; step: 'parsing' | 'resolving' | 'installing' | 'inspecting'; label: string }
    | { type: 'confirm'; pending: Extract<PendingT, { kind: 'confirm' }> }
    | { type: 'pick'; token: string; name: string; tree: FileTreeT }
    | { type: 'done'; localId: number; name: string; files: number; sandboxId: number | null }
    | { type: 'openPage'; url: string; reason: string }
    | { type: 'fail'; message: string }
    | { type: 'dismiss' }

export const IDLE: IntakeState = { phase: 'idle' }

export function reduce(state: IntakeState, event: IntakeEvent): IntakeState {
    switch (event.type) {
        case 'start':
            return { phase: 'working', step: event.step, label: event.label }
        case 'confirm':
            // Never replace a pending answer the user is looking at.
            return state.phase === 'working' || state.phase === 'pick'
                ? state
                : { phase: 'confirm', pending: event.pending }
        case 'pick':
            return { phase: 'pick', token: event.token, name: event.name, tree: event.tree }
        case 'done':
            return {
                phase: 'done',
                localId: event.localId,
                name: event.name,
                files: event.files,
                sandboxId: event.sandboxId,
            }
        case 'openPage':
            return { phase: 'openPage', url: event.url, reason: event.reason }
        case 'fail':
            return { phase: 'error', message: event.message }
        case 'dismiss':
            return IDLE
    }
}

export const STEP_TEXT: Record<'parsing' | 'resolving' | 'installing' | 'inspecting', string> = {
    parsing: 'Reading the link',
    resolving: 'Looking it up',
    installing: 'Downloading and installing',
    inspecting: 'Looking inside',
}

/** Rows of facts the confirm dialog shows, from what is known before the lookup. */
export function confirmFacts(
    pending: Extract<PendingT, { kind: 'confirm' }>
): { label: string; value: string }[] {
    const r = pending.reference
    const rows: { label: string; value: string }[] = [
        { label: 'Provider', value: pending.label },
        { label: 'Project', value: r.project ?? 'Unknown' },
        { label: 'Version', value: r.version ?? 'Latest' },
    ]

    if (r.game) rows.push({ label: 'Game', value: r.game })

    return rows
}

export function parsedLabel(p: ParsedExternalT): string {
    return `${p.provider.label}: ${p.reference.project ?? p.reference.url}`
}
