import { describe, expect, it } from 'vitest'

import type { ChatMessageT, ChatThreadT } from '~/lib/api/chat-schemas'
import {
    chatRows,
    lineState,
    postBlockReason,
    rateLimitWait,
    sendable,
    sendsOnKey,
    sortThreads,
    systemText,
    threadTitle,
    unreadLabel,
} from './chat-logic'

const msg = (id: string, at: string, over: Partial<ChatMessageT> = {}): ChatMessageT => ({
    id,
    createdAt: at,
    editedAt: null,
    content: 'hi',
    deleted: false,
    blocked: false,
    mine: false,
    sender: { id: 'u1' } as ChatMessageT['sender'],
    attachments: [],
    systemKey: null,
    systemArgs: null,
    ...over,
})

describe('composer', () => {
    it('sends on plain Enter only', () => {
        expect(sendsOnKey({ key: 'Enter', shiftKey: false })).toBe(true)
        expect(sendsOnKey({ key: 'Enter', shiftKey: true })).toBe(false)
        expect(sendsOnKey({ key: 'Enter', shiftKey: false, isComposing: true })).toBe(false)
        expect(sendsOnKey({ key: 'a', shiftKey: false })).toBe(false)
    })

    it('does not send blank text and trims the rest', () => {
        expect(sendable('  \n ')).toBeNull()
        expect(sendable(' hi ')).toBe('hi')
    })
})

describe('unreadLabel', () => {
    it('is empty at zero and exact above it', () => {
        expect(unreadLabel(0)).toBe('')
        expect(unreadLabel(1234)).toBe('1234')
    })
})

describe('threads', () => {
    const t = (over: Partial<ChatThreadT>) =>
        ({ title: null, others: [], isGroup: false, lastMessageAt: '2026-01-01', ...over }) as ChatThreadT

    it('titles by title, then people, then a fallback', () => {
        expect(threadTitle(t({ title: ' Squad ' }))).toBe('Squad')
        expect(
            threadTitle(t({ others: [{ name: 'Bo', username: 'bo' }, { name: null, username: 'cy' }] as ChatThreadT['others'] }))
        ).toBe('Bo, cy')
        expect(threadTitle(t({ isGroup: true }))).toBe('Group chat')
    })

    it('sorts newest first without mutating', () => {
        const a = t({ lastMessageAt: '2026-01-01' })
        const b = t({ lastMessageAt: '2026-02-01' })
        const input = [a, b]

        expect(sortThreads(input)).toEqual([b, a])
        expect(input[0]).toBe(a)
    })
})

describe('chatRows', () => {
    it('adds day dividers and groups a sender run', () => {
        const rows = chatRows([
            msg('1', '2026-11-22T10:00:00Z'),
            msg('2', '2026-11-22T10:01:00Z'),
            msg('3', '2026-11-22T10:30:00Z'),
            msg('4', '2026-11-23T09:00:00Z'),
        ])

        expect(rows.map((r) => r.kind)).toEqual(['day', 'message', 'message', 'message', 'day', 'message'])
        expect(rows.filter((r) => r.kind === 'message').map((r) => (r.kind === 'message' ? r.head : null))).toEqual([
            true,
            false,
            true,
            true,
        ])
    })

    it('breaks a run on a different sender and around system lines', () => {
        const rows = chatRows([
            msg('1', '2026-11-22T10:00:00Z'),
            msg('2', '2026-11-22T10:00:30Z', { systemKey: 'left', systemArgs: { name: 'Bo' } }),
            msg('3', '2026-11-22T10:00:40Z'),
            msg('4', '2026-11-22T10:00:50Z', { mine: true }),
        ])

        expect(rows.map((r) => r.kind)).toEqual(['day', 'message', 'system', 'message', 'message'])
        expect(rows.filter((r) => r.kind === 'message').every((r) => r.kind === 'message' && r.head)).toBe(true)
    })
})

describe('systemText', () => {
    it('words each system key', () => {
        expect(systemText({ systemKey: 'left', systemArgs: { name: 'Bo' } })).toBe('Bo left')
        expect(systemText({ systemKey: 'zzz', systemArgs: null })).toBe('Conversation updated')
    })
})

describe('lineState', () => {
    it('ranks deleted over hidden over muted', () => {
        expect(lineState({ deleted: true, hidden: true, muted: true }).kind).toBe('deleted')
        expect(lineState({ deleted: false, hidden: true, muted: true }).kind).toBe('hidden')
        expect(lineState({ deleted: false, hidden: false, muted: true }).kind).toBe('muted')
        expect(lineState({ deleted: false, hidden: false, muted: false }).kind).toBe('normal')
    })
})

describe('postBlockReason', () => {
    const chan = (over: object) =>
        ({ locked: false, muted: false, canPost: true, denyKey: null, ...over }) as Parameters<typeof postBlockReason>[0]

    it('is null when posting is allowed or unknown', () => {
        expect(postBlockReason(null)).toBeNull()
        expect(postBlockReason(chan({}))).toBeNull()
    })

    it('explains each refusal', () => {
        expect(postBlockReason(chan({ locked: true }))).toMatch(/locked/)
        expect(postBlockReason(chan({ muted: true }))).toMatch(/muted/)
        expect(postBlockReason(chan({ canPost: false, denyKey: 'chat.needVerified' }))).toMatch(/Verify/)
        expect(postBlockReason(chan({ canPost: false }))).toMatch(/cannot post/)
    })
})

describe('rateLimitWait', () => {
    it('reads the wait and units', () => {
        expect(rateLimitWait('Too many requests. Try again in 12 seconds.')).toBe(12)
        expect(rateLimitWait('Rate limited, retry in 2 min')).toBe(120)
        expect(rateLimitWait('slow down (1500 ms)')).toBe(2)
    })

    it('is 0 when limited with no wait, null when not a limit', () => {
        expect(rateLimitWait('Too many messages')).toBe(0)
        expect(rateLimitWait('Not found')).toBeNull()
    })
})
