import {
    useCallback,
    useEffect,
    useId,
    useLayoutEffect,
    useMemo,
    useRef,
    useState,
    type ReactNode,
} from 'react'
import { Check, ChevronDown } from 'lucide-react'

/**
 * A dropdown the app actually owns.
 *
 * WHY NOT `<select>`
 * ------------------
 * Because a native `<select>`'s POPUP is drawn by the operating system and
 * nothing in CSS reaches it. The closed control can be styled; the open list
 * cannot. That produces exactly the mismatch the custom titlebar exists to
 * remove — a GTK list on Linux, an Aqua one on macOS, a Fluent one on Windows —
 * and in dark mode it is worse than a mismatch, because two of the three ignore
 * the app's palette entirely and render dark text on white while everything
 * around them is dark.
 *
 * It also cannot hold an icon, and a game picker without game icons is a wall
 * of text.
 *
 * WHAT THAT COSTS, AND WHAT IS PAID BACK
 * --------------------------------------
 * Everything a native control gives for free has to be re-implemented, and the
 * usual failure of a hand-rolled dropdown is that half of it is not:
 *
 *   * **Keyboard.** Arrows move, Home/End jump, Enter and Space commit, Escape
 *     closes, Tab closes and moves on. Typing letters jumps to a match, which
 *     is the one native behaviour people use without noticing.
 *   * **Screen readers.** A real `combobox`/`listbox` pair with
 *     `aria-activedescendant`, so the focus never actually leaves the button
 *     and the reader still announces the highlighted row.
 *   * **Dismissal.** Pointer-down outside, not click — a click that starts
 *     inside and ends outside is a drag, not a dismissal.
 *   * **Placement.** Flipped above the button when there is no room below,
 *     which on a phone-sized window is most of the time.
 *
 * MOBILE
 * ------
 * The list is a normal absolutely-positioned element rather than a sheet. This
 * app's mobile shell is the same document at a narrower width, and a control
 * that became a modal below 768px would be a second thing to keep working.
 */

export type SelectOption<T extends string> = {
    value: T
    label: string
    /** Second line, for the cases where the label alone is ambiguous. */
    hint?: string
    /** An icon, a game's own artwork, a colour swatch. */
    icon?: ReactNode
    disabled?: boolean
    /** Renders a heading above this option. */
    group?: string
}

type Props<T extends string> = {
    value: T
    options: SelectOption<T>[]
    onChange: (next: T) => void
    /** Accessible name. Required — a picker nobody can name is one nobody can use. */
    label: string
    /** Shown when `value` matches no option. */
    placeholder?: string
    disabled?: boolean
    className?: string
    /** Match the width of the button rather than sizing to content. */
    fullWidth?: boolean
    /**
     * A search box at the top of the open list that narrows the rows by label
     * or value. Placeholder text for the box; omit for no box.
     */
    search?: string
    /**
     * A "Custom…" row at the bottom: choosing it turns the control into a text
     * field, `check` validates inline (an error message, or `null`), and Enter
     * hands the trimmed text to `onCustom`. With a search box, the row offers
     * the typed query directly.
     */
    custom?: {
        label: string
        /** The row's text when a query is typed, `{value}` for the query. */
        queryLabel?: string
        check: (text: string) => string | null
        onCustom: (text: string) => void
    }
}

