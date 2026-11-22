import fs from 'node:fs'
import path from 'node:path'

import { describe, expect, it } from 'vitest'

import { CanonicalError, canonicalString, parse, prettyString } from './canonical.ts'
import { ROOT } from './check.ts'

const DIR = path.join(ROOT, 'schemas/fixtures/canonical')
const NAMES = fs
    .readdirSync(DIR)
    .filter((f) => f.endsWith('.json'))
    .map((f) => f.slice(0, -'.json'.length))

describe('the canonical form', () => {
    it('has fixtures', () => {
        expect(NAMES.length).toBeGreaterThanOrEqual(6)
    })

    for (const name of NAMES) {
        it(`reproduces ${name}.canonical byte for byte`, () => {
            const doc = parse(fs.readFileSync(path.join(DIR, `${name}.json`), 'utf8'))
            const want = fs.readFileSync(path.join(DIR, `${name}.canonical`))
            expect(Buffer.from(canonicalString(doc), 'utf8').equals(want)).toBe(true)
        })

        it(`is a fixed point for ${name}`, () => {
            const once = canonicalString(parse(fs.readFileSync(path.join(DIR, `${name}.json`), 'utf8')))
            expect(canonicalString(parse(once))).toBe(once)
        })
    }

    it('sorts keys by code point, not UTF-16 unit', () => {
        // U+FF61 sorts before U+1F600 by code point, after it by UTF-16 unit.
        expect(canonicalString({ '\u{1F600}': 1, '｡': 2 })).toBe('{"｡":2,"\u{1F600}":1}')
    })

    it('refuses floats, unsafe integers and lone surrogates', () => {
        expect(() => canonicalString({ a: 1.5 })).toThrow(CanonicalError)
        expect(() => canonicalString({ a: 2 ** 60 })).toThrow(CanonicalError)
        expect(() => canonicalString({ a: '\uD800' })).toThrow(CanonicalError)
    })

    it('drops only the top-level $schema, and puts it first when pretty', () => {
        const doc = { b: { $schema: 'x' }, $schema: 's', a: 1 }
        expect(canonicalString(doc)).toBe('{"a":1,"b":{"$schema":"x"}}')
        expect(prettyString(doc).split('\n')[1]).toBe('    "$schema": "s",')
    })
})
