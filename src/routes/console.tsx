import {
    memo,
    useCallback,
    useEffect,
    useLayoutEffect,
    useRef,
    useState,
    useSyncExternalStore,
} from 'react'
import { FiCopy, FiSlash, FiSquare, FiTerminal, FiTrash2 } from 'react-icons/fi'

import { ipc } from '~/lib/ipc/commands'
import type { ConsoleProbeT } from '~/lib/ipc/schemas'
import { classify, HELP_LINES } from '~/lib/console/builtins'
import {
    loadHistory,
    pushHistory,
    saveHistory,
    walkHistory,
} from '~/lib/console/history'
import { consoleStore } from '~/lib/console/store'
import { plainText, type Line, type Tone } from '~/lib/console/transcript'

/**
 * **Console** - the TMC command-line tool, inside the app.
 *
 * What runs is the installed `tmc` itself (Rust finds it and spawns it with no
 * shell; see `tmc_core::console`), so a command typed here behaves exactly like
 * `tmc ...` in a terminal: same commands, same `~/.config/tmc` profiles, same
 * `TMC_*` variables. When the app is signed in, the tool is signed in as the
 * same user for the account commands without asking for a key.
 *
 * There is no process to spawn on Android and iOS, so the probe fails there and
 * the screen says so instead of offering a prompt that cannot work.
 */

type Probe =
    | { state: 'loading' }
    | { state: 'unavailable' }
    | { state: 'missing' }
    | { state: 'ready'; info: ConsoleProbeT }

const TONE: Record<NonNullable<Tone>, string> = {
    danger: 'text-danger',
    success: 'text-success',
    warning: 'text-warning',
    accent: 'text-accent',
    muted: 'text-muted',
}

/** A row only redraws when its `rev` moves. */
const Row = memo(
    function Row({ line }: { line: Line; rev: number }) {
        // Errors (stderr) are red whatever the tool coloured; a command echo is
        // the accent; the Console's own notes are muted.
        const base =
            line.kind === 'stderr'
                ? 'text-danger'
                : line.kind === 'cmd'
                  ? 'text-accent'
                  : line.kind === 'note' || line.kind === 'input'
                    ? 'text-muted'
                    : ''

        return (
            <div className={`min-h-[1.25em] whitespace-pre-wrap break-words ${base}`}>
                {line.kind === 'cmd' && <span className="select-none">$ tmc </span>}
                {line.spans.map((s, i) => (
                    <span
                        key={i}
                        className={[
                            s.style.tone && line.kind !== 'stderr'
                                ? TONE[s.style.tone]
                                : '',
                            s.style.bold ? 'font-bold' : '',
                            s.style.dim ? 'opacity-70' : '',
                            s.style.italic ? 'italic' : '',
                            s.style.underline ? 'underline' : '',
                        ].join(' ')}
                    >
                        {s.text}
                    </span>
                ))}
            </div>
        )
    },
    (a, b) => a.line.id === b.line.id && a.rev === b.rev
)

export default function ConsoleRoute() {
    const [probe, setProbe] = useState<Probe>({ state: 'loading' })

    useEffect(() => {
        let live = true

        ipc.consoleProbe()
            .then((info) => {
                if (live) setProbe(info.found ? { state: 'ready', info } : { state: 'missing' })
            })
            // The command is not compiled into mobile builds.
            .catch(() => live && setProbe({ state: 'unavailable' }))

        return () => {
            live = false
        }
    }, [])

    if (probe.state === 'loading') return null

    if (probe.state === 'unavailable') {
        return (
            <Notice title="The console is not available on this device">
                The command-line tool is a program, and phones and tablets cannot
                run one from an app. Use the Console in the desktop app, or run{' '}
                <code className="selectable">tmc</code> from a computer.
            </Notice>
        )
    }

    if (probe.state === 'missing') {
        return (
            <Notice title="The TMC command-line tool is not installed">
                The console runs the <code className="selectable">tmc</code> program
                installed on this computer; it is not part of the app. Install it
                with Python 3.11 or newer (
                <code className="selectable">pip install tmc-cli</code>), make sure{' '}
                <code className="selectable">tmc --version</code> works in a
                terminal, then reopen this screen.
            </Notice>
        )
    }

    return <Terminal info={probe.info} />
}

function Notice({ title, children }: { title: string; children: React.ReactNode }) {
    return (
        <div className="flex flex-col gap-2 p-4">
            <h1 className="text-lg font-bold">Console</h1>
            <div className="rounded-xl border border-dashed border-border p-8 text-center">
                <FiSlash aria-hidden className="mx-auto size-6 text-muted" />
                <p className="mt-2 text-sm">{title}</p>
                <p className="mx-auto mt-1 max-w-md text-xs text-muted">{children}</p>
            </div>
        </div>
    )
}

