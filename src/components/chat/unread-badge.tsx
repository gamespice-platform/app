import { useAuth } from '~/lib/auth/provider'
import { unreadLabel } from '~/lib/chat/chat-logic'
import { useChatUnread } from '~/lib/chat/use-chat'

/** The count of unread messages beside a nav item; nothing at zero, signed out or unavailable. */
export function MessagesBadge() {
    const { status } = useAuth()
    const unread = useChatUnread(status === 'signedIn')
    const label = unreadLabel(unread.data ?? 0)

    if (!label) return null

    return (
        <span
            aria-label={`${label} unread messages`}
            className="ml-auto rounded-full bg-accent px-1.5 py-0.5 text-[0.6rem] font-medium text-accent-foreground"
        >
            {label}
        </span>
    )
}
