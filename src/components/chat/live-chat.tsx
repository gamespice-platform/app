import { useEffect, useRef, useState } from 'react'
import { FiEdit2, FiSend, FiSmile, FiTrash2, FiCornerUpLeft, FiX } from 'react-icons/fi'

import Markdown from '~/components/markdown'
import { LoadError } from '~/components/states'
import { isUnavailable } from '~/lib/api/unavailable'
import { liveChatApi } from '~/lib/api/live-chat'
import type { LiveChatMessageT, LiveChatTargetT } from '~/lib/api/chat-schemas'
import { useAuth } from '~/lib/auth/provider'
import {
    QUICK_REACTIONS,
    clockTime,
    lineState,
    postBlockReason,
    rateLimitWait,
    sendable,
    sendsOnKey,
} from '~/lib/chat/chat-logic'
import { useLiveChat } from '~/lib/chat/use-live-chat'
import { messageOf } from '~/lib/ipc'
import { ChatUnavailable } from './unavailable'

/**
 * A live-chat room: a party's, a server's, or the in-game relay's. Mount it
 * with the target: `<LiveChat target={{ scope: 'PARTY', partyId }} />`.
 *
 * History and polling come from `useLiveChat`; this draws them. New lines are
 * announced through a polite live region, Enter sends (Shift+Enter is a
 * newline), and every refusal the site gives (locked, muted, rate limited) is
 * said next to the composer.
 */
export function LiveChat({
    target,
    label = 'Chat',
    className = '',
}: {
    target: LiveChatTargetT | null
    /** The accessible name of the log. */
    label?: string
    className?: string
}) {
    const { status } = useAuth()
    const room = useLiveChat(status === 'signedIn' ? target : null)
    const [replyTo, setReplyTo] = useState<LiveChatMessageT | null>(null)
    const endRef = useRef<HTMLDivElement>(null)
    const stick = useRef(true)
    const logRef = useRef<HTMLDivElement>(null)
    const count = room.messages.length

    // Follow the newest line unless the reader scrolled up to history.
    useEffect(() => {
        if (stick.current) endRef.current?.scrollIntoView?.({ block: 'end' })
    }, [count])

    if (status !== 'signedIn')
        return <p className={`p-4 text-xs text-muted ${className}`}>Sign in to read and post in chat.</p>

    if (room.unavailable) return <ChatUnavailable compact />

    const blocked = postBlockReason(room.channel)

    return (
        <div className={`flex min-h-0 flex-col ${className}`}>
            <div
                ref={logRef}
                role="log"
                aria-live="polite"
                aria-relevant="additions"
                aria-label={label}
                tabIndex={0}
                onScroll={(e) => {
                    const el = e.currentTarget

                    stick.current = el.scrollHeight - el.scrollTop - el.clientHeight < 48
                }}
                className="min-h-0 flex-1 overflow-auto p-2 focus-visible:outline focus-visible:outline-2 focus-visible:outline-accent"
            >
                {room.hasMore && (
                    <button
                        type="button"
                        onClick={() => void room.loadOlder().catch(() => undefined)}
                        className="mx-auto mb-2 block rounded-md border border-border px-3 py-1 text-[11px] text-muted hover:text-foreground"
                    >
                        Load earlier messages
                    </button>
                )}

                {room.loading && count === 0 && <p className="p-2 text-xs text-muted">Loading chat…</p>}
                {!room.loading && count === 0 && !room.error && (
                    <p className="p-2 text-xs text-muted">No messages yet. Say hello.</p>
                )}

                <ul className="flex flex-col gap-1.5">
                    {room.messages.map((m) => (
                        <Line key={m.id} message={m} onReply={setReplyTo} onChanged={room.refresh} />
                    ))}
                </ul>
                <div ref={endRef} />
            </div>

            {room.error && (
                <div className="px-2 pb-1">
                    <LoadError title="Could not reach chat" detail={messageOf(room.error)} onRetry={() => void room.refresh()} />
                </div>
            )}

            <Composer
                blocked={blocked}
                replyTo={replyTo}
                onCancelReply={() => setReplyTo(null)}
                onPost={async (body) => {
                    await room.post(body, replyTo?.id)
                    setReplyTo(null)
                    stick.current = true
                }}
            />
        </div>
    )
}

