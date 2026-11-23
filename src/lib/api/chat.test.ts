import { beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn()

vi.mock('@tauri-apps/api/core', () => ({
    invoke: (...a: unknown[]): unknown => invoke(...a),
}))

import { chatApi } from './chat'
import { liveChatApi } from './live-chat'

/*
 * Shapes are `ChatThreadT`, `ChatMessageT` and the `chat.send` return from
 * website-city's `~/types/chat`, with dates as ISO strings.
 */

const user = {
    id: 'clx2',
    name: 'Bo',
    username: 'bo',
    avatarKey: null,
    role: 'USER',
    lastVisitTime: null,
    activityStatus: 'ACTIVE',
    showActivity: false,
    linkable: true,
    anonymous: false,
}

const message = {
    id: '501',
    createdAt: '2026-11-22T20:00:00.000Z',
    editedAt: null,
    content: 'gg',
    deleted: false,
    blocked: false,
    mine: false,
    sender: user,
    attachments: [],
    systemKey: null,
    systemArgs: null,
}

const thread = {
    id: '77',
    isGroup: false,
    title: null,
    lastMessageAt: '2026-11-22T20:00:00.000Z',
    others: [user],
    preview: 'gg',
    unread: 2,
    muted: false,
    closed: false,
    pending: false,
    iconKey: null,
    memberCount: 2,
    ownerId: null,
    isOwner: false,
    canAddMore: 0,
}

describe('chat client', () => {
    beforeEach(() => invoke.mockReset())

    it('lists threads with a cursor and parses unread counts', async () => {
        invoke.mockResolvedValue({ items: [thread], nextCursor: 20 })

        const page = await chatApi.threads({ folder: 'requests', cursor: 20 })

        expect(page.items[0]!.unread).toBe(2)
        expect(invoke).toHaveBeenCalledWith('api_get', {
            path: '/chat/threads',
            query: [
                ['folder', 'requests'],
                ['cursor', '20'],
            ],
            auth: true,
        })
    })

    it('pages history backwards by cursor', async () => {
        invoke.mockResolvedValue({ items: [message], nextCursor: null })

        const page = await chatApi.history('77', '500', 30)

        expect(page.nextCursor).toBeNull()
        expect(invoke).toHaveBeenCalledWith('api_get', {
            path: '/chat/messages',
            query: [
                ['chatId', '77'],
                ['cursor', '500'],
                ['limit', '30'],
            ],
            auth: true,
        })
    })

    it('opens a conversation by user and marks one read', async () => {
        invoke.mockResolvedValueOnce({
            chatId: '77',
            messageId: '502',
            createdAt: '2026-11-22T20:01:00.000Z',
        })

        const sent = await chatApi.send({ toUserId: 'clx2', content: 'hi' })

        expect(sent.chatId).toBe('77')

        invoke.mockResolvedValueOnce(null)
        await chatApi.read('77')

        expect(invoke).toHaveBeenLastCalledWith('api_send', {
            method: 'POST',
            path: '/chat/read',
            body: { chatId: '77' },
            auth: true,
        })
    })

    it('rejects a negative unread count', async () => {
        vi.spyOn(console, 'error').mockImplementation(() => {})
        invoke.mockResolvedValue(-1)

        await expect(chatApi.unreadCount()).rejects.toThrow()
    })
})

describe('live chat client', () => {
    beforeEach(() => invoke.mockReset())

    const live = {
        id: '900',
        kind: 'USER',
        createdAt: '2026-11-22T20:00:00.000Z',
        editedAt: null,
        body: 'ready?',
        muted: false,
        hidden: false,
        deleted: false,
        user: { id: 'clx1', username: 'ash', name: 'Ashley', role: 'USER', avatarKey: null, linkable: true },
        authorName: 'Ashley',
        reply: null,
        reactions: [{ emoji: '👍', count: 1, mine: false }],
        canEdit: false,
        canDelete: false,
    }

    it('polls only for messages after the newest it holds', async () => {
        invoke.mockResolvedValue({
            channelId: '5',
            messages: [live],
            oldest: '900',
            newest: '900',
            hasMore: false,
        })

        const page = await liveChatApi.history({ scope: 'PARTY', partyId: '4471' }, { after: '899' })

        expect(page.messages).toHaveLength(1)
        expect(invoke).toHaveBeenCalledWith('api_get', {
            path: '/live-chat/messages',
            query: [
                ['scope', 'PARTY'],
                ['partyId', '4471'],
                ['after', '899'],
            ],
            auth: true,
        })
    })

    it('posts into a room and reads the channel permission', async () => {
        invoke.mockResolvedValueOnce(live)
        await liveChatApi.post({ scope: 'SERVER', serverId: 4821 }, 'ready?')

        expect(invoke).toHaveBeenLastCalledWith('api_send', {
            method: 'POST',
            path: '/live-chat/post',
            body: { target: { scope: 'SERVER', serverId: 4821 }, body: 'ready?', replyToId: undefined },
            auth: true,
        })

        invoke.mockResolvedValueOnce({
            id: '5',
            scope: 'PARTY',
            locked: false,
            cntMessages: 12,
            canPost: false,
            wouldBeMuted: false,
            muted: false,
            denyKey: 'chat.deny.locked',
        })

        const channel = await liveChatApi.channel({ scope: 'PARTY', partyId: '4471' })

        expect(channel?.denyKey).toBe('chat.deny.locked')
    })
})
