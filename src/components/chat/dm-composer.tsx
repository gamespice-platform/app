import { useState } from 'react'
import { FiSend } from 'react-icons/fi'

import { isUnavailable } from '~/lib/api/unavailable'
import { rateLimitWait, sendable, sendsOnKey } from '~/lib/chat/chat-logic'
import { messageOf } from '~/lib/ipc'

/** The send box of a direct-message conversation. Enter sends, Shift+Enter breaks the line. */
export function DmComposer({
    onSend,
    disabledReason,
}: {
    onSend: (content: string) => Promise<void>
    disabledReason?: string | null
}) {
    const [text, setText] = useState('')
    const [busy, setBusy] = useState(false)
    const [error, setError] = useState<string | null>(null)

    const submit = async () => {
        const body = sendable(text)

        if (!body || busy) return

        setBusy(true)
        setError(null)

        try {
            await onSend(body)
            setText('')
        } catch (e) {
            const msg = messageOf(e)
            const wait = rateLimitWait(msg)

            setError(
                isUnavailable(e)
                    ? 'Sending is not available yet.'
                    : wait !== null
                      ? `You are sending too fast${wait > 0 ? `. Try again in ${wait}s` : ''}.`
                      : msg
            )
        } finally {
            setBusy(false)
        }
    }

    if (disabledReason)
        return (
            <p role="status" className="border-t border-border p-3 text-xs text-muted">
                {disabledReason}
            </p>
        )

    return (
        <div className="border-t border-border p-2">
            <form
                onSubmit={(e) => {
                    e.preventDefault()
                    void submit()
                }}
                className="flex items-end gap-1.5"
            >
                <textarea
                    aria-label="Message"
                    rows={1}
                    value={text}
                    onChange={(e) => setText(e.target.value)}
                    onKeyDown={(e) => {
                        if (sendsOnKey(e.nativeEvent)) {
                            e.preventDefault()
                            void submit()
                        }
                    }}
                    placeholder="Write a message"
                    className="max-h-40 min-h-9 min-w-0 flex-1 resize-y rounded-md border border-border bg-background px-2 py-2 text-sm"
                />
                <button
                    type="submit"
                    disabled={busy || !sendable(text)}
                    aria-label="Send"
                    className="inline-flex size-9 items-center justify-center rounded-md bg-accent text-accent-foreground disabled:opacity-50"
                >
                    <FiSend className="size-4" />
                </button>
            </form>
            {error && (
                <p role="alert" className="mt-1 text-xs text-danger">
                    {error}
                </p>
            )}
        </div>
    )
}
