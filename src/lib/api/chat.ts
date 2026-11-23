import { z } from 'zod'

import {
    ChatMessagesResponse,
    ChatSentResponse,
    ChatThreadSchema,
    ChatThreadsResponse,
    ChatUnreadResponse,
    type ChatSendRequestT,
    type ChatThreadsQueryT,
} from './chat-schemas'
import { get, send } from './transport'

/**
 * Direct messages and group chats. The site serves these over tRPC
 * (`chat.*`, session cookie), and the app's device token reaches only
 * `/api/app/v1`, so these target the routes website-city would add. Until it
 * does each call fails with `isUnavailable(e) === true`. docs/party-chat-api.md.
 */

export const chatApi = {
    /** Newest activity first; `nextCursor` pages. `folder: 'requests'` is message requests. */
    threads: (query: ChatThreadsQueryT = {}) =>
        get('/chat/threads', ChatThreadsResponse, query, true),

    thread: (chatId: string) => get('/chat/thread', ChatThreadSchema, { chatId }, true),

    /** Newest first; pass the previous `nextCursor` for older ones. */
    history: (chatId: string, cursor?: string | null, limit?: number) =>
        get('/chat/messages', ChatMessagesResponse, { chatId, cursor, limit }, true),

    /** `chatId` for an existing conversation, `toUserId` to open one. */
    send: (input: ChatSendRequestT) => send('POST', '/chat/send', ChatSentResponse, input),

    /** Clears the thread's unread count. */
    read: (chatId: string) => send('POST', '/chat/read', z.null(), { chatId }),

    /** Total unread across every thread, for the badge. */
    unreadCount: () => get('/chat/unread', ChatUnreadResponse, undefined, true),

    /** Message requests waiting on an answer. */
    requestCount: () => get('/chat/requests/count', ChatUnreadResponse, undefined, true),
}
