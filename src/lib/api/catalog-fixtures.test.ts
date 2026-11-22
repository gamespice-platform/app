/**
 * The W5 catalog fixtures (synced from website-city by `contract:sync`) parse
 * through the synced contract and through the IPC view schemas the pickers
 * read. The Rust side parses the same files in `tmc_core::catalog::tests`.
 */

import { describe, expect, it } from 'vitest'

import {
    CatalogErrorWireSchema,
    CatalogWireSchema,
    GameVersionPageWireSchema,
    InstallPlanWireSchema,
    InstallSchema,
    LoaderVersionListWireSchema,
    RuntimeWireSchema,
} from './contract'
import { CatalogViewSchema, LoaderVersionsViewSchema } from '../ipc/schemas'

import catalog from './fixtures/catalog/catalog.json'
import versions from './fixtures/catalog/versions.json'
import loaderVersions from './fixtures/catalog/loader-versions.json'
import installPlan from './fixtures/catalog/install-plan.json'
import runtime from './fixtures/catalog/runtime.json'
import wireCatalog from './fixtures/catalog/wire/catalog.json'
import wireLoaderVersions from './fixtures/catalog/wire/loader-versions.json'
import wireInstallPlan from './fixtures/catalog/wire/install-plan.json'
import wireRuntime from './fixtures/catalog/wire/runtime.json'
import wireError from './fixtures/catalog/wire/error.json'
import wireInstallSync from './fixtures/catalog/wire/app-install-sync.json'

describe('catalog fixtures', () => {
    it('parse through the contract', () => {
        for (const c of [catalog, wireCatalog]) CatalogWireSchema.parse(c)
        GameVersionPageWireSchema.parse(versions)
        for (const l of [loaderVersions, wireLoaderVersions]) LoaderVersionListWireSchema.parse(l)
        for (const p of [installPlan, wireInstallPlan]) InstallPlanWireSchema.parse(p)
        for (const r of [runtime, wireRuntime]) RuntimeWireSchema.parse(r)
    })

    it('carry the withdrawal note at the top of the error envelope', () => {
        const err = CatalogErrorWireSchema.parse(wireError)
        expect(err).toMatchObject({ ok: false, code: 'catalog_withdrawn' })
        expect(wireError.note).toBe('Crashes on world load; use 54.1.0')
    })

    it('parse as the views catalog_get and catalog_loader_versions return', () => {
        const provenance = { source: 'cache', stale: true, fetchedAt: 1_763_800_000_000 }
        const view = CatalogViewSchema.parse({ ...wireCatalog, ...provenance })
        expect(view.versions.items.length).toBeGreaterThan(0)
        LoaderVersionsViewSchema.parse({ ...wireLoaderVersions, ...provenance })

        const compiled = { source: 'compiled', stale: false, fetchedAt: null }
        CatalogViewSchema.parse({
            ...wireCatalog,
            versions: { items: [], nextCursor: null },
            generatedAt: null,
            ...compiled,
        })
        expect(() => CatalogViewSchema.parse({ ...wireCatalog, source: 'disk', stale: false, fetchedAt: null })).toThrow()
    })

    /*
     * The fixture's install id is a placeholder string; the contract's install
     * id is an integer, and that is the one the app keeps. Every other field
     * the sync adds must match `InstallSchema`.
     */
    it('agree with InstallSchema on every sync field but the placeholder id', () => {
        const { id, ...rest } = wireInstallSync
        expect(typeof id).toBe('string')
        const sync = InstallSchema.pick({
            appId: true,
            name: true,
            gameVersion: true,
            loader: true,
            loaderVersion: true,
        }).strict()
        expect(sync.parse(rest)).toEqual(rest)
        expect(InstallSchema.shape.id.safeParse(7).success).toBe(true)
    })
})
