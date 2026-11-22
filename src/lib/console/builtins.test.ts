import { describe, expect, it } from 'vitest'

import { classify, HELP_LINES } from './builtins'

describe('classify', () => {
    it('ignores blank input', () => {
        expect(classify('   ')).toEqual({ kind: 'empty' })
    })

    it('treats clear and cls as the console, with or without the tmc prefix', () => {
        for (const l of ['clear', 'cls', ' CLEAR ', 'tmc clear'])
            expect(classify(l)).toEqual({ kind: 'clear' })
    })

    it('answers help itself, and turns help <x> into <x> --help', () => {
        expect(classify('help')).toEqual({ kind: 'help' })
        expect(classify('tmc ?')).toEqual({ kind: 'help' })
        expect(classify('help mod list')).toEqual({
            kind: 'run',
            line: 'mod list --help',
        })
    })

    it('passes everything else through untouched, prefix included', () => {
        expect(classify('tmc mod list --mine')).toEqual({
            kind: 'run',
            line: 'tmc mod list --mine',
        })
        expect(classify('clear --all')).toEqual({
            kind: 'run',
            line: 'clear --all',
        })
        expect(classify('mod create --name "A B"')).toEqual({
            kind: 'run',
            line: 'mod create --name "A B"',
        })
    })

    it('has help text', () => {
        expect(HELP_LINES.length).toBeGreaterThan(3)
    })
})
