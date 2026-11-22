import { useState } from 'react'

/**
 * A text input that commits on blur and on Enter, not on every keystroke.
 *
 * Each commit is an IPC round trip and a database write; committing per
 * character would be one per letter typed into a version field.
 */
export default function TextField({
    value,
    onCommit,
    placeholder,
    numeric,
    min,
    max,
    label,
}: {
    value: string
    onCommit: (next: string) => void
    placeholder?: string
    numeric?: boolean
    min?: number
    max?: number
    /** The accessible name, when no visible label wraps the field. */
    label?: string
}) {
    const [draft, setDraft] = useState(value)
    const [valueSeen, setValueSeen] = useState(value)

    // A new value from outside replaces whatever was being typed.
    if (valueSeen !== value) {
        setValueSeen(value)
        setDraft(value)
    }

    return (
        <input
            value={draft}
            type={numeric ? 'number' : 'text'}
            min={min}
            max={max}
            placeholder={placeholder}
            aria-label={label}
            onChange={(e) => setDraft(e.target.value)}
            onBlur={() => draft !== value && onCommit(draft)}
            onKeyDown={(e) => {
                if (e.key === 'Enter') e.currentTarget.blur()
                if (e.key === 'Escape') setDraft(value)
            }}
            className="w-full rounded-lg border border-border bg-background px-2 py-1.5 text-sm"
        />
    )
}
