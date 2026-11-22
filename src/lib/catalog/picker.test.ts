import { describe, expect, it } from 'vitest'

import { CatalogWireSchema, LoaderVersionListWireSchema } from '../api/contract'
import catalogJson from '../api/fixtures/catalog/catalog.json'
import loaderVersionsJson from '../api/fixtures/catalog/loader-versions.json'
import {
    carryLoaderVersion,
    checkCustomVersion,
    checkboxConsents,
    defaultLoaderVersion,
    execHost,
    expand,
    hasCatalog,
    loaderOptions,
    loaderVersionOptions,
    piecesText,
    planSteps,
    provenanceNotice,
    stepView,
    trustLine,
    versionOptions,
} from './picker'
import { ct } from './text'

const catalog = CatalogWireSchema.parse(catalogJson)
const forge = LoaderVersionListWireSchema.parse(loaderVersionsJson).items

describe('custom versions', () => {
    it.each(['1.21.4-custom', '24w14a', '1.20.1+build.3', 'a'])('accepts %s', (v) => {
        expect(checkCustomVersion(v)).toBeNull()
    })
    it.each(['../x', '..', '1..2', '', ' ', '-1', 'a b', 'a/b', 'a\\b', '{gameVersion}', 'x'.repeat(65)])(
        'refuses %j',
        (v) => {
            expect(checkCustomVersion(v)).toBe(ct('catalog.version.customBad'))
        }
    )
})

describe('versionOptions', () => {
    it('puts Any first and keeps the served order', () => {
        const rows = versionOptions(catalog.versions.items, null)
        expect(rows[0]).toEqual({ value: '', label: 'Any version' })
        expect(rows.slice(1).map((r) => r.value)).toEqual(
            catalog.versions.items.map((v) => v.versionId)
        )
    })
    it('adds an unlisted current value as custom', () => {
        const rows = versionOptions(catalog.versions.items, '1.21.4-custom')
        const last = rows[rows.length - 1]
        expect(last?.value).toBe('1.21.4-custom')
        expect(last?.label).toBe('1.21.4-custom (custom)')
        expect(last?.hint).toBe('TMC will not install this; install it yourself.')
    })
    it('marks unmanaged, preview and deprecated rows, and hides withdrawn ones', () => {
        const base = catalog.versions.items[0]!
        const rows = versionOptions(
            [
                { ...base, versionId: 'a', managed: false },
                { ...base, versionId: 'b', channel: 'PREVIEW', channelLabel: 'Snapshot' },
                { ...base, versionId: 'c', deprecated: { at: '2026-01-01T00:00:00Z', note: 'broken', withdrawn: false } },
                { ...base, versionId: 'd', deprecated: { at: '2026-01-01T00:00:00Z', note: null, withdrawn: true } },
            ],
            null
        )
        expect(rows.map((r) => r.hint)).toEqual([
            undefined,
            'Not installed by TMC',
            'Snapshot',
            'Deprecated: broken',
        ])
    })
})

describe('loaderOptions', () => {
    it('starts with None (vanilla) and sorts by position', () => {
        const rows = loaderOptions(catalog.loaders, 'forge')
        expect(rows[0]?.label).toBe('None (vanilla)')
        const sorted = [...catalog.loaders].sort((a, b) => a.position - b.position)
        expect(rows.slice(1).map((r) => r.value)).toEqual(sorted.map((l) => l.key))
    })
    it('keeps free text from before the catalog as an unknown loader', () => {
        const rows = loaderOptions(catalog.loaders, 'optifine')
        expect(rows[rows.length - 1]).toMatchObject({
            value: 'optifine',
            label: 'Unknown loader “optifine”',
        })
    })
    it('matches a loader key whatever its case', () => {
        expect(loaderOptions(catalog.loaders, 'Forge')).toHaveLength(catalog.loaders.length + 1)
    })
})

describe('loader versions', () => {
    it('marks the recommended build', () => {
        const rows = loaderVersionOptions(forge, null)
        expect(rows.map((r) => [r.value, r.hint])).toEqual([
            ['54.1.0', 'Recommended'],
            ['54.0.34', undefined],
        ])
    })
    it('defaults to recommended, else newest', () => {
        expect(defaultLoaderVersion(forge)).toBe('54.1.0')
        expect(defaultLoaderVersion(forge.map((v) => ({ ...v, recommended: false })))).toBe('54.1.0')
        expect(defaultLoaderVersion([])).toBeNull()
    })
    const names = { loader: 'Forge', gameVersion: '1.21.4' }
    it('keeps a version the new game version has a build of', () => {
        expect(carryLoaderVersion(forge, '54.0.34', names)).toEqual({ value: '54.0.34', notice: null })
    })
    it('moves to the recommended build and says so', () => {
        expect(carryLoaderVersion(forge, '53.0.1', names)).toEqual({
            value: '54.1.0',
            notice: 'Forge 53.0.1 has no build for 1.21.4; switched to 54.1.0.',
        })
    })
    it('picks quietly when there was none', () => {
        expect(carryLoaderVersion(forge, null, names)).toEqual({ value: '54.1.0', notice: null })
    })
    it('keeps the old value and says so when there is no build at all', () => {
        expect(carryLoaderVersion([], '53.0.1', names)).toEqual({
            value: '53.0.1',
            notice: 'No Forge build for 1.21.4',
        })
    })
})

