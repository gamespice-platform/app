import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { FiBell, FiPlus, FiSend, FiTrash2, FiX } from 'react-icons/fi'

import { ipc } from '~/lib/ipc/commands'
import {
    RULE_SPECS,
    alertsKey,
    evaluate,
    newEngine,
    newRule,
    parseConfig,
    pushHistory,
    specOf,
    testEvent,
    type AlertConfig,
    type AlertEvent,
    type EngineState,
    type Rule,
    type RuleKind,
    type Snapshot,
} from '~/lib/monitor-alerts'

const TOAST_MS = 8000

function loadConfig(key: string): AlertConfig {
    try {
        return parseConfig(window.localStorage.getItem(key))
    } catch {
        return parseConfig(null)
    }
}

function saveConfig(key: string, config: AlertConfig) {
    try {
        window.localStorage.setItem(key, JSON.stringify(config))
    } catch {
        // Quota or private mode: the rules just do not persist.
    }
}

const inTauri = () => '__TAURI_INTERNALS__' in window

let serial = 0
const makeId = () => `r${Date.now().toString(36)}${(serial++).toString(36)}`

/**
 * The monitor window's alert rules for one server: the saved config, the
 * engine fed by the probe loop, the toasts on screen and the notification
 * centre history. Runs for as long as the window is open, whichever tab shows.
 */
export function useMonitorAlerts(host: string | null, port: number | null, serverName: string) {
    const key = host !== null && port !== null ? alertsKey(host, port) : null
    const [config, setConfigState] = useState<AlertConfig>(() =>
        key ? loadConfig(key) : parseConfig(null)
    )
    const [history, setHistory] = useState<AlertEvent[]>([])
    const [toasts, setToasts] = useState<AlertEvent[]>([])
    const [unread, setUnread] = useState(0)

    const engine = useRef<EngineState>(newEngine())
    const configRef = useRef(config)
    const keyRef = useRef(key)
    const nameRef = useRef(serverName)

    useEffect(() => {
        configRef.current = config
        nameRef.current = serverName
    })

    // A different server: load its rules and start the engine over.
    useEffect(() => {
        if (keyRef.current === key) return

        keyRef.current = key
        engine.current = newEngine()
        setConfigState(key ? loadConfig(key) : parseConfig(null))
        setHistory([])
        setToasts([])
        setUnread(0)
    }, [key])

    const setConfig = useCallback((next: AlertConfig) => {
        setConfigState(next)

        if (keyRef.current) saveConfig(keyRef.current, next)
    }, [])

    const raise = useCallback((events: AlertEvent[]) => {
        if (events.length === 0) return

        const cfg = configRef.current

        setHistory((h) => events.reduce((acc, e) => pushHistory(acc, e, cfg.historyLimit), h))
        setToasts((t) => [...events, ...t])
        setUnread((n) => n + events.length)

        for (const e of events) {
            window.setTimeout(
                () => setToasts((t) => t.filter((x) => x !== e)),
                TOAST_MS
            )

            if (cfg.desktop && inTauri())
                void ipc
                    .notificationShow(`${nameRef.current}: ${e.title}`, e.body)
                    .catch(() => undefined)
        }
    }, [])

    /** Call with every probe's snapshot. */
    const feed = useCallback(
        (snap: Snapshot) => raise(evaluate(configRef.current, engine.current, snap)),
        [raise]
    )

    const test = useCallback(
        (rule: Rule) => raise([testEvent(rule, nameRef.current, Date.now())]),
        [raise]
    )

    return {
        config,
        setConfig,
        history,
        toasts,
        unread,
        feed,
        test,
        dismiss: (e: AlertEvent) => setToasts((t) => t.filter((x) => x !== e)),
        clearUnread: () => setUnread(0),
        clearHistory: () => setHistory([]),
    }
}

export type MonitorAlerts = ReturnType<typeof useMonitorAlerts>

// ------------------------------------------------------------------ Toasts

