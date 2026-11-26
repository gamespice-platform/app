import { useQuery } from '@tanstack/react-query'

import { useSettings } from '~/lib/settings/provider'
import { geo, formatEstimate } from './geo'

/**
 * The estimate for one host, as display text, or null.
 *
 * Gated on the setting: with it off the query is disabled, so no IPC call is
 * made and the backend is never asked to resolve or look anything up.
 * Estimates barely change, so they are cached for a long time per host.
 */
export function useLatencyEstimate(host: string | null | undefined): string | null {
    const { app } = useSettings()
    const on = !!app?.estimateLatency && !!host

    const q = useQuery({
        queryKey: ['geo-estimate', host],
        queryFn: async () => (await geo.estimate([host as string]))[0] ?? null,
        enabled: on,
        staleTime: 30 * 60_000,
        retry: false,
    })

    if (!on) return null

    return formatEstimate(q.data)
}

/** Why estimates are not showing (database missing, offline), for a tooltip. */
export function useEstimateError(host: string | null | undefined): string | null {
    const { app } = useSettings()
    const q = useQuery({
        queryKey: ['geo-estimate', host],
        queryFn: async () => (await geo.estimate([host as string]))[0] ?? null,
        enabled: !!app?.estimateLatency && !!host,
        staleTime: 30 * 60_000,
        retry: false,
    })

    return q.error ? String(q.error.message ?? q.error) : null
}