export default function Select<T extends string>({
    value,
    options,
    onChange,
    label,
    placeholder = 'Select…',
    disabled,
    className = '',
    fullWidth,
    search,
    custom,
}: Props<T>) {
    const [open, setOpen] = useState(false)
    const [query, setQuery] = useState('')
    // The "Custom…" text field: `null` when the control is a dropdown.
    const [editing, setEditing] = useState<string | null>(null)
    const [active, setActive] = useState(0)
    const [above, setAbove] = useState(false)

    const rootRef = useRef<HTMLDivElement>(null)
    const buttonRef = useRef<HTMLButtonElement>(null)
    const listRef = useRef<HTMLDivElement>(null)
    const searchRef = useRef<HTMLInputElement>(null)
    const editRef = useRef<HTMLInputElement>(null)

    /*
     * Type-ahead state. A ref rather than state: it changes on every keystroke
     * and nothing renders from it, so putting it in state would be a re-render
     * per character for no visible difference.
     */
    const typed = useRef({ text: '', at: 0 })

    const id = useId()

    const selected = options.find((option) => option.value === value)

    /*
     * The rows on show: every option, or those matching the search box. The
     * Custom row is not an option (its value is not a `T`); it sits at index
     * `rows.length` when there is one.
     */
    const rows = useMemo(() => {
        const needle = query.trim().toLowerCase()

        if (!search || !needle) return options

        return options.filter(
            (option) =>
                option.label.toLowerCase().includes(needle) ||
                option.value.toLowerCase().includes(needle)
        )
    }, [options, query, search])

    const count = rows.length + (custom ? 1 : 0)

    const selectable = useMemo(
        () => rows.filter((option) => !option.disabled),
        [rows]
    )

    const indexOfValue = useCallback(
        () =>
            Math.max(
                0,
                rows.findIndex((option) => option.value === value)
            ),
        [rows, value]
    )

    // ------------------------------------------------------------ Dismissal
    useEffect(() => {
        if (!open) return

        const onPointerDown = (event: PointerEvent) => {
            if (!rootRef.current?.contains(event.target as Node)) setOpen(false)
        }

        // `pointerdown`, not `click`: a click that starts inside the list and
        // ends outside it is a drag, and closing on that loses the selection
        // somebody was in the middle of making.
        document.addEventListener('pointerdown', onPointerDown)

        return () => document.removeEventListener('pointerdown', onPointerDown)
    }, [open])

    // ------------------------------------------------------------ Placement
    useLayoutEffect(() => {
        if (!open) return

        const button = buttonRef.current

        if (!button) return

        const rect = button.getBoundingClientRect()
        const below = window.innerHeight - rect.bottom

        // 260px is the list's max height. Flipping matters most on the mobile
        // shell, where a control near the bottom has nowhere to open.
        setAbove(below < 260 && rect.top > below)
    }, [open])

    // The search box takes the keyboard while the list is open.
    useEffect(() => {
        if (open) searchRef.current?.focus()
    }, [open])

    /** Opening starts with an empty search, so every row shows again. */
    const openList = () => {
        setQuery('')
        setActive(indexOfValue())
        setOpen(true)
    }

    const isEditing = editing !== null

    useEffect(() => {
        if (isEditing) editRef.current?.focus()
    }, [isEditing])

    // Keep the highlighted row in view while arrowing through a long list.
    useEffect(() => {
        if (!open) return

        listRef.current
            ?.querySelector(`[data-index="${active}"]`)
            ?.scrollIntoView({ block: 'nearest' })
    }, [open, active])

    const commit = (option: SelectOption<T>) => {
        if (option.disabled) return

        onChange(option.value)
        setOpen(false)
        buttonRef.current?.focus()
    }

    const startCustom = () => {
        setEditing(query.trim() || (selected ? '' : value))
        setOpen(false)
    }

    const commitAt = (index: number) => {
        const option = rows[index]

        if (option) commit(option)
        else if (custom && index === rows.length) startCustom()
    }

    const move = (delta: number) => {
        if (!selectable.length && !custom) return

        setActive((current) => {
            let next = current

            for (let step = 0; step < count; step += 1) {
                next = (next + delta + count) % count

                if (!rows[next]?.disabled) return next
            }

            return current
        })
    }

    const onKeyDown = (event: React.KeyboardEvent) => {
        switch (event.key) {
            case 'ArrowDown':
            case 'ArrowUp': {
                event.preventDefault()

                if (!open) {
                    openList()

                    return
                }

                move(event.key === 'ArrowDown' ? 1 : -1)

                return
            }
            case 'Home':
            case 'End': {
                if (!open) return

                event.preventDefault()
                setActive(event.key === 'Home' ? 0 : count - 1)

                return
            }
            case 'Enter':
            case ' ': {
                // A space typed into the search box is text, not a commit.
                if (event.key === ' ' && event.target === searchRef.current) {
                    return
                }

                event.preventDefault()

                if (!open) {
                    openList()

                    return
                }

                commitAt(active)

                return
            }
            case 'Escape': {
                if (!open) return

                event.preventDefault()
                setOpen(false)
                buttonRef.current?.focus()

                return
            }
            case 'Tab': {
                // Closes and moves on, as a native select does. Not prevented —
                // the focus should actually go to the next control.
                setOpen(false)

                return
            }
            default:
                break
        }

        /*
         * Type-ahead. A single printable character, with a one-second window in
         * which further characters extend the search rather than restart it —
         * so "mi", "min" and "mine" all reach Minecraft while "m" then a pause
         * then "m" cycles through the entries beginning with M.
         */
        if (event.key.length !== 1 || event.ctrlKey || event.metaKey) return

        // With a search box the letters are the search.
        if (event.target === searchRef.current) return

        const now = Date.now()

        typed.current.text =
            now - typed.current.at > 1000
                ? event.key.toLowerCase()
                : typed.current.text + event.key.toLowerCase()

        typed.current.at = now

        const query = typed.current.text

        const from = open ? active : indexOfValue()

        // Search from the row AFTER the current one, wrapping, so repeating a
        // letter steps through matches instead of sticking on the first.
        for (let step = 1; step <= rows.length; step += 1) {
            const index = (from + step) % rows.length
            const option = rows[index]

            if (
                option &&
                !option.disabled &&
                option.label.toLowerCase().startsWith(query)
            ) {
                setActive(index)

                if (!open) commit(option)

                return
            }
        }
    }

    if (custom && editing !== null) {
        const error = editing.trim() ? custom.check(editing) : null
        const done = () => {
            const text = editing.trim()

            if (text && !custom.check(text)) custom.onCustom(text)
            setEditing(null)
        }

        return (
            <div
                className={`${fullWidth ? 'w-full' : 'inline-block'} ${className}`}
            >
                <input
                    ref={editRef}
                    type="text"
                    aria-label={label}
                    aria-invalid={error !== null}
                    aria-describedby={error ? `${id}-error` : undefined}
                    value={editing}
                    spellCheck={false}
                    autoComplete="off"
                    onChange={(event) => setEditing(event.target.value)}
                    onKeyDown={(event) => {
                        if (event.key === 'Enter') {
                            event.preventDefault()
                            if (!error) done()
                        } else if (event.key === 'Escape') {
                            event.preventDefault()
                            setEditing(null)
                        }
                    }}
                    onBlur={() => (error ? setEditing(null) : done())}
                    className={`w-full rounded-lg border bg-surface px-2 py-1.5 text-sm focus:outline-none ${
                        error
                            ? 'border-danger/50'
                            : 'border-border focus:border-accent'
                    }`}
                />
                {error && (
                    <p id={`${id}-error`} className="mt-1 text-xs text-danger">
                        {error}
                    </p>
                )}
            </div>
        )
    }

    const customIndex = rows.length

    return (
        <div
            ref={rootRef}
            className={`relative ${fullWidth ? 'w-full' : 'inline-block'} ${className}`}
        >
            <button
                ref={buttonRef}
                type="button"
                role="combobox"
                aria-expanded={open}
                aria-haspopup="listbox"
                aria-controls={`${id}-list`}
                aria-label={label}
                aria-activedescendant={open ? `${id}-opt-${active}` : undefined}
                disabled={disabled}
                onClick={() => {
                    if (disabled) return

                    if (open) setOpen(false)
                    else openList()
                }}
                onKeyDown={onKeyDown}
                className={`flex ${
                    fullWidth ? 'w-full' : ''
                } items-center gap-2 rounded-lg border border-border bg-surface px-2 py-1.5 text-left text-sm transition-colors hover:border-accent focus:border-accent focus:outline-none disabled:cursor-not-allowed disabled:opacity-50`}
            >
                {selected?.icon && (
                    <span className="flex size-4 shrink-0 items-center justify-center">
                        {selected.icon}
                    </span>
                )}

                <span
                    className={`min-w-0 flex-1 truncate ${
                        selected ? '' : 'text-muted'
                    }`}
                >
                    {selected?.label ?? placeholder}
                </span>

                <ChevronDown
                    aria-hidden
                    className={`size-3.5 shrink-0 text-muted transition-transform ${
                        open ? 'rotate-180' : ''
                    }`}
                />
            </button>

            {open && (
                <div
                    ref={listRef}
                    id={`${id}-list`}
                    role="listbox"
                    aria-label={label}
                    className={`absolute z-50 max-h-[260px] min-w-full overflow-y-auto rounded-lg border border-border bg-surface p-1 shadow-lg ${
                        above ? 'bottom-full mb-1' : 'top-full mt-1'
                    }`}
                >
                    {search && (
                        <input
                            ref={searchRef}
                            type="search"
                            aria-label={search}
                            aria-controls={`${id}-list`}
                            aria-activedescendant={`${id}-opt-${active}`}
                            placeholder={search}
                            value={query}
                            spellCheck={false}
                            autoComplete="off"
                            onChange={(event) => {
                                setQuery(event.target.value)
                                setActive(0)
                            }}
                            onKeyDown={onKeyDown}
                            className="mb-1 w-full rounded-md border border-border bg-surface-2 px-2 py-1 text-sm focus:border-accent focus:outline-none"
                        />
                    )}

                    {rows.length === 0 && !custom && (
                        <p className="px-2 py-3 text-center text-xs text-muted">
                            Nothing to choose from
                        </p>
                    )}

                    {rows.map((option, index) => {
                        const previous = rows[index - 1]?.group
                        const heading =
                            option.group && option.group !== previous
                                ? option.group
                                : undefined

                        return (
                            <div key={option.value}>
                                {heading && (
                                    <p className="px-2 pb-1 pt-2 text-[0.65rem] font-semibold uppercase tracking-wide text-muted">
                                        {heading}
                                    </p>
                                )}

                                <div
                                    id={`${id}-opt-${index}`}
                                    data-index={index}
                                    role="option"
                                    aria-selected={option.value === value}
                                    aria-disabled={option.disabled}
                                    /*
                                     * `pointerdown`, not `click`: the document
                                     * listener above closes on pointerdown, and
                                     * a click handler here would never fire
                                     * because the list is gone before the
                                     * pointer comes back up.
                                     */
                                    onPointerDown={(event) => {
                                        event.preventDefault()
                                        commit(option)
                                    }}
                                    onPointerEnter={() =>
                                        !option.disabled && setActive(index)
                                    }
                                    className={`flex cursor-pointer items-center gap-2 rounded-md px-2 py-1.5 text-sm ${
                                        option.disabled
                                            ? 'cursor-not-allowed opacity-40'
                                            : index === active
                                              ? 'bg-surface-tertiary'
                                              : ''
                                    }`}
                                >
                                    {option.icon && (
                                        <span className="flex size-4 shrink-0 items-center justify-center">
                                            {option.icon}
                                        </span>
                                    )}

                                    <span className="min-w-0 flex-1">
                                        <span className="block truncate">
                                            {option.label}
                                        </span>
                                        {option.hint && (
                                            <span className="block truncate text-[0.7rem] text-muted">
                                                {option.hint}
                                            </span>
                                        )}
                                    </span>

                                    {option.value === value && (
                                        <Check
                                            aria-hidden
                                            className="size-3.5 shrink-0 text-accent"
                                        />
                                    )}
                                </div>
                            </div>
                        )
                    })}

                    {custom && (
                        <div
                            id={`${id}-opt-${customIndex}`}
                            data-index={customIndex}
                            role="option"
                            aria-selected={false}
                            onPointerDown={(event) => {
                                event.preventDefault()
                                startCustom()
                            }}
                            onPointerEnter={() => setActive(customIndex)}
                            className={`flex cursor-pointer items-center gap-2 rounded-md border-t border-border px-2 py-1.5 text-sm text-accent ${
                                customIndex === active ? 'bg-surface-tertiary' : ''
                            }`}
                        >
                            {query.trim() && custom.queryLabel
                                ? custom.queryLabel.replace('{value}', query.trim())
                                : custom.label}
                        </div>
                    )}
                </div>
            )}
        </div>
    )
}