/** In-app toasts, top right of the monitor window. Text only. */
export function AlertToasts({ alerts }: { alerts: MonitorAlerts }) {
    if (alerts.toasts.length === 0) return null

    return (
        <div
            role="status"
            aria-live="polite"
            className="pointer-events-none fixed right-3 top-3 z-50 flex w-72 flex-col gap-2"
        >
            {alerts.toasts.slice(0, 5).map((e, i) => (
                <div
                    key={`${e.ruleId}-${e.at}-${i}`}
                    className="pointer-events-auto flex items-start gap-2 rounded-lg border border-border bg-surface p-3 shadow-lg"
                >
                    <FiBell className="mt-0.5 size-4 shrink-0 text-accent" />
                    <div className="min-w-0 flex-1">
                        <p className="truncate text-xs font-semibold">{e.title}</p>
                        <p className="selectable break-words text-xs text-muted">{e.body}</p>
                    </div>
                    <button
                        type="button"
                        aria-label="Dismiss"
                        onClick={() => alerts.dismiss(e)}
                        className="text-muted hover:text-accent focus-visible:outline focus-visible:outline-2 focus-visible:outline-accent"
                    >
                        <FiX className="size-3.5" />
                    </button>
                </div>
            ))}
        </div>
    )
}

// ------------------------------------------------------------------- Panel

const field =
    'w-20 rounded border border-border bg-background px-2 py-1 text-xs focus-visible:outline focus-visible:outline-2 focus-visible:outline-accent'

function NumberField({
    label,
    value,
    onChange,
    unit,
}: {
    label: string
    value: number
    onChange: (n: number) => void
    unit: string
}) {
    return (
        <label className="flex items-center gap-1.5 text-xs text-muted">
            {label}
            <input
                type="number"
                min={0}
                value={value}
                onChange={(e) => {
                    const n = Number(e.target.value)

                    if (Number.isFinite(n) && n >= 0) onChange(n)
                }}
                className={field}
            />
            {unit}
        </label>
    )
}

function RuleRow({
    rule,
    onChange,
    onRemove,
    onTest,
}: {
    rule: Rule
    onChange: (r: Rule) => void
    onRemove: () => void
    onTest: () => void
}) {
    const spec = specOf(rule.kind)

    return (
        <li className="rounded-lg border border-border bg-surface p-3">
            <div className="flex flex-wrap items-center gap-x-3 gap-y-2">
                <label className="flex items-center gap-2 text-sm font-medium">
                    <input
                        type="checkbox"
                        checked={rule.enabled}
                        onChange={(e) => onChange({ ...rule, enabled: e.target.checked })}
                        aria-label={`Enable ${spec.label}`}
                    />
                    {spec.label}
                </label>

                <div className="ml-auto flex items-center gap-2">
                    <button
                        type="button"
                        onClick={onTest}
                        title="Raise a sample alert for this rule"
                        className="inline-flex items-center gap-1.5 rounded-lg border border-border bg-surface px-2.5 py-1 text-xs font-medium hover:border-accent hover:text-accent focus-visible:outline focus-visible:outline-2 focus-visible:outline-accent"
                    >
                        <FiSend className="size-3" /> Test
                    </button>
                    <button
                        type="button"
                        onClick={onRemove}
                        aria-label={`Remove ${spec.label}`}
                        title="Remove this rule"
                        className="rounded p-1 text-muted hover:text-danger focus-visible:outline focus-visible:outline-2 focus-visible:outline-accent"
                    >
                        <FiTrash2 className="size-3.5" />
                    </button>
                </div>
            </div>

            <p className="mt-1 text-[11px] text-muted">{spec.help}</p>

            <div className="mt-2 flex flex-wrap items-center gap-x-4 gap-y-2">
                {spec.thresholdLabel && (
                    <NumberField
                        label="Threshold"
                        unit={spec.thresholdLabel}
                        value={rule.threshold}
                        onChange={(threshold) => onChange({ ...rule, threshold })}
                    />
                )}
                {spec.sustained && (
                    <NumberField
                        label="For at least"
                        unit="s"
                        value={rule.seconds}
                        onChange={(seconds) => onChange({ ...rule, seconds })}
                    />
                )}
                <NumberField
                    label="Cooldown"
                    unit="s (0 = none)"
                    value={rule.cooldownSecs}
                    onChange={(cooldownSecs) => onChange({ ...rule, cooldownSecs })}
                />
            </div>
        </li>
    )
}

const when = (at: number) =>
    new Date(at).toLocaleTimeString(undefined, {
        hour: '2-digit',
        minute: '2-digit',
        second: '2-digit',
    })

