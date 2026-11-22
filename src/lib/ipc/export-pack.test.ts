import { beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn()

vi.mock('@tauri-apps/api/core', () => ({
    invoke: (...a: unknown[]): unknown => invoke(...a),
}))

import { ipc } from './commands'
import { ExportKindSchema } from './schemas'

describe('sandboxExportPack', () => {
    beforeEach(() => invoke.mockReset())

    it('sends an id and a kind, never a path, and parses the report', async () => {
        invoke.mockResolvedValue({
            fileName: 'My pack.mrpack',
            files: 3,
            bytes: 1024,
            notes: ['one mod has no download link'],
        })

        const report = await ipc.sandboxExportPack(7, 'mrpack')

        expect(invoke).toHaveBeenCalledWith('sandbox_export_pack', {
            id: 7,
            kind: 'mrpack',
        })
        expect(report.fileName).toBe('My pack.mrpack')
        expect(report.notes).toHaveLength(1)
    })

    it('defaults missing notes to an empty list', async () => {
        invoke.mockResolvedValue({ fileName: 'a.zip', files: 0, bytes: 0 })

        await expect(ipc.sandboxExportPack(1, 'share')).resolves.toMatchObject({
            notes: [],
        })
    })

    it('rejects a reply that is not a report', async () => {
        invoke.mockResolvedValue({ fileName: 5 })
        vi.spyOn(console, 'error').mockImplementation(() => {})

        await expect(ipc.sandboxExportPack(1, 'share')).rejects.toThrow()
    })

    it('knows exactly two kinds', () => {
        expect(ExportKindSchema.options).toEqual(['mrpack', 'share'])
    })
})
