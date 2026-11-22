import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'

import { TREE } from '~/lib/external/selection.test'
import type { CredentialReportT } from '~/lib/external/schemas'

import { ConfirmExternalInstall } from './confirm-external-install'
import { FileSelectionPicker } from './file-selection-picker'
import { PasteExternalLink } from './paste-external-link'
import { credentialLabel, ProviderCredentials } from './provider-credentials'
import { SetupExternalDialog } from './setup-external-dialog'

const noop = () => undefined

describe('external components (static markup)', () => {
    it('credentials: password field, never a value, not-enabled providers have no input', () => {
        const report: CredentialReportT = {
            backend: 'encryptedFile',
            fallback: true,
            providers: [
                { provider: 'nexus', label: 'Nexus Mods', configured: true, source: 'user', acceptsUserKey: true, note: '' },
                { provider: 'curseforge', label: 'CurseForge', configured: false, source: 'none', acceptsUserKey: false, note: '' },
            ],
        }
        const html = renderToStaticMarkup(createElement(ProviderCredentials, { report, onSet: noop, onClear: noop }))

        expect(html).toContain('type="password"')
        expect(html).toContain('encrypted file')
        expect(html).toContain('Not enabled')
        expect(html).toContain('Replace')
        expect(html).toContain('Clear')
        expect(html.match(/type="password"/g)).toHaveLength(1)
        expect(credentialLabel(report.providers[1]!)).toBe('Not enabled')
    })

    it('setup: warns before replacing and offers the override as a separate box', () => {
        const html = renderToStaticMarkup(
            createElement(SetupExternalDialog, {
                mode: 'setup',
                reports: [
                    { scheme: 'nxm', status: { state: 'registered', by: 'Vortex', ours: false } },
                    { scheme: 'modrinth', status: { state: 'free' } },
                ],
                onApply: noop,
                onClose: noop,
            })
        )

        expect(html).toContain('role="dialog"')
        expect(html).toContain('will replace it')
        expect(html).toContain('Replace Vortex')
        expect(html).toContain('Recommended')
        // Only the free scheme starts ticked.
        expect(html).toMatch(/id="scheme-modrinth"[^>]*checked/)
        expect(html).not.toMatch(/id="scheme-nxm"[^>]*checked/)
    })

    it('confirm: shows facts and an explicit Install', () => {
        const url = 'https://modrinth.com/mod/sodium'
        const html = renderToStaticMarkup(
            createElement(ConfirmExternalInstall, {
                pending: {
                    kind: 'confirm',
                    provider: 'modrinth',
                    label: 'Modrinth',
                    url,
                    reference: { provider: 'modrinth', project: 'sodium', version: null, game: null, class: null, url, extras: {} },
                },
                onInstall: noop,
                onCancel: noop,
            })
        )

        expect(html).toContain('Modrinth')
        expect(html).toContain('sodium')
        expect(html).toContain('Install')
        expect(html).toContain('Nothing is downloaded until you choose Install')
    })

    it('picker: tree with tri-state semantics, filter and presets', () => {
        const html = renderToStaticMarkup(
            createElement(FileSelectionPicker, {
                tree: TREE,
                onConfirm: noop,
                onCancel: noop,
                initialLayout: { sort: { key: 'name', dir: 'asc' }, columns: { size: true, type: false } },
            })
        )

        expect(html).toContain('role="tree"')
        expect(html).toContain('Only .dll / .jar')
        expect(html).toContain('Expand all')
        expect(html).toContain('0 of 5 files selected')
        expect(html).toContain('aria-expanded="false"')
    })

    it('paste field: labelled, with progress and undo semantics', () => {
        const base = { onSubmit: noop, onUndo: noop, onDismiss: noop }
        const working = renderToStaticMarkup(
            createElement(PasteExternalLink, { ...base, state: { phase: 'working', step: 'installing', label: 'Downloading' } })
        )
        const done = renderToStaticMarkup(
            createElement(PasteExternalLink, { ...base, state: { phase: 'done', localId: 1, name: 'Sodium', files: 1, sandboxId: null } })
        )

        expect(working).toContain('role="progressbar"')
        expect(working).toContain('disabled')
        expect(done).toContain('Undo')
        expect(done).toContain('Added Sodium')
    })
})