export function AlertsPanel({ alerts, address }: { alerts: MonitorAlerts; address: string }) {
    const { config, setConfig } = alerts
    const [adding, setAdding] = useState<RuleKind>('join')

    const kinds = useMemo(() => RULE_SPECS, [])

    // Opening the tab reads everything that arrived so far.
    const { clearUnread } = alerts

    useEffect(() => {
        clearUnread()
    }, [clearUnread, alerts.history.length])

    const update = (id: string, next: Rule) =>
        setConfig({ ...config, rules: config.rules.map((r) => (r.id === id ? next : r)) })

    return (
        <section
            aria-label="Notifications"
            className="flex min-h-0 min-w-0 flex-1 flex-col gap-3 overflow-y-auto rounded-xl border border-border bg-surface-2 p-4 lg:flex-row"
        >
            <div className="min-w-0 flex-1 space-y-3">
                <div className="flex flex-wrap items-center gap-x-5 gap-y-2">
                    <label className="flex items-center gap-2 text-sm font-semibold">
                        <input
                            type="checkbox"
                            checked={config.enabled}
                            onChange={(e) => setConfig({ ...config, enabled: e.target.checked })}
                        />
                        Alerts for {address}
                    </label>
                    <label className="flex items-center gap-2 text-xs text-muted">
                        <input
                            type="checkbox"
                            checked={config.desktop}
                            onChange={(e) => setConfig({ ...config, desktop: e.target.checked })}
                        />
                        Also show a desktop notification
                    </label>
                </div>

                <p className="text-[11px] text-muted">
                    Rules are saved on this device for this server only and checked on every
                    probe while this window is open.
                </p>

                {config.rules.length === 0 && (
                    <p className="rounded-lg border border-dashed border-border p-4 text-xs text-muted">
                        No rules yet. Add one below, for example &ldquo;Player joins&rdquo;.
                    </p>
                )}

                <ul className="space-y-2">
                    {config.rules.map((rule) => (
                        <RuleRow
                            key={rule.id}
                            rule={rule}
                            onChange={(next) => update(rule.id, next)}
                            onRemove={() =>
                                setConfig({
                                    ...config,
                                    rules: config.rules.filter((r) => r.id !== rule.id),
                                })
                            }
                            onTest={() => alerts.test(rule)}
                        />
                    ))}
                </ul>

                <div className="flex flex-wrap items-center gap-2">
                    <select
                        aria-label="Event"
                        value={adding}
                        onChange={(e) => setAdding(e.target.value as RuleKind)}
                        className="rounded border border-border bg-background px-2 py-1.5 text-xs"
                    >
                        {kinds.map((s) => (
                            <option key={s.kind} value={s.kind}>
                                {s.label}
                            </option>
                        ))}
                    </select>
                    <button
                        type="button"
                        onClick={() =>
                            setConfig({
                                ...config,
                                rules: [...config.rules, newRule(adding, makeId())],
                            })
                        }
                        className="inline-flex items-center gap-1.5 rounded-lg border border-border bg-surface px-3 py-1.5 text-xs font-medium hover:border-accent hover:text-accent focus-visible:outline focus-visible:outline-2 focus-visible:outline-accent"
                    >
                        <FiPlus className="size-3.5" /> Add rule
                    </button>
                </div>
            </div>

            <aside aria-label="Recent alerts" className="min-w-0 lg:w-80">
                <div className="mb-2 flex items-center justify-between">
                    <h2 className="text-sm font-semibold">Recent alerts</h2>
                    {alerts.history.length > 0 && (
                        <button
                            type="button"
                            onClick={alerts.clearHistory}
                            className="text-xs text-muted hover:text-accent"
                        >
                            Clear
                        </button>
                    )}
                </div>

                {alerts.history.length === 0 ? (
                    <p className="text-xs text-muted">Nothing yet.</p>
                ) : (
                    <ol className="space-y-1.5">
                        {alerts.history.map((e, i) => (
                            <li
                                key={`${e.ruleId}-${e.at}-${i}`}
                                className="rounded-lg border border-border bg-surface px-3 py-2"
                            >
                                <p className="flex justify-between gap-2 text-xs font-semibold">
                                    <span className="truncate">{e.title}</span>
                                    <span className="shrink-0 font-normal text-muted">{when(e.at)}</span>
                                </p>
                                <p className="selectable break-words text-xs text-muted">{e.body}</p>
                            </li>
                        ))}
                    </ol>
                )}
            </aside>
        </section>
    )
}
