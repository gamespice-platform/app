import { z } from 'zod'

import {
    LiveChatChannelSchema,
    LiveChatListResponse,
    LiveChatMessageSchema,
    type LiveChatHistoryQueryT,
    type LiveChatTargetT,
} from './chat-schemas'
import { get, send } from './transport'

/**
 * Live chat rooms: a party's, a server's, or a game's. tRPC-only on the site
 * (`liveChat.*`), so these target proposed REST routes and fail with
 * `isUnavailable(e) === true` until they exist. docs/party-chat-api.md.
 *
 * "Subscribe" is not a socket: the site polls (`CHAT_POLL_MS`, 4 s) and so does
 * `~/lib/chat/live-room`, asking only for messages `after` the newest it has.
 */

/** A target as query pairs: the room is one of `partyId` / `serverId`. */
function targetQuery(target: LiveChatTargetT) {
    return {
        scope: target.scope,
        partyId: target.partyId,
        serverId: target.serverId,
    }
}

export const liveChatApi = {
    /** Whether the caller may read and post, and the site's reason when not. */
    channel: (target: LiveChatTargetT) =>
        get('/live-chat/channel', LiveChatChannelSchema.nullable(), targetQuery(target), true),

    /** Ascending. `before` pages back; `after` is the poll. */
    history: (target: LiveChatTargetT, page: LiveChatHistoryQueryT = {}) =>
        get('/live-chat/messages', LiveChatListResponse, { ...targetQuery(target), ...page }, true),

    post: (target: LiveChatTargetT, body: string, replyToId?: string | null) =>
        send('POST', '/live-chat/post', LiveChatMessageSchema, { target, body, replyToId }),

    edit: (id: string, body: string) =>
        send('POST', '/live-chat/edit', LiveChatMessageSchema, { id, body }),

    remove: (id: string) => send('POST', '/live-chat/delete', z.null(), { id }),

    react: (id: string, emoji: string, on = true) =>
        send('POST', '/live-chat/react', z.null(), { id, emoji, on }),
}
