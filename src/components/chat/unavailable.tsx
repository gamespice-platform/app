import { FiMessageSquare } from 'react-icons/fi'

/**
 * Chat is served by routes the site is still adding for the app. Until they
 * answer, every chat surface says so in one voice rather than failing.
 */
export function ChatUnavailable({ compact = false }: { compact?: boolean }) {
    return (
        <div
            role="status"
            className={`flex flex-col items-center gap-2 text-center ${compact ? 'p-4' : 'p-8'}`}
        >
            <FiMessageSquare className="size-6 text-muted" aria-hidden />
            <p className="text-sm font-medium">Chat is not available on this server yet</p>
            <p className="max-w-xs text-xs text-muted">
                The site has not enabled chat for the app. Nothing is wrong on your side; it will
                appear here once it does.
            </p>
        </div>
    )
}
