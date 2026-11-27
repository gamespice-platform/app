import { describe, expect, it } from 'vitest'

import {
    alertsKey,
    defaultConfig,
    diffNames,
    evaluate,
    newEngine,
    newRule,
    parseConfig,
    pushHistory,
    testEvent,
    type AlertConfig,
    type RuleKind,
    type Snapshot,
} from './monitor-alerts'

const snap = (atSec: number, over: Partial<Snapshot> = {}): Snapshot => ({
    at: atSec * 1000,
    reach: 'online',
    rtt: 40,
    players: 3,
    maxPlayers: 10,
    map: 'dm1',
    names: ['a', 'b', 'c'],
    ...over,
})

function setup(kind: RuleKind, over: Partial<ReturnType<typeof newRule>> = {}): AlertConfig {
    const cfg = defaultConfig()

    cfg.rules.push({ ...newRule(kind, 'r1'), cooldownSecs: 0, seconds: 0, ...over })

    return cfg
}

/** Run snapshots through the engine and collect events per snapshot. */
function run(cfg: AlertConfig, snaps: Snapshot[]) {
    const st = newEngine()

    return snaps.map((s) => evaluate(cfg, st, s))
}

describe('diffNames', () => {
    it('ignores blank names, which are players still connecting', () => {
        expect(diffNames(['a'], ['a', '', '  '])).toEqual([])
        expect(diffNames(['a', ''], ['a'])).toEqual([])
        expect(diffNames(['a', ''], ['a', 'b'])).toEqual(['b'])
    })

    it('finds additions and counts duplicates', () => {
        expect(diffNames(['a'], ['a', 'b', 'b'])).toEqual(['b', 'b'])
        expect(diffNames(['a', 'a'], ['a'])).toEqual([])
    })
})

describe('first snapshot', () => {
    it('only seeds the state', () => {
        const out = run(setup('full'), [snap(0, { players: 10 })])

        expect(out[0]).toEqual([])
    })
})

describe('join and leave', () => {
    it('names who joined and who left', () => {
        const j = run(setup('join'), [snap(0), snap(1, { names: ['a', 'b', 'c', 'd'], players: 4 })])

        expect(j[1]![0]!.body).toContain('d joined')

        const l = run(setup('leave'), [snap(0), snap(1, { names: ['a'], players: 1 })])

        expect(l[1]![0]!.body).toContain('b, c left')
    })

    it('falls back to the count when there is no list', () => {
        const j = run(setup('join'), [
            snap(0, { names: null }),
            snap(1, { names: null, players: 5 }),
        ])

        expect(j[1]![0]!.body).toContain('2 players joined')
    })

    it('stays quiet when nothing changed or the server was down', () => {
        const out = run(setup('join'), [snap(0), snap(1), snap(2, { reach: 'timeout', players: null, names: null }), snap(3)])

        expect(out.flat()).toEqual([])
    })
})

describe('map change', () => {
    it('fires on a different map only', () => {
        const out = run(setup('map'), [snap(0), snap(1), snap(2, { map: 'dm2' }), snap(3, { map: null })])

        expect(out[1]).toEqual([])
        expect(out[2]![0]!.body).toBe('dm1 to dm2.')
        expect(out[3]).toEqual([])
    })
})

describe('online and offline', () => {
    it('announces the server coming back, with the downtime', () => {
        const out = run(setup('online'), [
            snap(0),
            snap(10, { reach: 'timeout', rtt: null }),
            snap(70, { reach: 'timeout', rtt: null }),
            snap(80),
        ])

        expect(out[3]![0]!.body).toContain('after 70s')
        expect(out.slice(0, 3).flat()).toEqual([])
    })

    it('offline waits for its sustain time and fires once', () => {
        const cfg = setup('offline', { seconds: 5 })
        const off = (t: number) => snap(t, { reach: 'offline', rtt: null })
        const out = run(cfg, [snap(0), off(1), off(4), off(6), off(20)])

        expect(out.map((e) => e.length)).toEqual([0, 0, 0, 1, 0])
    })
})

describe('timeout', () => {
    it('needs min seconds of no answers, ignores a single blip', () => {
        const cfg = setup('timeout', { seconds: 10 })
        const to = (t: number) => snap(t, { reach: 'timeout', rtt: null })
        const out = run(cfg, [snap(0), to(1), snap(2), to(3), to(8), to(14), to(30)])

        expect(out.map((e) => e.length)).toEqual([0, 0, 0, 0, 0, 1, 0])
        expect(out[5]![0]!.body).toContain('10s')
    })

    it('re-arms after the server answers again', () => {
        const cfg = setup('timeout', { seconds: 0 })
        const to = (t: number) => snap(t, { reach: 'timeout', rtt: null })
        const out = run(cfg, [snap(0), to(1), to(2), snap(3), to(4)])

        expect(out.map((e) => e.length)).toEqual([0, 1, 0, 0, 1])
    })
})

