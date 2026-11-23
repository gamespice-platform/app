import { useEffect, useRef, useState } from 'react'
import { Link, useNavigate, useParams, useSearchParams } from 'react-router-dom'
import { FiArrowLeft, FiSlash } from 'react-icons/fi'

import { ChatUnavailable } from '~/components/chat/unavailable'
import { DmComposer } from '~/components/chat/dm-composer'
import Markdown from '~/components/markdown'
import { LoadError, SignInPrompt } from '~/components/states'
import { chatApi } from '~/lib/api/chat'
import type { ChatThreadT } from '~/lib/api/chat-schemas'
import { useAuth } from '~/lib/auth/provider'
import {
    chatRows,
    clockTime,
    sortThreads,
    systemText,
    threadTitle,
    unreadLabel,
} from '~/lib/chat/chat-logic'
import { useChatConversation, useChatRequestCount, useChatThread, useChatThreads } from '~/lib/chat/use-chat'
import { messageOf } from '~/lib/ipc'

/**
 * **Messages** — direct messages, the same threads the website shows.
 *
 * Inbox and message requests on the left, the open conversation on the right
 * (one at a time on a narrow window). `/messages?to=<userId>` opens a draft to a
 * member with no thread yet; the first send creates it.
 */
export default function MessagesRoute() {
    const { status } = useAuth()

    if (status !== 'signedIn')
        return (
            <div className="flex flex-col gap-3 p-3">
                <h1 className="text-lg font-bold">Messages</h1>
                <SignInPrompt>Sign in to read and send messages.</SignInPrompt>
            </div>
        )

    return <SignedIn />
}

type FolderT = 'inbox' | 'requests'

function SignedIn() {
    const { chatId } = useParams<{ chatId?: string }>()
    const [search] = useSearchParams()
    const to = search.get('to')
    const [folder, setFolder] = useState<FolderT>('inbox')
    const threads = useChatThreads({ folder })
    // The tab badge is the count endpoint; the requests list itself is only polled
    // while that folder is showing (`threads`), never twice.
    const requests = useChatRequestCount()
    const open = chatId != null || to != null
    const requestCount = requests.data ?? 0
    const listed = chatId ? (threads.data?.items.find((t) => t.id === chatId) ?? null) : null
    // A thread outside the folder on screen (a deep link, a request while on Inbox) is fetched alone.
    const single = useChatThread(chatId ?? null, chatId != null && listed == null)
    const thread = listed ?? single.data ?? null
    const refreshLists = () => {
        void threads.refresh()
        void requests.refresh()
        void single.refresh()
    }

    if (threads.unavailable) return <ChatUnavailable />

    return (
        <div className="flex h-full min-h-0 flex-col gap-2 p-3">
            <h1 className="text-lg font-bold">Messages</h1>

            <div className="flex min-h-0 flex-1 gap-3">
                <aside
                    aria-label="Conversations"
                    className={`flex min-h-0 w-full flex-col overflow-hidden rounded-xl border border-border bg-surface min-[860px]:w-80 min-[860px]:shrink-0 ${open ? 'hidden min-[860px]:flex' : ''}`}
                >
                    <div role="tablist" className="flex border-b border-border">
                        {(['inbox', 'requests'] as const).map((f) => (
                            <button
                                key={f}
                                type="button"
                                role="tab"
                                aria-selected={folder === f}
                                onClick={() => setFolder(f)}
                                className={`flex-1 px-3 py-2 text-xs font-medium focus-visible:outline focus-visible:outline-2 focus-visible:outline-accent ${folder === f ? 'border-b-2 border-accent text-accent' : 'text-muted hover:text-foreground'}`}
                            >
                                {f === 'inbox' ? 'Inbox' : `Requests${requestCount ? ` (${requestCount})` : ''}`}
                            </button>
                        ))}
                    </div>
                    <ThreadList key={folder} folder={folder} state={threads} activeId={chatId ?? null} />
                </aside>

                <section
                    aria-label="Conversation"
                    className={`min-h-0 min-w-0 flex-1 overflow-hidden rounded-xl border border-border bg-surface ${open ? 'flex' : 'hidden min-[860px]:flex'} flex-col`}
                >
                    {chatId ? (
                        <Conversation key={chatId} chatId={chatId} thread={thread} onActed={refreshLists} />
                    ) : to ? (
                        <Draft key={to} toUserId={to} />
                    ) : (
                        <p className="m-auto p-6 text-sm text-muted">Pick a conversation.</p>
                    )}
                </section>
            </div>
        </div>
    )
}

