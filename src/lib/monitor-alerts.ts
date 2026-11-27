/**
 * Pure rule engine behind the monitor window's Notifications tab.
 *
 * Every probe of the server becomes a `Snapshot`; `evaluate` diffs it against
 * the previous one and returns the events the user's rules asked for. Kept free
 * of React, Tauri and the clock (time comes from the snapshot) so it is
 * unit-tested in node. Rules are stored per monitored server in the browser's
 * local storage and never leave the device.
 *
 * Two shapes of rule:
 *   - edge rules (join, leave, map change, online, offline) fire when the diff
 *     shows the change;
 *   - level rules (timeout, high latency, players below / above N, full,
 *     empty) fire once when the condition has held for `seconds`, then stay
 *     quiet until it has cleared (so a server that stays full is one alert,
 *     not one per probe).
 * Both then honour `cooldownSecs`: a rule that fired is silent for that long.
 * Every number is configurable and 0 means "no minimum" / "no cooldown".
 */

export type Reach = 'online' | 'offline' | 'timeout'

/** What one probe told us. */
export type Snapshot = {
    /** Milliseconds since the Unix epoch. */
    at: number
    reach: Reach
    /** Round-trip time of a successful probe. */
    rtt: number | null
    players: number | null
    maxPlayers: number | null
    map: string | null
    /** Player names, when the protocol listed them; `null` when unknown. */
    names: string[] | null
}

export type RuleKind =
    | 'join'
    | 'leave'
    | 'map'
    | 'online'
    | 'offline'
    | 'timeout'
    | 'latency'
    | 'below'
    | 'above'
    | 'full'
    | 'empty'

export type RuleSpec = {
    kind: RuleKind
    label: string
    help: string
    /** What `threshold` means for this kind, or null when it has none. */
    thresholdLabel: string | null
    defaultThreshold: number
    /** Whether `seconds` (sustain time) applies. */
    sustained: boolean
    defaultSeconds: number
    defaultCooldownSecs: number
}

export const RULE_SPECS: readonly RuleSpec[] = [
    { kind: 'join', label: 'Player joins', help: 'Someone connects to the server.', thresholdLabel: null, defaultThreshold: 0, sustained: false, defaultSeconds: 0, defaultCooldownSecs: 10 },
    { kind: 'leave', label: 'Player leaves', help: 'Someone disconnects from the server.', thresholdLabel: null, defaultThreshold: 0, sustained: false, defaultSeconds: 0, defaultCooldownSecs: 10 },
    { kind: 'map', label: 'Map changes', help: 'The server loads a different map.', thresholdLabel: null, defaultThreshold: 0, sustained: false, defaultSeconds: 0, defaultCooldownSecs: 5 },
    { kind: 'online', label: 'Server comes online', help: 'The server answers again after being offline or timing out.', thresholdLabel: null, defaultThreshold: 0, sustained: false, defaultSeconds: 0, defaultCooldownSecs: 60 },
    { kind: 'offline', label: 'Server goes offline', help: 'The server reports itself offline.', thresholdLabel: null, defaultThreshold: 0, sustained: true, defaultSeconds: 5, defaultCooldownSecs: 60 },
    { kind: 'timeout', label: 'Server timeout', help: 'The server has not answered probes for at least this many seconds.', thresholdLabel: null, defaultThreshold: 0, sustained: true, defaultSeconds: 10, defaultCooldownSecs: 120 },
    { kind: 'latency', label: 'High average latency', help: 'Average latency stays above the threshold for at least this many seconds.', thresholdLabel: 'ms', defaultThreshold: 150, sustained: true, defaultSeconds: 30, defaultCooldownSecs: 300 },
    { kind: 'below', label: 'Players drop below', help: 'The player count falls below N.', thresholdLabel: 'players', defaultThreshold: 4, sustained: true, defaultSeconds: 0, defaultCooldownSecs: 300 },
    { kind: 'above', label: 'Players rise above', help: 'The player count rises above N.', thresholdLabel: 'players', defaultThreshold: 10, sustained: true, defaultSeconds: 0, defaultCooldownSecs: 300 },
    { kind: 'full', label: 'Server is full', help: 'Every slot is taken.', thresholdLabel: null, defaultThreshold: 0, sustained: true, defaultSeconds: 0, defaultCooldownSecs: 300 },
    { kind: 'empty', label: 'Server is empty', help: 'Nobody is playing.', thresholdLabel: null, defaultThreshold: 0, sustained: true, defaultSeconds: 30, defaultCooldownSecs: 300 },
]