describe('latency', () => {
    it('fires when the average over the window stays above the limit', () => {
        const cfg = setup('latency', { threshold: 100, seconds: 10 })
        const slow = Array.from({ length: 13 }, (_, i) => snap(i, { rtt: 200 }))
        const out = run(cfg, slow)

        const fired = out.map((e, i) => (e.length ? i : -1)).filter((i) => i >= 0)

        expect(fired).toEqual([9])
        expect(out[9]![0]!.body).toContain('200 ms')
    })

    it('a short spike does not move the average past the limit', () => {
        const cfg = setup('latency', { threshold: 100, seconds: 10 })
        const s = Array.from({ length: 30 }, (_, i) => snap(i, { rtt: i === 15 ? 400 : 40 }))

        expect(run(cfg, s).flat()).toEqual([])
    })

    it('does not fire before the window is covered', () => {
        const cfg = setup('latency', { threshold: 100, seconds: 30 })

        expect(run(cfg, [snap(0, { rtt: 500 }), snap(1, { rtt: 500 })]).flat()).toEqual([])
    })
})

describe('player count levels', () => {
    it('below N fires once per dip and re-arms', () => {
        const cfg = setup('below', { threshold: 3 })
        const p = (t: number, n: number) => snap(t, { players: n, names: null })
        const out = run(cfg, [p(0, 5), p(1, 2), p(2, 1), p(3, 5), p(4, 2)])

        expect(out.map((e) => e.length)).toEqual([0, 1, 0, 0, 1])
    })

    it('above N and its sustain time', () => {
        const cfg = setup('above', { threshold: 5, seconds: 3 })
        const p = (t: number, n: number) => snap(t, { players: n, names: null })
        const out = run(cfg, [p(0, 2), p(1, 8), p(2, 8), p(3, 2), p(4, 8), p(5, 8), p(6, 8), p(7, 8)])

        expect(out.map((e) => e.length)).toEqual([0, 0, 0, 0, 0, 0, 0, 1])
    })

    it('full and empty', () => {
        const full = run(setup('full'), [snap(0), snap(1, { players: 10 }), snap(2, { players: 10 })])

        expect(full.map((e) => e.length)).toEqual([0, 1, 0])

        const empty = run(setup('empty'), [snap(0), snap(1, { players: 0, names: [] })])

        expect(empty[1]).toHaveLength(1)
    })

    it('unknown counts never satisfy a level', () => {
        const out = run(setup('below', { threshold: 99 }), [snap(0), snap(1, { players: null })])

        expect(out.flat()).toEqual([])
    })
})

describe('cooldown and toggles', () => {
    it('a cooldown swallows repeats until it passes; 0 means none', () => {
        const j = (t: number) => snap(t, { names: Array.from({ length: t + 4 }, (_, i) => `p${i}`), players: t + 4 })
        const seq = [snap(0), j(1), j(2), j(40)]
        const cool = run(setup('join', { cooldownSecs: 30 }), seq)

        expect(cool.map((e) => e.length)).toEqual([0, 1, 0, 1])

        const none = run(setup('join', { cooldownSecs: 0 }), seq)

        expect(none.map((e) => e.length)).toEqual([0, 1, 1, 1])
    })

    it('a disabled rule or a disabled server raises nothing', () => {
        const seq = [snap(0), snap(1, { map: 'x' })]

        expect(run(setup('map', { enabled: false }), seq).flat()).toEqual([])

        const off = setup('map')

        off.enabled = false

        expect(run(off, seq).flat()).toEqual([])
    })

    it('a removed rule forgets its state', () => {
        const cfg = setup('below', { threshold: 3 })
        const st = newEngine()

        evaluate(cfg, st, snap(0))
        evaluate(cfg, st, snap(1, { players: 1 }))
        expect(st.rules.has('r1')).toBe(true)

        cfg.rules = []
        evaluate(cfg, st, snap(2))
        expect(st.rules.size).toBe(0)
    })
})

describe('config persistence', () => {
    it('round-trips and keys by lower-cased address', () => {
        const cfg = setup('latency')

        expect(parseConfig(JSON.stringify(cfg))).toEqual(cfg)
        expect(alertsKey(' Play.Example.COM ', 27015)).toBe('gs.monitor.alerts.play.example.com:27015')
    })

    it('survives garbage', () => {
        expect(parseConfig('{nope')).toEqual(defaultConfig())
        expect(parseConfig(null)).toEqual(defaultConfig())

        const cfg = parseConfig({
            rules: [
                { id: 'a', kind: 'bogus' },
                { id: 'b', kind: 'below', threshold: -4, seconds: 'x' },
                { id: 'b', kind: 'above' },
                null,
            ],
        })

        expect(cfg.rules).toHaveLength(1)
        expect(cfg.rules[0]!.threshold).toBe(4)
        expect(cfg.rules[0]!.seconds).toBe(0)
    })
})

describe('history and test events', () => {
    it('trims to the limit, 0 = unlimited', () => {
        const rule = newRule('map', 'r')
        const ev = (t: number) => testEvent(rule, 'srv', t)
        let h = [] as ReturnType<typeof ev>[]

        for (let i = 0; i < 5; i++) h = pushHistory(h, ev(i), 3)
        expect(h.map((e) => e.at)).toEqual([4, 3, 2])

        h = []
        for (let i = 0; i < 500; i++) h = pushHistory(h, ev(i), 0)
        expect(h).toHaveLength(500)
    })

    it('test events are marked', () => {
        const e = testEvent(newRule('join', 'r'), 'srv', 1)

        expect(e.test).toBe(true)
        expect(e.title).toContain('Player joins')
    })
})
