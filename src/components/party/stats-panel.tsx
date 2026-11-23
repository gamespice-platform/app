import { useQuery } from '@tanstack/react-query'

import { partyApi } from '~/lib/api/party'
import { isUnavailable } from '~/lib/api/unavailable'
import { messageOf } from '~/lib/ipc'
import { kdRatio } from '~/lib/party/logic'
import { ErrorLine, Panel, UnavailableNote } from './party-ui'

/** The viewer's party totals. A proposed route: says so plainly until the site has it. */
export default function StatsPanel() {
    const stats = useQuery({
        queryKey: ['party', 'stats'],
        queryFn: () => partyApi.stats(),
        retry: false,
    })

    if (stats.isPending) return null

    return (
        <Panel title="Your party stats">
            {stats.error ? (
                isUnavailable(stats.error) ? (
                    <UnavailableNote what="Party stats" />
                ) : (
                    <div className="p-3">
                        <ErrorLine>{messageOf(stats.error)}</ErrorLine>
                    </div>
                )
            ) : (
                <dl className="grid grid-cols-3 gap-3 p-3 text-xs sm:grid-cols-6">
                    {[
                        ['Parties', stats.data.parties],
                        ['Kills', stats.data.kills],
                        ['Deaths', stats.data.deaths],
                        ['Assists', stats.data.assist],
                        ['Score', stats.data.score],
                        ['K/D', kdRatio(stats.data.kills, stats.data.deaths)],
                    ].map(([label, value]) => (
                        <div key={label}>
                            <dt className="text-muted">{label}</dt>
                            <dd className="text-sm font-semibold">{value}</dd>
                        </div>
                    ))}
                </dl>
            )}
        </Panel>
    )
}