export function specOf(kind: RuleKind): RuleSpec {
    return RULE_SPECS.find((s) => s.kind === kind) ?? RULE_SPECS[0]!
}

export type Rule = {
    id: string
    kind: RuleKind
    enabled: boolean
    /** N for below / above, milliseconds for latency; ignored otherwise. */
    threshold: number
    /** How long a level condition must hold. 0 = fire on the first probe. */
    seconds: number
    /** Silence after firing. 0 = no cooldown. */
    cooldownSecs: number
}

export type AlertConfig = {
    /** Master switch for this server. */
    enabled: boolean
    /** Also raise an OS notification card (desktop app only). */
    desktop: boolean
    /** Events kept in the notification centre. 0 = unlimited. */
    historyLimit: number
    rules: Rule[]
}

export type AlertEvent = {
    ruleId: string
    kind: RuleKind
    at: number
    title: string
    body: string
    /** True for the "Test" button. */
    test?: boolean
}

export const DEFAULT_HISTORY_LIMIT = 100

export function newRule(kind: RuleKind, id: string): Rule {
    const spec = specOf(kind)

    return {
        id,
        kind,
        enabled: true,
        threshold: spec.defaultThreshold,
        seconds: spec.defaultSeconds,
        cooldownSecs: spec.defaultCooldownSecs,
    }
}

export function defaultConfig(): AlertConfig {
    return { enabled: true, desktop: true, historyLimit: DEFAULT_HISTORY_LIMIT, rules: [] }
}

// ------------------------------------------------------------ Persistence

export const ALERTS_KEY_PREFIX = 'gs.monitor.alerts.'

/** The storage key for one monitored server; host is case-insensitive. */
export function alertsKey(host: string, port: number): string {
    return `${ALERTS_KEY_PREFIX}${host.trim().toLowerCase()}:${port}`
}

const KINDS = new Set<string>(RULE_SPECS.map((s) => s.kind))

function num(v: unknown, fallback: number): number {
    return typeof v === 'number' && Number.isFinite(v) && v >= 0 ? v : fallback
}

/** Read a stored config defensively: anything malformed is dropped, not thrown. */
export function parseConfig(raw: unknown): AlertConfig {
    const out = defaultConfig()

    let data: unknown = raw

    if (typeof raw === 'string') {
        try {
            data = JSON.parse(raw)
        } catch {
            return out
        }
    }

    if (!data || typeof data !== 'object') return out

    const o = data as Record<string, unknown>

    if (typeof o.enabled === 'boolean') out.enabled = o.enabled
    if (typeof o.desktop === 'boolean') out.desktop = o.desktop

    out.historyLimit = Math.floor(num(o.historyLimit, DEFAULT_HISTORY_LIMIT))

    if (Array.isArray(o.rules)) {
        const seen = new Set<string>()

        for (const r of o.rules) {
            if (!r || typeof r !== 'object') continue

            const x = r as Record<string, unknown>

            if (typeof x.kind !== 'string' || !KINDS.has(x.kind)) continue
            if (typeof x.id !== 'string' || !x.id || seen.has(x.id)) continue

            const spec = specOf(x.kind as RuleKind)

            seen.add(x.id)
            out.rules.push({
                id: x.id,
                kind: spec.kind,
                enabled: x.enabled !== false,
                threshold: num(x.threshold, spec.defaultThreshold),
                seconds: num(x.seconds, spec.defaultSeconds),
                cooldownSecs: num(x.cooldownSecs, spec.defaultCooldownSecs),
            })
        }
    }

    return out
}

// ----------------------------------------------------------------- Engine

type RuleState = {
    /** When the level condition began to hold, or null while it does not. */
    since: number | null
    /** The current stretch of the condition already produced its alert. */
    latched: boolean
    lastFiredAt: number | null
}

/** Mutable memory of an engine run: the last snapshot and per-rule timers. */
export type EngineState = {
    prev: Snapshot | null
    rules: Map<string, RuleState>
    /** Recent rtts, for the rolling average. */
    rtts: { at: number; rtt: number }[]
    /** When the server stopped being reachable, for "came online" context. */
    downSince: number | null
}

export function newEngine(): EngineState {
    return { prev: null, rules: new Map(), rtts: [], downSince: null }
}

