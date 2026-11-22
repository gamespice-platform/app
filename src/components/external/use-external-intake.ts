import { useCallback, useEffect, useReducer, useRef } from 'react'

import { externalIpc } from '~/lib/external/ipc'
import { IDLE, reduce, STEP_TEXT, type IntakeState } from '~/lib/external/intake'
import type { PendingT } from '~/lib/external/schemas'
import { messageOf } from '~/lib/ipc'

export type ExternalIntake = {
    state: IntakeState
    /** The user pasted or typed a link: go straight through, with Undo after. */
    submitLink: (text: string, target?: { appId?: number; sandboxId?: number }) => Promise<void>
    /** A dropped folder or zip, named by its vault token. */
    submitPath: (token: string, name: string, target?: { appId?: number; sandboxId?: number }) => Promise<void>
    /** The picker's answer for a `pick` phase. */
    submitSelection: (selected: string[], target?: { appId?: number; sandboxId?: number }) => Promise<void>
    /** A link from outside the app: show the confirm dialog, do nothing else. */
    offer: (pending: Extract<PendingT, { kind: 'confirm' }>) => void
    /** The user pressed Install on the confirm dialog. */
    accept: (target?: { appId?: number; sandboxId?: number }) => Promise<void>
    undo: () => Promise<void>
    dismiss: () => void
}

/**
 * Drives one external install at a time. Routes-neutral: it imports no route,
 * so any screen (the Mods page, a game page) can mount the same behaviour.
 */
export function useExternalIntake(onChanged?: () => void): ExternalIntake {
    const [state, dispatch] = useReducer(reduce, IDLE)
    const current = useRef(state)

    useEffect(() => {
        current.current = state
    }, [state])

    const finish = useCallback(
        (report: Awaited<ReturnType<typeof externalIpc.install>>, name: string) => {
            const o = report.outcome

            if (o.kind === 'installed') {
                dispatch({ type: 'done', localId: o.localId, name: o.name || name, files: o.files, sandboxId: report.sandboxId })
                onChanged?.()
            } else if (o.kind === 'openPage') {
                dispatch({ type: 'openPage', url: o.url, reason: o.reason })
            } else {
                dispatch({ type: 'fail', message: o.reason })
            }
        },
        [onChanged]
    )

    const submitLink = useCallback<ExternalIntake['submitLink']>(
        async (text, target) => {
            dispatch({ type: 'start', step: 'parsing', label: STEP_TEXT.parsing })

            try {
                const parsed = await externalIpc.parse(text)

                // `install` resolves again itself; one lookup, shown as one step.
                dispatch({ type: 'start', step: 'installing', label: `${STEP_TEXT.installing} (${parsed.provider.label})` })
                finish(await externalIpc.install(text, target), parsed.reference.project ?? text)
            } catch (err) {
                dispatch({ type: 'fail', message: messageOf(err) })
            }
        },
        [finish]
    )

    const submitPath = useCallback<ExternalIntake['submitPath']>(
        async (token, name, target) => {
            dispatch({ type: 'start', step: 'inspecting', label: STEP_TEXT.inspecting })

            try {
                const report = await externalIpc.inspectPath(token, target?.appId)

                if (report.inspection.kind === 'needsSelection') {
                    dispatch({ type: 'pick', token, name, tree: report.inspection.tree })

                    return
                }

                dispatch({ type: 'start', step: 'installing', label: STEP_TEXT.installing })
                finish(
                    await externalIpc.installPath(token, {
                        name: report.inspection.meta.name ?? name,
                        ...target,
                    }),
                    name
                )
            } catch (err) {
                dispatch({ type: 'fail', message: messageOf(err) })
            }
        },
        [finish]
    )

    const submitSelection = useCallback<ExternalIntake['submitSelection']>(
        async (selected, target) => {
            const s = current.current

            if (s.phase !== 'pick') return

            dispatch({ type: 'start', step: 'installing', label: STEP_TEXT.installing })

            try {
                finish(await externalIpc.installPath(s.token, { selected, name: s.name, ...target }), s.name)
            } catch (err) {
                dispatch({ type: 'fail', message: messageOf(err) })
            }
        },
        [finish]
    )

    const offer = useCallback<ExternalIntake['offer']>((pending) => {
        dispatch({ type: 'confirm', pending })
    }, [])

    const accept = useCallback<ExternalIntake['accept']>(
        async (target) => {
            const s = current.current

            if (s.phase !== 'confirm') return

            await submitLink(s.pending.url, target)
        },
        [submitLink]
    )

    const undo = useCallback(async () => {
        const s = current.current

        if (s.phase !== 'done') return

        try {
            await externalIpc.remove(s.localId)
            dispatch({ type: 'dismiss' })
            onChanged?.()
        } catch (err) {
            dispatch({ type: 'fail', message: messageOf(err) })
        }
    }, [onChanged])

    const dismiss = useCallback(() => dispatch({ type: 'dismiss' }), [])

    return { state, submitLink, submitPath, submitSelection, offer, accept, undo, dismiss }
}