describe('provenance', () => {
    it('says nothing about a fresh list', () => {
        expect(provenanceNotice({ source: 'remote', stale: false, fetchedAt: 1 })).toBeNull()
        expect(provenanceNotice({ source: 'cache', stale: false, fetchedAt: 1 })).toBeNull()
    })
    it('dates a stale cache', () => {
        expect(
            provenanceNotice({ source: 'cache', stale: true, fetchedAt: 5 }, (ms) => `t${ms}`)
        ).toBe('Offline: list from t5.')
    })
    it('flags the compiled snapshot', () => {
        expect(provenanceNotice({ source: 'compiled', stale: false, fetchedAt: null })).toMatch(
            /shipped with the app/
        )
    })
    it('treats an empty catalog as none', () => {
        expect(hasCatalog({ loaders: [], versions: { items: [] } })).toBe(false)
        expect(hasCatalog(catalog)).toBe(true)
    })
})

const values = { gameVersion: '1.21.4', loaderVersion: '54.1.0', javaMajor: '21' }

describe('expand', () => {
    it('fills known values and keeps the rest as tokens', () => {
        expect(expand('forge-{gameVersion}-{loaderVersion}-installer.jar', values)).toEqual([
            { text: 'forge-1.21.4-54.1.0-installer.jar', token: false },
        ])
        expect(expand('{artifact.installer}', values)).toEqual([
            { text: '{artifact.installer}', token: true },
        ])
        expect(piecesText(expand('java {var.javaMajor}', values))).toBe('java 21')
    })
})

const forgeLayer = {
    manifestKey: 'minecraft/loader/forge',
    revision: 1,
    title: 'Forge',
    layerHash: 'ab',
    exec: true,
    values,
    downloadHosts: ['maven.minecraftforge.net'],
    steps: [
        {
            action: 'download',
            artifact: 'installer',
            to: { root: 'downloads', path: 'forge-{gameVersion}-{loaderVersion}-installer.jar' },
        },
        {
            action: 'runInstaller',
            program: '{runtime.java}',
            args: ['-jar', '{artifact.installer}', '--installClient', '{layerFiles}'],
            cwd: '{layerWork}',
        },
    ],
}

describe('plan steps', () => {
    it('lays out the full argv, one element per argument', () => {
        const v = stepView(forgeLayer.steps[1]!, values)
        expect(v.argv?.map(piecesText)).toEqual([
            '{runtime.java}',
            '-jar',
            '{artifact.installer}',
            '--installClient',
            '{layerFiles}',
        ])
        expect(v.cwd && piecesText(v.cwd)).toBe('{layerWork}')
    })
    it('shows where a download lands', () => {
        const v = stepView(forgeLayer.steps[0]!, values)
        expect(v.target && piecesText(v.target)).toBe('downloads/forge-1.21.4-54.1.0-installer.jar')
        expect(v.artifact).toBe('installer')
    })
    it('numbers steps across layers', () => {
        const groups = planSteps({ layers: [forgeLayer, { ...forgeLayer, manifestKey: 'x' }] })
        expect(groups.flatMap((g) => g.steps.map((s) => s.n))).toEqual([1, 2, 3, 4])
    })
    it('names the installer host and the trust', () => {
        expect(execHost({ layers: [forgeLayer, { ...forgeLayer, exec: false, downloadHosts: ['x'] }] })).toBe(
            'maven.minecraftforge.net'
        )
        expect(trustLine({ trust: 'official', planHash: 'deadbeefcafe0011' })).toBe(
            'Published by TMC, signed. Plan deadbeefcafe.'
        )
        expect(trustLine({ trust: 'local', planHash: 'x' })).toMatch(/^Local file, unsigned/)
    })
    it('gives only manifest consents a checkbox', () => {
        expect(
            checkboxConsents([{ key: 'exec:minecraft/loader/forge' }, { key: 'runtime:java-21' }, { key: 'eula' }])
        ).toEqual([{ key: 'eula' }])
    })
})