const fmtSecs = (s: number) => (s >= 90 ? `${Math.round(s / 60)} min` : `${Math.round(s)}s`)

function names(list: string[], limit = 5): string {
    const shown = list.slice(0, limit).join(', ')

    return list.length > limit ? `${shown} and ${list.length - limit} more` : shown
}

/**
 * Players that are in `b` and not in `a`, counting duplicates. A blank name is
 * a slot still connecting (Source lists it before the player has a name); it
 * is neither a join nor a leave, so it is ignored. The named entry that
 * replaces it a moment later is the join.
 */
export function diffNames(a: string[], b: string[]): string[] {
    const left = new Map<string, number>()

    for (const n of a) if (n.trim() !== '') left.set(n, (left.get(n) ?? 0) + 1)

    const out: string[] = []

    for (const n of b) {
        if (n.trim() === '') continue

        const c = left.get(n) ?? 0

        if (c > 0) left.set(n, c - 1)
        else out.push(n)
    }

    return out
}

/** Mean of the rtts within the trailing window, and how long they span. */
function rollingAverage(
    rtts: { at: number; rtt: number }[],
    now: number,
    windowMs: number
): { mean: number; spanMs: number } | null {
    const inWindow = rtts.filter((r) => r.at >= now - windowMs)

    if (inWindow.length === 0) return null

    const sum = inWindow.reduce((a, r) => a + r.rtt, 0)

    return { mean: sum / inWindow.length, spanMs: now - inWindow[0]!.at }
}

/**
 * Feed one snapshot through the rules; returns the events to raise.
 *
 * The first snapshot only seeds `prev`: nothing "changed" yet, so a monitor
 * opened on a server that is already full does not announce it.
 */
export function evaluate(
    config: AlertConfig,
    state: EngineState,
    snap: Snapshot
): AlertEvent[] {
    const prev = state.prev

    state.prev = snap

    if (snap.reach === 'online' && snap.rtt !== null)
        state.rtts.push({ at: snap.at, rtt: snap.rtt })

    // Keep only as much rtt history as the longest latency rule needs.
    const longest = Math.max(
        0,
        ...config.rules.filter((r) => r.kind === 'latency').map((r) => r.seconds)
    )
    const keepFrom = snap.at - Math.max(longest, 1) * 1000 - 5000

    while (state.rtts.length > 0 && state.rtts[0]!.at < keepFrom) state.rtts.shift()

    if (snap.reach !== 'online') state.downSince ??= snap.at

    const wasDown = state.downSince
    const events: AlertEvent[] = []

    if (snap.reach === 'online') state.downSince = null

    // Rule memory for rules that no longer exist is dropped.
    const ids = new Set(config.rules.map((r) => r.id))

    for (const id of state.rules.keys()) if (!ids.has(id)) state.rules.delete(id)

    if (!prev || !config.enabled) return events

    for (const rule of config.rules) {
        if (!rule.enabled) {
            state.rules.delete(rule.id)
            continue
        }

        const st = state.rules.get(rule.id) ?? { since: null, latched: false, lastFiredAt: null }

        state.rules.set(rule.id, st)

        const hit = detect(rule, prev, snap, st, state, wasDown)

        if (!hit) continue

        const cooling =
            rule.cooldownSecs > 0 &&
            st.lastFiredAt !== null &&
            snap.at - st.lastFiredAt < rule.cooldownSecs * 1000

        if (cooling) continue

        st.lastFiredAt = snap.at
        events.push({ ruleId: rule.id, kind: rule.kind, at: snap.at, ...hit })
    }

    return events
}

type Hit = { title: string; body: string }

/** Track a level condition; returns true exactly once per stretch it holds. */
function level(rule: Rule, st: RuleState, holds: boolean, now: number): boolean {
    if (!holds) {
        st.since = null
        st.latched = false

        return false
    }

    st.since ??= now

    if (st.latched) return false
    if (now - st.since < rule.seconds * 1000) return false

    st.latched = true

    return true
}