function Terminal({ info }: { info: ConsoleProbeT }) {
    useSyncExternalStore(consoleStore.subscribe, consoleStore.getVersion)

    const { transcript, running } = consoleStore
    const [text, setText] = useState('')
    const [history, setHistory] = useState<string[]>(loadHistory)
    const [cursor, setCursor] = useState(history.length)
    const [draft, setDraft] = useState('')
    const [copied, setCopied] = useState(false)

    const scroller = useRef<HTMLDivElement>(null)
    const input = useRef<HTMLInputElement>(null)
    const stick = useRef(true)

    useEffect(() => {
        void consoleStore.reattach()
        input.current?.focus()
    }, [])

    // Follow the output unless the user scrolled up to read something.
    useLayoutEffect(() => {
        const el = scroller.current

        if (el && stick.current) el.scrollTop = el.scrollHeight
    })

    const submit = useCallback(async () => {
        const typed = text

        setText('')
        setCursor(history.length)
        setDraft('')
        stick.current = true

        if (running) {
            await consoleStore.sendInput(typed, true)

            return
        }

        const parsed = classify(typed)

        if (parsed.kind === 'empty') return

        if (parsed.kind === 'clear') return consoleStore.clear()

        if (parsed.kind === 'help') {
            consoleStore.note(typed.trim(), 'cmd')
            HELP_LINES.forEach((l) => consoleStore.note(l, 'note'))

            return
        }

        // History keeps what Rust says was run (secrets redacted), not the raw
        // text, so a token pasted on the line is not stored.
        await consoleStore.run(parsed.line)

        const shown = consoleStore.lastDisplay ?? null

        if (shown) {
            const next = pushHistory(history, shown)

            setHistory(next)
            setCursor(next.length)
            saveHistory(next)
        }
    }, [text, running, history])

    const onKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
        const key = e.key.toLowerCase()

        if (e.key === 'Enter') {
            e.preventDefault()
            void submit()
        } else if (e.key === 'ArrowUp' || e.key === 'ArrowDown') {
            e.preventDefault()

            if (cursor >= history.length) setDraft(text)

            const step = walkHistory(
                history,
                cursor,
                e.key === 'ArrowUp' ? 'up' : 'down',
                cursor >= history.length ? text : draft
            )

            setCursor(step.cursor)
            setText(step.text)
        } else if (e.ctrlKey && key === 'c') {
            // With text selected Ctrl+C is copy; otherwise it is cancel.
            const selected = window.getSelection()?.toString() ?? ''

            if (selected) return

            e.preventDefault()

            if (running) void consoleStore.stop()
            else setText('')
        } else if (e.ctrlKey && key === 'l') {
            e.preventDefault()
            consoleStore.clear()
        } else if (e.ctrlKey && key === 'd' && running && !text) {
            e.preventDefault()
            void consoleStore.closeInput()
        }
    }

    const copyAll = async () => {
        try {
            await navigator.clipboard.writeText(plainText(transcript))
            setCopied(true)
            setTimeout(() => setCopied(false), 1500)
        } catch {
            // The text is selectable; copying by hand still works.
        }
    }

    return (
        <div className="flex h-full min-h-0 flex-col gap-3 p-4">
            <header className="flex flex-wrap items-center justify-between gap-3">
                <div>
                    <h1 className="text-lg font-bold">Console</h1>
                    <p className="selectable text-xs text-muted">
                        {info.version ?? 'tmc'}
                        {info.path ? ` - ${info.path}` : ''}
                        {info.signedIn
                            ? ' - signed in as you'
                            : ' - using your tmc profiles'}
                    </p>
                </div>

                <div className="flex items-center gap-2">
                    {running && (
                        <button
                            type="button"
                            onClick={() => void consoleStore.stop()}
                            className="flex items-center gap-1.5 rounded-lg border border-danger/50 px-3 py-1.5 text-sm text-danger"
                        >
                            <FiSquare className="size-3.5" />
                            Stop
                        </button>
                    )}
                    <button
                        type="button"
                        onClick={() => void copyAll()}
                        className="flex items-center gap-1.5 rounded-lg border border-border px-3 py-1.5 text-sm hover:border-accent"
                    >
                        <FiCopy className="size-3.5" />
                        {copied ? 'Copied' : 'Copy'}
                    </button>
                    <button
                        type="button"
                        onClick={() => consoleStore.clear()}
                        className="flex items-center gap-1.5 rounded-lg border border-border px-3 py-1.5 text-sm hover:border-accent"
                    >
                        <FiTrash2 className="size-3.5" />
                        Clear
                    </button>
                </div>
            </header>

            <div
                className="flex min-h-[20rem] flex-1 flex-col overflow-hidden rounded-xl border border-border bg-surface"
                onClick={() => {
                    if (!window.getSelection()?.toString()) input.current?.focus()
                }}
            >
                <div
                    ref={scroller}
                    role="log"
                    aria-live="off"
                    onScroll={(e) => {
                        const el = e.currentTarget

                        stick.current =
                            el.scrollHeight - el.scrollTop - el.clientHeight < 24
                    }}
                    className="selectable min-h-0 flex-1 overflow-auto p-3 font-mono text-xs leading-5"
                >
                    {transcript.lines.length === 0 && (
                        <p className="flex items-center gap-1.5 text-muted">
                            <FiTerminal className="size-3.5" />
                            Type a command, or <code>help</code>. The leading{' '}
                            <code>tmc</code> is optional.
                        </p>
                    )}
                    {transcript.lines.map((line) => (
                        <Row key={line.id} line={line} rev={line.rev} />
                    ))}
                </div>

                <div className="flex items-center gap-2 border-t border-border px-3 py-2 font-mono text-xs">
                    <span className="select-none text-accent">
                        {running ? '>' : '$ tmc'}
                    </span>
                    <input
                        ref={input}
                        value={text}
                        onChange={(e) => setText(e.target.value)}
                        onKeyDown={onKeyDown}
                        spellCheck={false}
                        autoCapitalize="off"
                        autoCorrect="off"
                        autoComplete="off"
                        aria-label={running ? 'Input for the running command' : 'Command'}
                        placeholder={
                            running ? 'Input for the running command...' : 'mod list --mine'
                        }
                        className="min-w-0 flex-1 bg-transparent outline-none placeholder:text-muted"
                    />
                </div>
            </div>
        </div>
    )
}
