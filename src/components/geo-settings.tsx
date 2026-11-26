import { useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Button } from '@modcommunity/shared'

import { Row, Toggle } from '~/components/form'
import { useSettings } from '~/lib/settings/provider'
import { formatBytes, geo, progressPercent } from '~/lib/geo/geo'

/**
 * Settings, Servers: the opt-in latency estimate. Off, it shows one toggle and
 * does nothing else (no status polling, no download). On, it shows the
 * database's state, the download token and source, and the licence line that
 * IP2Location LITE's CC BY-SA 4.0 requires.
 */
export default function GeoSettings() {
    const { app, setApp } = useSettings()
    const on = !!app?.estimateLatency
    const qc = useQueryClient()
    const [token, setToken] = useState('')
    const [source, setSource] = useState<string | null>(null)

    const status = useQuery({
        queryKey: ['geo-status'],
        queryFn: geo.status,
        enabled: on,
        // Poll only while a download runs.
        refetchInterval: (q) => (q.state.data?.phase === 'idle' ? false : 500),
    })

    const refresh = (s: unknown) => qc.setQueryData(['geo-status'], s)

    const configure = useMutation({
        mutationFn: geo.configure,
        onSuccess: (s) => {
            refresh(s)
            setToken('')
            setSource(null)
        },
    })

    const download = useMutation({
        mutationFn: async () => {
            // Poll status while the command blocks, so progress moves.
            return geo.download()
        },
        onSettled: () => {
            void qc.invalidateQueries({ queryKey: ['geo-status'] })
            void qc.invalidateQueries({ queryKey: ['geo-estimate'] })
        },
    })

    const remove = useMutation({
        mutationFn: geo.remove,
        onSuccess: (s) => {
            refresh(s)
            void qc.invalidateQueries({ queryKey: ['geo-estimate'] })
        },
    })

    const s = status.data
    const busy = s != null && s.phase !== 'idle'
    const pct = s ? progressPercent(s.bytesDone, s.bytesTotal) : null
    const error =
        download.error?.message ??
        configure.error?.message ??
        remove.error?.message ??
        s?.error ??
        status.error?.message

    return (
        <>
            <Row
                label="Estimate latency"
                hint="Show a distance-based guess (for example ~45 ms est.) beside servers. Turning this on downloads a free geolocation database and looks server addresses up on this computer; your public address is fetched from one web request."
                control={
                    <Toggle
                        label="Estimate latency"
                        checked={on}
                        onChange={(estimateLatency) =>
                            void setApp({ estimateLatency })
                        }
                    />
                }
            />

            {on && (
                <div className="space-y-3 rounded-xl border border-border bg-surface p-3 text-sm">
                    {s && (
                        <p className="text-xs text-muted">
                            {s.sizeBytes != null
                                ? `Database: ${formatBytes(s.sizeBytes)}, ${(s.rangesV4 + s.rangesV6).toLocaleString()} ranges, updated ${
                                      s.updatedAt
                                          ? new Date(
                                                s.updatedAt * 1000
                                            ).toLocaleDateString()
                                          : 'unknown'
                                  }. Refreshed monthly.`
                                : 'No database yet. Add a download token and press Download.'}
                        </p>
                    )}

                    {busy && s && (
                        <div>
                            <div
                                role="progressbar"
                                aria-label="Database download"
                                aria-valuenow={pct ?? undefined}
                                className="h-2 overflow-hidden rounded bg-border"
                            >
                                <div
                                    className="h-full bg-accent"
                                    style={{ width: `${pct ?? 100}%` }}
                                />
                            </div>
                            <p className="mt-1 text-xs text-muted">
                                {s.phase === 'converting'
                                    ? 'Building the database…'
                                    : `${formatBytes(s.bytesDone)}${
                                          s.bytesTotal
                                              ? ` of ${formatBytes(s.bytesTotal)}`
                                              : ''
                                      }`}
                            </p>
                        </div>
                    )}

                    {error && (
                        <p role="alert" className="text-xs text-danger">
                            {error}
                        </p>
                    )}

                    <div className="flex flex-wrap items-center gap-2">
                        <input
                            type="password"
                            aria-label="IP2Location download token"
                            autoComplete="off"
                            placeholder={
                                s?.hasToken
                                    ? 'Token saved (enter a new one to replace)'
                                    : 'Download token'
                            }
                            value={token}
                            onChange={(e) => setToken(e.target.value)}
                            className="min-w-64 rounded border border-border bg-surface px-2 py-1 text-xs"
                        />
                        <Button
                            btnType="secondary"
                            disabled={!token.trim() || configure.isPending}
                            onClick={() => configure.mutate({ token })}
                        >
                            Save token
                        </Button>
                        <Button
                            disabled={
                                busy ||
                                download.isPending ||
                                (!s?.hasToken && !!s?.source.includes('{token}'))
                            }
                            onClick={() => download.mutate()}
                        >
                            {s?.sizeBytes != null ? 'Update now' : 'Download'}
                        </Button>
                        <Button
                            btnType="secondary"
                            disabled={
                                busy || s?.sizeBytes == null || remove.isPending
                            }
                            onClick={() => remove.mutate()}
                        >
                            Delete database
                        </Button>
                    </div>

                    <details className="text-xs">
                        <summary className="cursor-pointer text-muted">
                            Download source
                        </summary>
                        <div className="mt-2 flex flex-wrap items-center gap-2">
                            <input
                                aria-label="Database download URL"
                                value={source ?? s?.source ?? ''}
                                onChange={(e) => setSource(e.target.value)}
                                className="min-w-96 flex-1 rounded border border-border bg-surface px-2 py-1"
                            />
                            <Button
                                btnType="secondary"
                                disabled={source == null || configure.isPending}
                                onClick={() =>
                                    configure.mutate({ source: source ?? '' })
                                }
                            >
                                Save
                            </Button>
                            <Button
                                btnType="secondary"
                                onClick={() => configure.mutate({ reset: true })}
                            >
                                Reset
                            </Button>
                        </div>
                        <p className="mt-1 text-muted">
                            Must be https. <code>{'{token}'}</code> is replaced by
                            your token. Free tokens come from your IP2Location LITE
                            account and are limited to a few downloads a day.
                        </p>
                    </details>

                    <p className="text-xs text-muted">
                        {s?.attribution ??
                            'This product includes IP2Location LITE data available from https://lite.ip2location.com.'}{' '}
                        Licensed {s?.license ?? 'CC BY-SA 4.0'}.
                    </p>
                </div>
            )}
        </>
    )
}
