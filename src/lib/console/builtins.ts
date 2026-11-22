/**
 * The Console's own commands, the few that are not `tmc` arguments.
 *
 * Everything else on the line is handed to Rust untouched: splitting, quoting
 * and the optional leading `tmc` are done there (`tmc_core::console`), once,
 * so a line behaves the same here as `tmc ...` in a shell. This file only
 * decides whether the line is for the Console itself.
 */

export type Builtin =
    | { kind: 'clear' }
    | { kind: 'help' }
    | { kind: 'run'; line: string }
    | { kind: 'empty' }

/**
 * Classify a typed line. A leading `tmc` is optional and ignored for the
 * decision, so `tmc clear` and `clear` agree. `help` alone is the Console's
 * help; `help <topic>` becomes `<topic> --help`, which is what the CLI answers
 * with.
 *
 * Only whole-word matches count, and only when nothing else is on the line
 * (`clear` with arguments is a `tmc` command and is passed through).
 */
export function classify(raw: string): Builtin {
    const line = raw.trim()

    if (!line) return { kind: 'empty' }

    const m = /^(?:tmc\s+)?(.*)$/i.exec(line)
    const rest = (m?.[1] ?? line).trim()

    if (/^(clear|cls)$/i.test(rest)) return { kind: 'clear' }

    if (/^(help|\?)$/i.test(rest)) return { kind: 'help' }

    const topic = /^help\s+(.+)$/i.exec(rest)

    if (topic?.[1]) return { kind: 'run', line: `${topic[1]} --help` }

    return { kind: 'run', line }
}

/** What `help` prints. Plain text lines; the screen draws them as notes. */
export const HELP_LINES: readonly string[] = [
    'This is the TMC command-line tool. A command here behaves exactly like `tmc ...` in a terminal.',
    'The leading "tmc" is optional: `mod list --mine` and `tmc mod list --mine` are the same.',
    '',
    '  --help, help <command>   what a command takes (the tool prints its own list)',
    '  clear                    empty this screen (Ctrl+L)',
    '  Up / Down                walk through earlier commands',
    '  Ctrl+C                   stop the running command',
    '  Ctrl+D                   end the input of the running command',
    '  Tab                      does not complete; paste a path if you need one',
    '',
    'While a command runs, what you type is sent to it (a prompt, a confirmation).',
    'When you are signed in to the app, the tool is signed in as you for the account commands;',
    'other commands use your profiles in ~/.config/tmc/config.json and the TMC_* variables, as in a shell.',
]
