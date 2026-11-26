import path from 'node:path'

import { describe, expect, it } from 'vitest'

import { checkRepository, fixtureProblems, lintFixtures, loadSchemas } from './check.ts'

describe('manifests:check', () => {
    it('passes on the seeds', () => {
        const { problems, files } = checkRepository()
        expect(problems).toEqual([])
        expect(files).toBeGreaterThan(10)
    })

    it('--release passes: the real exec key signs the loader manifests and no placeholder remains', () => {
        const { problems, markers } = checkRepository(undefined, true)
        expect(markers).toEqual([])
        expect(problems).toEqual([])
    })

    it('runs every ok fixture clean and fails every bad one with its own code', () => {
        const results = lintFixtures()
        expect(results.filter((r) => r.expected === null).length).toBeGreaterThanOrEqual(3)
        expect(results.filter((r) => r.expected !== null).length).toBeGreaterThanOrEqual(30)
        for (const r of results) {
            const codes = [...new Set(r.problems.map((p) => p.code))]
            expect([path.basename(r.file), codes]).toEqual([path.basename(r.file), r.expected ? [r.expected] : []])
        }
        expect(fixtureProblems(results)).toEqual([])
    })

    it('reports a step against its own action, not every oneOf branch', () => {
        const results = lintFixtures(undefined, loadSchemas())
        const cwd = results.find((r) => r.file.endsWith('schema--runinstaller-cwd.json'))
        expect(cwd.problems.map((p) => p.where)).toEqual(['/steps/1/cwd'])
    })
})
