import { z } from 'zod'

/**
 * Direct-message and live-chat payloads, from the site's `~/types/chat` and
 * `~/types/live-chat`. Neither has an `/api/app/v1` route today (they are tRPC
 * with a session cookie), so these describe the routes the app clients target
 * and the shapes the procedures already return: ids are strings, dates ISO.
 * See docs/party-chat-api.md.
 */

export const ChatFolderVals = ['inbox', 'requests'] as const
export const LiveChatScopeVals = ['PARTY', 'SERVER', 'GAME'] as const

/** Mirrors `SocialUserT`; `role` and `activityStatus` stay strings. */
export const ChatUserSchema = z.object({
    id: z.string(),
    name: z.string().nullable(),
    username: z.string().nullable(),
    avatarKey: z.string().nullable(),
    role: z.string(),
    lastVisitTime: z.string().nullable(),
    activityStatus: z.string(),
    showActivity: z.boolean(),
    linkable: z.boolean(),
    anonymous: z.boolean(),
})

export const ChatAttachmentSchema = z.object({
    id: z.number().int(),
    fileId: z.string(),
    key: z.string(),
    title: z.string().nullable(),
    size: z.string(),
    type: z.string(),
    isPublic: z.boolean(),
})

export const ChatMessageSchema = z.object({
    id: z.string(),
    createdAt: z.string(),
    editedAt: z.string().nullable(),
    content: z.string().nullable(),
    deleted: z.boolean(),
    blocked: z.boolean(),
    mine: z.boolean(),
    sender: ChatUserSchema.nullable(),
    attachments: z.array(ChatAttachmentSchema),
    /** `created | added | removed | left | renamed` for a system line. */
    systemKey: z.string().nullable(),
    systemArgs: z.record(z.string(), z.string()).nullable(),
})
export type ChatMessageT = z.infer<typeof ChatMessageSchema>

export const ChatThreadSchema = z.object({
    id: z.string(),
    isGroup: z.boolean(),
    title: z.string().nullable(),
    lastMessageAt: z.string(),
    others: z.array(ChatUserSchema),
    preview: z.string().nullable(),
    unread: z.number().int(),
    muted: z.boolean(),
    closed: z.boolean(),
    pending: z.boolean(),
    iconKey: z.string().nullable(),
    memberCount: z.number().int(),
    ownerId: z.string().nullable(),
    isOwner: z.boolean(),
    canAddMore: z.number().int(),
})
export type ChatThreadT = z.infer<typeof ChatThreadSchema>

export const ChatThreadsResponse = z.object({
    items: z.array(ChatThreadSchema),
    nextCursor: z.number().int().nullable(),
})
export type ChatThreadsResponseT = z.infer<typeof ChatThreadsResponse>

export const ChatMessagesResponse = z.object({
    items: z.array(ChatMessageSchema),
    nextCursor: z.string().nullable(),
})
export type ChatMessagesResponseT = z.infer<typeof ChatMessagesResponse>

export const ChatSentResponse = z.object({
    chatId: z.string(),
    messageId: z.string(),
    createdAt: z.string(),
})
export type ChatSentResponseT = z.infer<typeof ChatSentResponse>

/** The badge: unread messages across every thread. */
export const ChatUnreadResponse = z.number().int().nonnegative()

export const ChatThreadsQuery = z.object({
    limit: z.number().int().min(1).max(50).optional(),
    cursor: z.number().int().min(0).nullish(),
    openOnly: z.boolean().optional(),
    search: z.string().nullish(),
    folder: z.enum(ChatFolderVals).optional(),
})
export type ChatThreadsQueryT = z.infer<typeof ChatThreadsQuery>

/** Send into `chatId`, or start with `toUserId`. */
export const ChatSendRequest = z.object({
    chatId: z.string().min(1).nullish(),
    toUserId: z.string().min(1).nullish(),
    content: z.string().optional(),
    fileIds: z.array(z.string().min(1)).optional(),
})
export type ChatSendRequestT = z.infer<typeof ChatSendRequest>

// ---------------------------------------------------------------- Live chat

export const LiveChatTargetSchema = z.object({
    scope: z.enum(LiveChatScopeVals),
    partyId: z.string().optional(),
    serverId: z.number().int().positive().optional(),
})
export type LiveChatTargetT = z.infer<typeof LiveChatTargetSchema>

export const LiveChatMessageSchema = z.object({
    id: z.string(),
    kind: z.string(),
    createdAt: z.string(),
    editedAt: z.string().nullable(),
    body: z.string(),
    muted: z.boolean(),
    hidden: z.boolean(),
    deleted: z.boolean(),
    user: z
        .object({
            id: z.string(),
            username: z.string().nullable(),
            name: z.string().nullable(),
            role: z.string(),
            avatarKey: z.string().nullable(),
            linkable: z.boolean(),
        })
        .nullable(),
    authorName: z.string().nullable(),
    reply: z
        .object({
            id: z.string(),
            authorName: z.string().nullable(),
            excerpt: z.string(),
        })
        .nullable(),
    reactions: z.array(
        z.object({ emoji: z.string(), count: z.number().int(), mine: z.boolean() })
    ),
    canEdit: z.boolean(),
    canDelete: z.boolean(),
})
export type LiveChatMessageT = z.infer<typeof LiveChatMessageSchema>

/** `liveChat.list`: ascending page plus its edges. `after` fetches only newer. */
export const LiveChatListResponse = z.object({
    channelId: z.string().nullable(),
    messages: z.array(LiveChatMessageSchema),
    oldest: z.string().nullable(),
    newest: z.string().nullable(),
    hasMore: z.boolean(),
})
export type LiveChatListResponseT = z.infer<typeof LiveChatListResponse>

export const LiveChatChannelSchema = z.object({
    id: z.string(),
    scope: z.string(),
    locked: z.boolean(),
    cntMessages: z.number().int(),
    canPost: z.boolean(),
    wouldBeMuted: z.boolean(),
    muted: z.boolean(),
    /** The site's i18n key for why posting is refused. */
    denyKey: z.string().nullable(),
})
export type LiveChatChannelT = z.infer<typeof LiveChatChannelSchema>

export const LiveChatHistoryQuery = z.object({
    limit: z.number().int().min(1).max(100).optional(),
    before: z.string().nullish(),
    after: z.string().nullish(),
})
export type LiveChatHistoryQueryT = z.infer<typeof LiveChatHistoryQuery>

export const LiveChatPostRequest = z.object({
    target: LiveChatTargetSchema,
    body: z.string().trim().min(1),
    replyToId: z.string().nullish(),
})