function Line({
    message: m,
    onReply,
    onChanged,
}: {
    message: LiveChatMessageT
    onReply: (m: LiveChatMessageT) => void
    onChanged: () => Promise<void>
}) {
    const [editing, setEditing] = useState(false)
    const [draft, setDraft] = useState(m.body)
    const [picking, setPicking] = useState(false)
    const [error, setError] = useState<string | null>(null)
    const state = lineState(m)
    const who = m.user?.name ?? m.user?.username ?? m.authorName ?? 'Someone'

    const run = async (fn: () => Promise<unknown>) => {
        setError(null)

        try {
            await fn()
            await onChanged()
        } catch (e) {
            setError(messageOf(e))
        }
    }

    if (state.kind === 'deleted')
        return <li className="px-1 text-[11px] italic text-muted">A message was removed.</li>

    if (m.kind !== 'MESSAGE' && m.kind !== 'message' && m.kind !== '')
        return <li className="px-1 text-center text-[11px] text-muted">{m.body}</li>

    return (
        <li className="group rounded-md px-1 py-0.5 hover:bg-surface-hover">
            <div className="flex items-baseline gap-2">
                <span className="truncate text-xs font-semibold">{who}</span>
                <time dateTime={m.createdAt} className="text-[10px] text-muted">
                    {clockTime(m.createdAt)}
                </time>
                {m.editedAt && <span className="text-[10px] text-muted">(edited)</span>}
            </div>

            {m.reply && (
                <p className="truncate border-l-2 border-border pl-2 text-[11px] text-muted">
                    {m.reply.authorName ?? 'Someone'}: {m.reply.excerpt}
                </p>
            )}

            {state.kind === 'hidden' ? (
                <p className="text-xs italic text-muted">Hidden by a moderator.</p>
            ) : editing ? (
                <form
                    onSubmit={(e) => {
                        e.preventDefault()

                        const body = sendable(draft)

                        if (body) void run(() => liveChatApi.edit(m.id, body)).then(() => setEditing(false))
                    }}
                    className="flex gap-1"
                >
                    <input
                        autoFocus
                        aria-label="Edit message"
                        value={draft}
                        onChange={(e) => setDraft(e.target.value)}
                        onKeyDown={(e) => e.key === 'Escape' && setEditing(false)}
                        className="min-w-0 flex-1 rounded-md border border-border bg-background px-2 py-1 text-xs"
                    />
                    <button type="submit" className="rounded-md bg-accent px-2 text-[11px] text-accent-foreground">
                        Save
                    </button>
                </form>
            ) : (
                <div className={state.kind === 'muted' ? 'opacity-60' : ''}>
                    <Markdown source={m.body} />
                    {state.kind === 'muted' && (
                        <p className="text-[10px] text-muted">Only you can see this: you are muted.</p>
                    )}
                </div>
            )}

            <div className="mt-0.5 flex flex-wrap items-center gap-1">
                {m.reactions.map((r) => (
                    <button
                        key={r.emoji}
                        type="button"
                        aria-pressed={r.mine}
                        aria-label={`${r.emoji} ${r.count}${r.mine ? ', you reacted' : ''}`}
                        onClick={() => void run(() => liveChatApi.react(m.id, r.emoji, !r.mine))}
                        className={`rounded-full border px-1.5 text-[11px] ${r.mine ? 'border-accent text-accent' : 'border-border'}`}
                    >
                        {r.emoji} {r.count}
                    </button>
                ))}

                <span className="flex items-center gap-0.5 opacity-0 transition-opacity focus-within:opacity-100 group-hover:opacity-100">
                    <IconBtn label="React" onClick={() => setPicking((p) => !p)}>
                        <FiSmile className="size-3" />
                    </IconBtn>
                    <IconBtn label="Reply" onClick={() => onReply(m)}>
                        <FiCornerUpLeft className="size-3" />
                    </IconBtn>
                    {m.canEdit && !editing && (
                        <IconBtn label="Edit" onClick={() => { setDraft(m.body); setEditing(true) }}>
                            <FiEdit2 className="size-3" />
                        </IconBtn>
                    )}
                    {m.canDelete && (
                        <IconBtn label="Delete" onClick={() => void run(() => liveChatApi.remove(m.id))}>
                            <FiTrash2 className="size-3" />
                        </IconBtn>
                    )}
                </span>

                {picking &&
                    QUICK_REACTIONS.map((emoji) => (
                        <button
                            key={emoji}
                            type="button"
                            aria-label={`React with ${emoji}`}
                            onClick={() => {
                                setPicking(false)
                                void run(() => liveChatApi.react(m.id, emoji, true))
                            }}
                            className="rounded px-1 text-sm hover:bg-surface-2"
                        >
                            {emoji}
                        </button>
                    ))}
            </div>

            {error && (
                <p role="alert" className="text-[11px] text-danger">
                    {isUnavailable(error) ? 'That action is not available yet.' : error}
                </p>
            )}
        </li>
    )
}