function detect(
    rule: Rule,
    prev: Snapshot,
    snap: Snapshot,
    st: RuleState,
    state: EngineState,
    wasDown: number | null
): Hit | null {
    const online = snap.reach === 'online'

    switch (rule.kind) {
        case 'join': {
            if (!online || prev.reach !== 'online') return null

            if (snap.names && prev.names) {
                const joined = diffNames(prev.names, snap.names)

                if (joined.length === 0) return null

                return { title: 'Player joined', body: `${names(joined)} joined.` }
            }

            if (snap.players !== null && prev.players !== null && snap.players > prev.players) {
                const n = snap.players - prev.players

                return { title: 'Player joined', body: `${n} ${n === 1 ? 'player' : 'players'} joined (${snap.players} now).` }
            }

            return null
        }

        case 'leave': {
            if (!online || prev.reach !== 'online') return null

            if (snap.names && prev.names) {
                const left = diffNames(snap.names, prev.names)

                if (left.length === 0) return null

                return { title: 'Player left', body: `${names(left)} left.` }
            }

            if (snap.players !== null && prev.players !== null && snap.players < prev.players) {
                const n = prev.players - snap.players

                return { title: 'Player left', body: `${n} ${n === 1 ? 'player' : 'players'} left (${snap.players} now).` }
            }

            return null
        }

        case 'map': {
            if (!online || !snap.map || !prev.map || snap.map === prev.map) return null

            return { title: 'Map changed', body: `${prev.map} to ${snap.map}.` }
        }

        case 'online': {
            if (!online || prev.reach === 'online') return null

            const down = wasDown !== null ? ` after ${fmtSecs((snap.at - wasDown) / 1000)}` : ''

            return { title: 'Server is online', body: `The server is answering again${down}.` }
        }

        case 'offline':
        case 'timeout': {
            const want: Reach = rule.kind
            const holds = snap.reach === want
            const fired = level(rule, st, holds, snap.at)

            if (!fired) return null

            return rule.kind === 'offline'
                ? { title: 'Server is offline', body: 'The server reports itself offline.' }
                : {
                      title: 'Server timed out',
                      body:
                          rule.seconds > 0
                              ? `No answer for ${fmtSecs(rule.seconds)}.`
                              : 'The server did not answer.',
                  }
        }

        case 'latency': {
            // A probe that timed out has no rtt; the average is over answers.
            const avg = online ? rollingAverage(state.rtts, snap.at, rule.seconds * 1000) : null
            const covered = avg !== null && avg.spanMs >= rule.seconds * 1000 * 0.9
            const holds = covered && avg.mean > rule.threshold

            // The window already is the sustain time; `level` only latches.
            if (!level({ ...rule, seconds: 0 }, st, holds, snap.at)) return null

            return {
                title: 'High latency',
                body: `Average ${Math.round(avg!.mean)} ms over ${fmtSecs(rule.seconds)} (limit ${rule.threshold} ms).`,
            }
        }

        case 'below':
        case 'above':
        case 'full':
        case 'empty': {
            const p = snap.players
            const holds =
                online &&
                p !== null &&
                (rule.kind === 'below'
                    ? p < rule.threshold
                    : rule.kind === 'above'
                      ? p > rule.threshold
                      : rule.kind === 'full'
                        ? snap.maxPlayers !== null && snap.maxPlayers > 0 && p >= snap.maxPlayers
                        : p === 0)

            if (!level(rule, st, holds, snap.at)) return null

            const held = rule.seconds > 0 ? ` for ${fmtSecs(rule.seconds)}` : ''

            switch (rule.kind) {
                case 'below':
                    return { title: 'Few players', body: `${p} players, below ${rule.threshold}${held}.` }
                case 'above':
                    return { title: 'Busy server', body: `${p} players, above ${rule.threshold}${held}.` }
                case 'full':
                    return { title: 'Server is full', body: `${p} / ${snap.maxPlayers} players${held}.` }
                default:
                    return { title: 'Server is empty', body: `Nobody is playing${held}.` }
            }
        }
    }
}

/** A sample event for the "Test" button; uses the rule's own wording. */
export function testEvent(rule: Rule, serverName: string, at: number): AlertEvent {
    const spec = specOf(rule.kind)

    return {
        ruleId: rule.id,
        kind: rule.kind,
        at,
        title: `Test: ${spec.label}`,
        body: `This is how "${spec.label}" will look for ${serverName}.`,
        test: true,
    }
}

/** Append, trimming to `limit` (0 = unlimited); newest first. */
export function pushHistory(history: AlertEvent[], event: AlertEvent, limit: number): AlertEvent[] {
    const next = [event, ...history]

    return limit > 0 && next.length > limit ? next.slice(0, limit) : next
}
