import { useMutation, useQueryClient } from '@tanstack/react-query'
import { FiInfo } from 'react-icons/fi'
import type { ReactNode } from 'react'

import { useDialog } from '~/components/use-dialog'

/** Small shared pieces of the party screens. */

export const BTN_PRIMARY =
    'inline-flex items-center justify-center gap-1.5 rounded-lg bg-accent px-3 py-1.5 text-xs font-semibold text-accent-foreground transition hover:brightness-110 disabled:opacity-50'
export const BTN_GHOST =
    'inline-flex items-center justify-center gap-1.5 rounded-lg border border-border px-3 py-1.5 text-xs font-medium transition hover:border-accent disabled:opacity-50'
export const BTN_DANGER =
    'inline-flex items-center justify-center gap-1.5 rounded-lg border border-danger/60 px-3 py-1.5 text-xs font-medium text-danger transition hover:bg-danger/10 disabled:opacity-50'
export const INPUT =
    'w-full rounded-lg border border-border bg-background px-2 py-1.5 text-sm'

/** A mutation that refreshes the party views (and the friends/play bars) when it settles. */
export function usePartyAction<A = void, R = unknown>(
    run: (arg: A) => Promise<R>,
    onDone?: (result: R) => void
) {
    const client = useQueryClient()

    return useMutation({
        mutationFn: run,
        onSuccess: (result) => onDone?.(result),
        onSettled: () => {
            void client.invalidateQueries({ queryKey: ['social'] })
            void client.invalidateQueries({ queryKey: ['play'] })
        },
    })
}

export function Panel({
    title,
    action,
    children,
}: {
    title: string
    action?: ReactNode
    children: ReactNode
}) {
    return (
        <section className="flex flex-col gap-1.5">
            <div className="flex items-center justify-between gap-2">
                <h2 className="text-[11px] font-semibold uppercase tracking-wide text-muted">
                    {title}
                </h2>
                {action}
            </div>
            <div className="rounded-xl border border-border bg-surface">{children}</div>
        </section>
    )
}

export function ErrorLine({ children }: { children: ReactNode }) {
    return (
        <p role="alert" className="text-[11px] text-danger">
            {children}
        </p>
    )
}

/** What a screen says when the site has no app route for a feature yet. */
export function UnavailableNote({ what }: { what: string }) {
    return (
        <p
            role="status"
            className="flex items-start gap-2 rounded-lg border border-border bg-surface p-3 text-xs text-muted"
        >
            <FiInfo className="mt-0.5 size-3.5 shrink-0" aria-hidden />
            <span>
                {what} is not available on the server yet. It works on the website, and will appear
                here once the server offers it to the app.
            </span>
        </p>
    )
}

export function Initial({ name }: { name: string }) {
    return (
        <span
            aria-hidden
            className="flex size-8 shrink-0 items-center justify-center rounded-full bg-surface-2 text-xs font-semibold"
        >
            {name.slice(0, 1).toUpperCase() || '?'}
        </span>
    )
}

/** The modal frame every party dialog uses: focus trap, Escape, labelled. */
export function Modal({
    title,
    onClose,
    children,
}: {
    title: string
    onClose: () => void
    children: ReactNode
}) {
    const ref = useDialog<HTMLDivElement>(onClose)

    return (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4">
            <div
                ref={ref}
                role="dialog"
                aria-modal="true"
                aria-label={title}
                tabIndex={-1}
                className="max-h-[90vh] w-full max-w-lg overflow-y-auto rounded-xl border border-border bg-surface p-4 focus:outline-none"
            >
                <h2 className="text-sm font-semibold">{title}</h2>
                {children}
            </div>
        </div>
    )
}
