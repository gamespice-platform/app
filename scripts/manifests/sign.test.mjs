import crypto from 'node:crypto'
import fs from 'node:fs'
import path from 'node:path'

import { describe, expect, it } from 'vitest'

import { ROOT } from './check.ts'
import { parse } from './canonical.ts'
import { TEST_KEY_ID, rawPublicKey, signManifest, verifySignature } from './sign.ts'

const DOC = { id: 'demo/loader/x', steps: [{ action: 'download', timeoutSecs: 600 }] }

function key() {
    const { privateKey } = crypto.generateKeyPairSync('ed25519')
    return { privateKey, entry: { keyId: 'ephemeral', ed25519: rawPublicKey(privateKey) } }
}

describe('exec manifest signatures', () => {
    it('verifies what it signs, and nothing else', () => {
        const { privateKey, entry } = key()
        const sig = signManifest(DOC, privateKey, [entry])
        expect(sig.keyId).toBe('ephemeral')
        expect(sig['x-tmc-placeholder']).toBeUndefined()
        expect(verifySignature(DOC, sig, [entry])).toBeNull()

        const edited = structuredClone(DOC)
        edited.steps[0].timeoutSecs = 601
        expect(verifySignature(edited, sig, [entry])?.code).toBe('signature-invalid')
    })

    it('does not care about key order or $schema', () => {
        const { privateKey, entry } = key()
        const sig = signManifest(DOC, privateKey, [entry])
        const reordered = { $schema: 'x', steps: DOC.steps, id: DOC.id }
        expect(verifySignature(reordered, sig, [entry])).toBeNull()
    })

    it('refuses to sign with a key exec-keys.json does not list', () => {
        const { privateKey } = key()
        expect(() => signManifest(DOC, privateKey, [key().entry])).toThrow(/not in exec-keys/)
    })

    it('names unknown keys and malformed signatures', () => {
        const { privateKey, entry } = key()
        const sig = signManifest(DOC, privateKey, [entry])
        expect(verifySignature(DOC, sig, [])?.code).toBe('signature-unknown-key')
        expect(verifySignature(DOC, { ...sig, signature: 'abc' }, [entry])?.code).toBe('signature-malformed')
        expect(verifySignature(DOC, { ...sig, extra: 1 }, [entry])?.code).toBe('signature-malformed')
    })

    for (const loader of ['forge', 'neoforge']) {
        it(`holds ${loader}'s manifest to its real-key signature, which a one-byte edit breaks`, () => {
            const file = path.join(ROOT, `manifests/minecraft/loaders/${loader}.manifest.json`)
            const keys = parse(fs.readFileSync(path.join(ROOT, 'manifests/keys/exec-keys.json'), 'utf8'))
            const text = fs.readFileSync(file, 'utf8')
            const sig = parse(fs.readFileSync(`${file}.sig`, 'utf8'))
            expect(sig.keyId).toBe('tmc-exec-2026')
            expect(sig['x-tmc-placeholder']).toBeUndefined()
            expect(verifySignature(parse(text), sig, keys)).toBeNull()

            const edited = text.replace('"timeoutSecs": 900', '"timeoutSecs": 901')
            expect(edited).not.toBe(text)
            expect(verifySignature(parse(edited), sig, keys)?.code).toBe('signature-invalid')
        })
    }
})
