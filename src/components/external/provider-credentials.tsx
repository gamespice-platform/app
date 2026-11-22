import { useState } from 'react'

import type { CredentialReportT, ProviderIdT } from '~/lib/external/schemas'

import { buttonPlain, buttonPrimary } from './modal'

/**
 * Which providers work and where each key lives. A secret is never displayed:
 * the field is write-only (`type=password`) and is emptied after saving.
 * Providers the site has not enabled read "Not enabled".
 */
export function ProviderCredentials({
    report,
    onSet,
    onClear,
    busy = false,
    error,
}: {
    report: CredentialReportT
    onSet: (provider: ProviderIdT, value: string) => void | Promise<void>
    onClear: (provider: ProviderIdT) => void | Promise<void>
    busy?: boolean
    error?: string | null
}) {
    return (
        <div className="flex flex-col gap-3">
            <p className="text-xs text-muted">
                Stored in: {report.backend === 'keychain' ? 'the system keychain' : report.backend === 'encryptedFile' ? 'an encrypted file' : 'memory only'}
            </p>
            {report.fallback && (
                <p role="alert" className="rounded-lg border border-border bg-surface-2 p-2 text-xs text-warning">
                    The system keychain is not available, so keys are kept in an encrypted file on this device.
                    Anyone who can read your user files and run this app can use them.
                </p>
            )}
            {error && (
                <p role="alert" className="text-xs text-danger">
                    {error}
                </p>
            )}
            <ul className="flex flex-col divide-y divide-border rounded-xl border border-border bg-surface">
                {report.providers.map((p) => (
                    <CredentialRow key={p.provider} p={p} busy={busy} onSet={onSet} onClear={onClear} />
                ))}
            </ul>
        </div>
    )
}

export function credentialLabel(p: CredentialReportT['providers'][number]): string {
    if (p.configured) return p.source === 'site' ? 'Configured by GameSpice' : 'Configured'

    return p.acceptsUserKey ? 'Not configured' : 'Not enabled'
}

function CredentialRow({
    p,
    busy,
    onSet,
    onClear,
}: {
    p: CredentialReportT['providers'][number]
    busy: boolean
    onSet: (provider: ProviderIdT, value: string) => void | Promise<void>
    onClear: (provider: ProviderIdT) => void | Promise<void>
}) {
    const [value, setValue] = useState('')
    const field = `cred-${p.provider}`

    return (
        <li className="flex flex-col gap-2 p-3">
            <div className="flex items-center justify-between gap-3">
                <span className="text-sm font-medium">{p.label}</span>
                <span className={`text-xs ${p.configured ? 'text-accent' : 'text-muted'}`}>{credentialLabel(p)}</span>
            </div>
            {p.note && <p className="text-xs text-muted">{p.note}</p>}
            {p.acceptsUserKey && (
                <form
                    className="flex flex-wrap items-center gap-2"
                    onSubmit={async (e) => {
                        e.preventDefault()

                        if (!value) return

                        await onSet(p.provider, value)
                        setValue('')
                    }}
                >
                    <label htmlFor={field} className="sr-only">
                        {p.label} key
                    </label>
                    <input
                        id={field}
                        type="password"
                        autoComplete="off"
                        spellCheck={false}
                        value={value}
                        onChange={(e) => setValue(e.target.value)}
                        placeholder={p.configured ? 'Paste a new key to replace it' : 'Paste your key'}
                        className="min-w-0 flex-1 rounded-lg border border-border bg-surface-2 px-2 py-1.5 text-sm"
                    />
                    <button type="submit" disabled={!value || busy} className={buttonPrimary}>
                        {p.configured ? 'Replace' : 'Save'}
                    </button>
                    {p.configured && p.source === 'user' && (
                        <button type="button" disabled={busy} onClick={() => void onClear(p.provider)} className={buttonPlain}>
                            Clear
                        </button>
                    )}
                </form>
            )}
        </li>
    )
}
