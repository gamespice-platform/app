/*
 * Exec manifest signatures (docs/loader-manifests.md, "Trust").
 *
 * A signature is pure Ed25519 over the manifest's canonical bytes
 * (canonical.ts), not over their sha256: Ed25519 hashes internally, and this
 * is what plugin signatures already do. It lives next to the manifest as
 * `<file>.sig`, a small JSON document in the same pretty form:
 *
 *     { "alg": "ed25519", "keyId": "...", "signature": "<base64, 64 bytes>" }
 *
 * The public keys are `manifests/keys/exec-keys.json`:
 * `[{ "keyId": "...", "ed25519": "<base64, the raw 32-byte key>" }]`.
 *
 * CLI: `npm run manifests:sign -- <manifest>...` signs with the PKCS#8 PEM
 * private key at `$TMC_EXEC_SIGNING_KEY`. The key id is found by matching the
 * key's public half against exec-keys.json, so an unlisted key cannot sign.
 * A signature by the test key carries an `x-tmc-placeholder`, which
 * `manifests:check --release` refuses, as it refuses the test key id itself.
 */

import crypto from 'node:crypto'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

import { canonicalBytes, parse, prettyString } from './canonical.ts'

export const TEST_KEY_ID = 'tmc-exec-test'
export const TEST_KEY_NOTE =
    'signed with the test key tmc-exec-test; the owner re-signs with the real exec key before release'

export type ExecKey = { keyId: string; ed25519: string; 'x-tmc-placeholder'?: string }
export type Signature = {
    alg: 'ed25519'
    keyId: string
    signature: string
    'x-tmc-placeholder'?: string
}
export type SigProblem = { code: string; message: string }

const B64 = /^[A-Za-z0-9+/]+={0,2}$/

function b64Bytes(s: unknown, length: number): Buffer | null {
    if (typeof s !== 'string' || !B64.test(s)) return null
    const bytes = Buffer.from(s, 'base64')
    return bytes.length === length && bytes.toString('base64') === s ? bytes : null
}

export function publicKeyFromRaw(raw: string): crypto.KeyObject {
    const bytes = b64Bytes(raw, 32)
    if (!bytes) throw new Error('an ed25519 key is 32 bytes of base64')
    return crypto.createPublicKey({
        key: { kty: 'OKP', crv: 'Ed25519', x: bytes.toString('base64url') },
        format: 'jwk',
    })
}

export function rawPublicKey(key: crypto.KeyObject): string {
    const pub = key.type === 'private' ? crypto.createPublicKey(key) : key
    const jwk = pub.export({ format: 'jwk' })
    if (jwk.crv !== 'Ed25519' || typeof jwk.x !== 'string') {
        throw new Error('not an Ed25519 key')
    }
    return Buffer.from(jwk.x, 'base64url').toString('base64')
}

/** The shape problems of an exec-keys.json document, as messages. */
export function execKeysProblems(keys: unknown): string[] {
    if (!Array.isArray(keys)) return ['exec-keys.json must be an array']
    const out: string[] = []
    const seen = new Set<string>()
    keys.forEach((k, i) => {
        const allowed = new Set(['keyId', 'ed25519', 'x-tmc-placeholder'])
        if (typeof k !== 'object' || k === null || Array.isArray(k)) {
            out.push(`/${i}: not an object`)
            return
        }
        for (const f of Object.keys(k)) {
            if (!allowed.has(f)) out.push(`/${i}: unknown field ${f}`)
        }
        if (typeof k.keyId !== 'string' || !/^[a-z0-9-]{1,64}$/.test(k.keyId)) {
            out.push(`/${i}: keyId must match ^[a-z0-9-]{1,64}$`)
        } else if (seen.has(k.keyId)) {
            out.push(`/${i}: duplicate keyId ${k.keyId}`)
        } else {
            seen.add(k.keyId)
        }
        if (!b64Bytes(k.ed25519, 32)) out.push(`/${i}: ed25519 must be 32 bytes of base64`)
    })
    return out
}

