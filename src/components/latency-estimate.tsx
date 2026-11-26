import { useEstimateError, useLatencyEstimate } from '~/lib/geo/use-estimate'

/**
 * `~45 ms est.` under or beside a measured ping. Renders nothing while the
 * setting is off. Muted and italic so it never reads as a measurement.
 */
export default function LatencyEstimate({
    host,
    className = '',
}: {
    host: string | null | undefined
    className?: string
}) {
    const text = useLatencyEstimate(host)
    const error = useEstimateError(host)

    if (error)
        return (
            <span className={`text-xs text-muted ${className}`} title={error}>
                est. unavailable
            </span>
        )

    if (!text) return null

    return (
        <span
            className={`text-xs italic tabular-nums text-muted ${className}`}
            title="Estimated from the distance between you and the server, not measured."
        >
            {text}
        </span>
    )
}
