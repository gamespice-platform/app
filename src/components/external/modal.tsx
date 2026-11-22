import type { ReactNode } from 'react'
import { FiX } from 'react-icons/fi'

import { useDialog } from '~/components/use-dialog'

/** The dialog frame every external-mod dialog shares: focus trap, Escape, labelled. */
export function Modal({
    title,
    subtitle,
    onClose,
    children,
    footer,
    wide = false,
}: {
    title: string
    subtitle?: string
    onClose: () => void
    children: ReactNode
    footer?: ReactNode
    wide?: boolean
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
                className={`flex max-h-[90vh] w-full flex-col overflow-hidden rounded-xl border border-border bg-surface ${wide ? 'max-w-3xl' : 'max-w-lg'}`}
            >
                <header className="flex items-center justify-between border-b border-border p-4">
                    <div className="min-w-0">
                        <h2 className="text-sm font-semibold">{title}</h2>
                        {subtitle && <p className="truncate text-[11px] text-muted">{subtitle}</p>}
                    </div>
                    <button
                        type="button"
                        onClick={onClose}
                        aria-label="Close"
                        className="rounded-lg p-1.5 text-muted hover:bg-surface-2"
                    >
                        <FiX className="size-4" />
                    </button>
                </header>
                <div className="flex-1 overflow-y-auto p-4">{children}</div>
                {footer && (
                    <footer className="flex items-center justify-end gap-2 border-t border-border p-3">
                        {footer}
                    </footer>
                )}
            </div>
        </div>
    )
}

export const buttonPrimary =
    'rounded-lg bg-accent px-3 py-1.5 text-sm font-medium text-accent-foreground disabled:opacity-50'
export const buttonPlain =
    'rounded-lg border border-border px-3 py-1.5 text-sm hover:bg-surface-2 disabled:opacity-50'