function IconBtn({
    label,
    onClick,
    children,
}: {
    label: string
    onClick: () => void
    children: React.ReactNode
}) {
    return (
        <button
            type="button"
            title={label}
            aria-label={label}
            onClick={onClick}
            className="rounded p-1 text-muted hover:text-foreground focus-visible:outline focus-visible:outline-2 focus-visible:outline-accent"
        >
            {children}
        </button>
    )
}

function Composer({
    blocked,
    replyTo,
    onCancelReply,
    onPost,
}: {
    blocked: string | null
    replyTo: LiveChatMessageT | null
    onCancelReply: () => void
    onPost: (body: string) => Promise<void>
}) {
    const [text, setText] = useState('')
    const [busy, setBusy] = useState(false)
    const [error, setError] = useState<string | null>(null)
    const [until, setUntil] = useState(0)
    const [now, setNow] = useState(() => Date.now())

    // Tick only while a rate-limit wait is showing.
    useEffect(() => {
        if (until <= Date.now()) return

        const id = setInterval(() => setNow(Date.now()), 500)

        return () => clearInterval(id)
    }, [until])

    const waitLeft = Math.max(0, Math.ceil((until - now) / 1000))

    const submit = async () => {
        const body = sendable(text)

        if (!body || busy || blocked || waitLeft > 0) return

        setBusy(true)
        setError(null)

        try {
            await onPost(body)
            setText('')
        } catch (e) {
            const msg = messageOf(e)
            const wait = rateLimitWait(msg)

            if (wait !== null) {
                setUntil(Date.now() + wait * 1000)
                setNow(Date.now())
            }

            setError(isUnavailable(e) ? 'Posting is not available yet.' : msg)
        } finally {
            setBusy(false)
        }
    }

    return (
        <div className="border-t border-border p-2">
            {replyTo && (
                <p className="mb-1 flex items-center gap-1 text-[11px] text-muted">
                    <span className="min-w-0 flex-1 truncate">
                        Replying to {replyTo.user?.name ?? replyTo.authorName ?? 'a message'}
                    </span>
                    <IconBtn label="Cancel reply" onClick={onCancelReply}>
                        <FiX className="size-3" />
                    </IconBtn>
                </p>
            )}

            {blocked ? (
                <p role="status" className="text-xs text-muted">{blocked}</p>
            ) : (
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
                        placeholder="Write a message (Markdown works)"
                        className="max-h-32 min-h-8 min-w-0 flex-1 resize-y rounded-md border border-border bg-background px-2 py-1.5 text-xs"
                    />
                    <button
                        type="submit"
                        disabled={busy || waitLeft > 0 || !sendable(text)}
                        aria-label="Send"
                        className="inline-flex size-8 items-center justify-center rounded-md bg-accent text-accent-foreground disabled:opacity-50"
                    >
                        <FiSend className="size-3.5" />
                    </button>
                </form>
            )}

            {waitLeft > 0 ? (
                <p role="alert" className="mt-1 text-[11px] text-warning">
                    You are sending too fast. Try again in {waitLeft}s.
                </p>
            ) : (
                error && (
                    <p role="alert" className="mt-1 text-[11px] text-danger">
                        {error}
                    </p>
                )
            )}
        </div>
    )
}