export function signManifest(
    doc: unknown,
    privateKey: crypto.KeyObject,
    keys: ExecKey[],
): Signature {
    const raw = rawPublicKey(privateKey)
    const entry = keys.find((k) => k.ed25519 === raw)
    if (!entry) {
        throw new Error(
            `this key is not in exec-keys.json (its public half is ${raw}); add it there first`,
        )
    }
    const sig: Signature = {
        alg: 'ed25519',
        keyId: entry.keyId,
        signature: crypto.sign(null, canonicalBytes(doc), privateKey).toString('base64'),
    }
    if (entry.keyId === TEST_KEY_ID) sig['x-tmc-placeholder'] = TEST_KEY_NOTE
    return sig
}

/** Null when `sig` is a valid signature of `doc` by a key in `keys`. */
export function verifySignature(
    doc: unknown,
    sig: unknown,
    keys: ExecKey[],
): SigProblem | null {
    if (typeof sig !== 'object' || sig === null || Array.isArray(sig)) {
        return { code: 'signature-malformed', message: 'the .sig is not a JSON object' }
    }
    const s = sig as Record<string, unknown>
    const extra = Object.keys(s).filter(
        (k) => !['alg', 'keyId', 'signature', 'x-tmc-placeholder'].includes(k),
    )
    const bytes = b64Bytes(s.signature, 64)
    if (s.alg !== 'ed25519' || typeof s.keyId !== 'string' || !bytes || extra.length) {
        return {
            code: 'signature-malformed',
            message:
                'the .sig must be { "alg": "ed25519", "keyId": "...", "signature": "<64 bytes of base64>" }',
        }
    }
    const entry = keys.find((k) => k.keyId === s.keyId)
    if (!entry) {
        return {
            code: 'signature-unknown-key',
            message: `signed by ${s.keyId}, which is not in exec-keys.json`,
        }
    }
    let ok = false
    try {
        ok = crypto.verify(null, canonicalBytes(doc), publicKeyFromRaw(entry.ed25519), bytes)
    } catch {
        ok = false
    }
    return ok
        ? null
        : {
              code: 'signature-invalid',
              message: `the signature by ${s.keyId} does not match this manifest's canonical bytes`,
          }
}

function main(argv: string[]): number {
    const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..')
    let keysPath = path.join(root, 'manifests/keys/exec-keys.json')
    const files: string[] = []
    for (let i = 0; i < argv.length; i++) {
        if (argv[i] === '--keys') keysPath = path.resolve(argv[++i] ?? '')
        else files.push(argv[i])
    }
    const keyFile = process.env.TMC_EXEC_SIGNING_KEY
    if (!keyFile || files.length === 0) {
        console.error(
            'usage: TMC_EXEC_SIGNING_KEY=<pkcs8 pem> npm run manifests:sign -- [--keys exec-keys.json] <manifest>...',
        )
        return 2
    }
    const privateKey = crypto.createPrivateKey(fs.readFileSync(keyFile))
    const keys = parse(fs.readFileSync(keysPath, 'utf8')) as ExecKey[]
    const shape = execKeysProblems(keys)
    if (shape.length) {
        console.error(`${keysPath}: ${shape.join('; ')}`)
        return 1
    }
    for (const file of files) {
        const text = fs.readFileSync(file, 'utf8')
        const doc = parse(text)
        if (prettyString(doc) !== text) {
            console.error(`${file}: not in pretty canonical form; npm run manifests:check says how`)
            return 1
        }
        const sig = signManifest(doc, privateKey, keys)
        fs.writeFileSync(`${file}.sig`, prettyString(sig))
        console.log(`${file}.sig: signed by ${sig.keyId}`)
    }
    return 0
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
    process.exitCode = main(process.argv.slice(2))
}
