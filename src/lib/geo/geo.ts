import { z } from 'zod'

import { call } from '~/lib/ipc'

/**
 * The opt-in latency ESTIMATE: a distance-based guess from a local
 * geolocation database, never a measurement. The Rust side is
 * `commands/geo.rs`; with Settings, Servers, "Estimate latency" off, the
 * download and estimate commands refuse and nothing is fetched or resolved.
 */
export const GeoStatusSchema = z.object({
    phase: z.enum(['idle', 'downloading', 'converting']),
    bytesDone: z.number(),
    bytesTotal: z.number().nullable(),
    error: z.string().nullable(),
    sizeBytes: z.number().nullable(),
    updatedAt: z.number().nullable(),
    rangesV4: z.number(),
    rangesV6: z.number(),
    hasToken: z.boolean(),
    source: z.string(),
    selfIpUrl: z.string(),
    attribution: z.string(),
    license: z.string(),
    licenseUrl: z.string(),
})

export type GeoStatusT = z.infer<typeof GeoStatusSchema>

export type GeoConfigureT = {
    source?: string
    selfIpUrl?: string
    /** A new download token; `''` clears it. It is never read back. */
    token?: string
    reset?: boolean
}

export const geo = {
    status: () => call('geo_status', GeoStatusSchema, {}),
    configure: (input: GeoConfigureT) =>
        call('geo_configure', GeoStatusSchema, { input }),
    download: () => call('geo_download', GeoStatusSchema, {}),
    remove: () => call('geo_delete', GeoStatusSchema, {}),
    estimate: (hosts: string[]) =>
        call('geo_estimate', z.array(z.number().nullable()), { hosts }),
}

/** `~45 ms est.`: the wording that keeps an estimate apart from a ping. */
export function formatEstimate(ms: number | null | undefined): string | null {
    if (ms == null || !Number.isFinite(ms)) return null

    return `~${Math.round(ms)} ms est.`
}

export function formatBytes(n: number): string {
    if (n < 1024) return `${n} B`
    if (n < 1024 * 1024) return `${(n / 1024).toFixed(0)} KB`

    return `${(n / 1024 / 1024).toFixed(1)} MB`
}

/** Download progress as a whole percent, or null when the size is unknown. */
export function progressPercent(done: number, total: number | null): number | null {
    if (!total || total <= 0) return null

    return Math.min(100, Math.floor((done / total) * 100))
}
