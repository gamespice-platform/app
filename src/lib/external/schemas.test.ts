import { describe, expect, it } from 'vitest'

import {
    AddOutcomeSchema,
    CredentialReportSchema,
    InspectionSchema,
    ParsedExternalSchema,
    PendingSchema,
    SchemeStatusSchema,
} from './schemas'

describe('external schemas match the Rust serde output', () => {
    it('parses a keyed provider that is not configured', () => {
        const v = ParsedExternalSchema.parse({
            reference: {
                provider: 'nexus',
                project: 'mods/1',
                version: null,
                game: 'skyrim',
                class: null,
                url: 'nxm://skyrim/mods/1',
            },
            provider: {
                provider: 'nexus',
                label: 'Nexus Mods',
                state: { state: 'notConfigured', reason: 'off' },
            },
        })
        expect(v.provider.state.state).toBe('notConfigured')
        expect(v.reference.extras).toEqual({})
    })

    it('distinguishes the add outcomes', () => {
        expect(
            AddOutcomeSchema.parse({
                kind: 'installed',
                localId: 3,
                name: 'x',
                verified: 'sha512',
                files: 2,
            }).kind
        ).toBe('installed')
        expect(
            AddOutcomeSchema.parse({
                kind: 'openPage',
                url: 'https://x',
                reason: 'r',
            }).kind
        ).toBe('openPage')
        expect(() => AddOutcomeSchema.parse({ kind: 'nope' })).toThrow()
    })

    it('reads both inspection results', () => {
        const tree = {
            nodes: [],
            suggested: [],
            totalFiles: 0,
            totalBytes: 0,
            truncated: false,
        }
        expect(InspectionSchema.parse({ kind: 'needsSelection', tree }).kind).toBe(
            'needsSelection'
        )
    })

    it('reads pending links and scheme states', () => {
        expect(
            PendingSchema.parse({ kind: 'rejected', url: 'nxm://', reason: 'x' })
                .kind
        ).toBe('rejected')
        expect(
            SchemeStatusSchema.parse({
                state: 'registered',
                by: 'vortex',
                ours: false,
            }).state
        ).toBe('registered')
        expect(SchemeStatusSchema.parse({ state: 'free' }).state).toBe('free')
    })

    it('parses the credential report, which carries no secret field', () => {
        const v = CredentialReportSchema.parse({
            backend: 'encryptedFile',
            fallback: true,
            providers: [
                {
                    provider: 'nexus',
                    label: 'Nexus Mods',
                    configured: true,
                    source: 'user',
                    acceptsUserKey: true,
                    note: 'Using the key stored on this computer.',
                },
            ],
        })
        expect(v.fallback).toBe(true)
        expect(Object.keys(v.providers[0] ?? {})).not.toContain('value')
    })
})