function ThreadList({
    folder,
    state,
    activeId,
}: {
    folder: FolderT
    state: ReturnType<typeof useChatThreads>
    activeId: string | null
}) {
    const [extra, setExtra] = useState<ChatThreadT[]>([])
    const [cursor, setCursor] = useState<number | null | undefined>(undefined)
    const [moreError, setMoreError] = useState<string | null>(null)
    const [pages, setPages] = useState(0)

    // Pages added with "Load more" are not part of the polled first page, so re-read
    // them (as many as were loaded) whenever the first page refreshes; otherwise a
    // read, a decline or a new reply would stay stale below the fold.
    const firstCursor = state.data?.nextCursor ?? null

    useEffect(() => {
        if (pages === 0 || firstCursor == null) return

        let dead = false
        const reload = async () => {
            const all: ChatThreadT[] = []
            let cur: number | null = firstCursor

            for (let i = 0; i < pages && cur != null; i++) {
                const p = await chatApi.threads({ folder, cursor: cur })

                all.push(...p.items)
                cur = p.nextCursor
            }

            if (!dead) {
                setExtra(all)
                setCursor(cur)
            }
        }

        void reload().catch(() => undefined)

        return () => {
            dead = true
        }
        // `state.data` changes identity on every poll, which is the trigger.
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [state.data, folder, pages])

    if (state.loading) return <p className="p-4 text-xs text-muted">Loading…</p>
    if (state.error)
        return (
            <div className="p-2">
                <LoadError title="Could not load messages" detail={messageOf(state.error)} onRetry={() => void state.refresh()} />
            </div>
        )

    const first = state.data?.items ?? []
    const heldIds = new Set(first.map((t) => t.id))
    const items = sortThreads([...first, ...extra.filter((t) => !heldIds.has(t.id))])
    const next = cursor === undefined ? (state.data?.nextCursor ?? null) : cursor

    if (items.length === 0)
        return (
            <p className="p-4 text-xs text-muted">
                {folder === 'inbox' ? 'No conversations yet. Message a friend to start one.' : 'No message requests.'}
            </p>
        )

    return (
        <ul className="min-h-0 flex-1 overflow-auto">
            {items.map((t) => (
                <li key={t.id}>
                    <Link
                        to={`/messages/${encodeURIComponent(t.id)}`}
                        aria-current={t.id === activeId ? 'page' : undefined}
                        className={`flex items-center gap-2 border-b border-border px-3 py-2.5 hover:bg-surface-hover focus-visible:outline focus-visible:outline-2 focus-visible:outline-accent ${t.id === activeId ? 'bg-surface-hover' : ''}`}
                    >
                        <span className="min-w-0 flex-1">
                            <span className={`block truncate text-sm ${t.unread > 0 ? 'font-semibold' : ''}`}>{threadTitle(t)}</span>
                            <span className="block truncate text-[11px] text-muted">{t.preview ?? 'No messages'}</span>
                        </span>
                        {t.unread > 0 && (
                            <span aria-label={`${t.unread} unread`} className="rounded-full bg-accent px-1.5 py-0.5 text-[0.6rem] font-medium text-accent-foreground">
                                {unreadLabel(t.unread)}
                            </span>
                        )}
                    </Link>
                </li>
            ))}
            {next != null && (
                <li className="p-2 text-center">
                    <button
                        type="button"
                        onClick={() =>
                            void chatApi
                                .threads({ folder, cursor: next })
                                .then((p) => {
                                    setExtra((cur) => [...cur, ...p.items])
                                    setCursor(p.nextCursor)
                                    setPages((n) => n + 1)
                                    setMoreError(null)
                                })
                                .catch((e: unknown) => setMoreError(messageOf(e)))
                        }
                        className="rounded-md border border-border px-3 py-1 text-[11px] text-muted hover:text-foreground"
                    >
                        Load more
                    </button>
                    {moreError && <p role="alert" className="mt-1 text-[11px] text-danger">{moreError}</p>}
                </li>
            )}
        </ul>
    )
}

function Header({ title, children }: { title: string; children?: React.ReactNode }) {
    return (
        <header className="flex items-center gap-2 border-b border-border px-3 py-2">
            <Link to="/messages" aria-label="Back to conversations" className="rounded p-1 text-muted hover:text-foreground min-[860px]:hidden">
                <FiArrowLeft className="size-4" />
            </Link>
            <h2 className="min-w-0 flex-1 truncate text-sm font-semibold">{title}</h2>
            {children}
        </header>
    )
}

function Conversation({
    chatId,
    thread,
    onActed,
}: {
    chatId: string
    thread: ChatThreadT | null
    onActed: () => void
}) {
    const convo = useChatConversation(chatId)
    const [acted, setActed] = useState<string | null>(null)
    const [confirmBlock, setConfirmBlock] = useState(false)
    const [blocked, setBlocked] = useState(false)
    const [error, setError] = useState<string | null>(null)
    const endRef = useRef<HTMLDivElement>(null)
    const count = convo.messages.length
    const rows = chatRows(convo.messages, (iso) => new Date(iso).toLocaleDateString())
    const other = thread && !thread.isGroup ? thread.others[0] : undefined
    const otherName = other?.name ?? other?.username ?? 'this member'

    useEffect(() => {
        endRef.current?.scrollIntoView?.({ block: 'end' })
    }, [count])

    if (convo.unavailable) return <ChatUnavailable />

    const act = async (name: string, fn: () => Promise<unknown>) => {
        setError(null)

        try {
            await fn()
            setActed(name)
            // The thread leaves (or joins) the Requests folder and the badge changes.
            onActed()

            return true
        } catch (e) {
            setError(messageOf(e))

            return false
        }
    }

    return (
        <>
            <Header title={thread ? threadTitle(thread) : 'Conversation'}>
                {other &&
                    (blocked ? (
                        <button
                            type="button"
                            onClick={() =>
                                void act('unblock', () => chatApi.block(other.id, false)).then((ok) => ok && setBlocked(false))
                            }
                            className="inline-flex items-center gap-1 rounded-md border border-border px-2 py-1 text-[11px] text-muted hover:text-foreground"
                        >
                            <FiSlash className="size-3" aria-hidden /> Unblock
                        </button>
                    ) : confirmBlock ? (
                        <span role="group" aria-label={`Block ${otherName}`} className="flex items-center gap-1 text-[11px]">
                            <button
                                type="button"
                                onClick={() => {
                                    setConfirmBlock(false)
                                    void act('block', () => chatApi.block(other.id)).then((ok) => ok && setBlocked(true))
                                }}
                                className="rounded-md bg-danger px-2 py-1 text-white"
                            >
                                Block {otherName}
                            </button>
                            <button type="button" onClick={() => setConfirmBlock(false)} className="rounded-md border border-border px-2 py-1">
                                Cancel
                            </button>
                        </span>
                    ) : (
                        <button
                            type="button"
                            onClick={() => setConfirmBlock(true)}
                            className="inline-flex items-center gap-1 rounded-md border border-border px-2 py-1 text-[11px] text-muted hover:text-danger"
                        >
                            <FiSlash className="size-3" aria-hidden /> Block
                        </button>
                    ))}
            </Header>

            {thread?.pending && (
                <div className="flex flex-wrap items-center gap-2 border-b border-border bg-surface-2 px-3 py-2 text-xs">
                    <span className="flex-1">{threadTitle(thread)} wants to message you.</span>
                    <button type="button" onClick={() => void act('accept', () => chatApi.respond(chatId, true))} className="rounded-md bg-accent px-3 py-1 text-accent-foreground">
                        Accept
                    </button>
                    <button type="button" onClick={() => void act('decline', () => chatApi.respond(chatId, false))} className="rounded-md border border-border px-3 py-1">
                        Decline
                    </button>
                </div>
            )}

            {(acted || error) && (
                <p role={error ? 'alert' : 'status'} className={`px-3 py-1 text-xs ${error ? 'text-danger' : 'text-muted'}`}>
                    {error ??
                        (acted === 'block'
                            ? 'Blocked. Their messages will stop arriving.'
                            : acted === 'unblock'
                              ? 'Unblocked.'
                              : acted === 'accept'
                                ? 'Request accepted.'
                                : 'Request declined.')}
                </p>
            )}

            <div role="log" aria-live="polite" aria-relevant="additions" aria-label="Messages" tabIndex={0} className="min-h-0 flex-1 overflow-auto p-3 focus-visible:outline focus-visible:outline-2 focus-visible:outline-accent">
                {convo.hasOlder && (
                    <button type="button" onClick={() => void convo.loadOlder().catch(() => undefined)} className="mx-auto mb-3 block rounded-md border border-border px-3 py-1 text-[11px] text-muted hover:text-foreground">
                        Load earlier messages
                    </button>
                )}
                {convo.loading && count === 0 && <p className="text-xs text-muted">Loading…</p>}
                {convo.error && <LoadError title="Could not load this conversation" detail={messageOf(convo.error)} onRetry={() => void convo.refresh()} />}

                <ul className="flex flex-col gap-1">
                    {rows.map((r) => {
                        if (r.kind === 'day')
                            return (
                                <li key={r.key} className="my-2 text-center text-[10px] uppercase tracking-wide text-muted">
                                    {r.label}
                                </li>
                            )

                        if (r.kind === 'system')
                            return (
                                <li key={r.key} className="text-center text-[11px] text-muted">
                                    {systemText(r.message)}
                                </li>
                            )

                        const m = r.message

                        return (
                            <li key={r.key} className={`flex flex-col ${m.mine ? 'items-end' : 'items-start'}`}>
                                {r.head && (
                                    <span className="mt-1 text-[10px] text-muted">
                                        {m.mine ? 'You' : (m.sender?.name ?? m.sender?.username ?? 'Someone')} · {clockTime(m.createdAt)}
                                    </span>
                                )}
                                <div className={`max-w-[85%] rounded-lg px-3 py-1.5 ${m.mine ? 'bg-accent text-accent-foreground' : 'bg-surface-2'}`}>
                                    {m.deleted ? (
                                        <span className="text-xs italic opacity-70">Message removed</span>
                                    ) : m.blocked ? (
                                        <span className="text-xs italic opacity-70">Message from a blocked member</span>
                                    ) : (
                                        <Markdown source={m.content ?? ''} />
                                    )}
                                    {m.editedAt && !m.deleted && <span className="ml-1 text-[10px] opacity-70">(edited)</span>}
                                </div>
                            </li>
                        )
                    })}
                </ul>
                <div ref={endRef} />
            </div>

            <DmComposer
                onSend={(content) => convo.send(content)}
                disabledReason={
                    thread?.closed ? 'This conversation is closed.' : thread?.pending ? 'Accept the request to reply.' : null
                }
            />
        </>
    )
}

/** A conversation not yet created: the first send makes the thread and moves to it. */
function Draft({ toUserId }: { toUserId: string }) {
    const navigate = useNavigate()

    return (
        <>
            <Header title="New message" />
            <p className="m-auto p-6 text-sm text-muted">Write the first message below to start the conversation.</p>
            <DmComposer
                onSend={async (content) => {
                    const sent = await chatApi.send({ toUserId, content })

                    void navigate(`/messages/${encodeURIComponent(sent.chatId)}`, { replace: true })
                }}
            />
        </>
    )
}
