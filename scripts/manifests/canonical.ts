/*
 * The canonical form of an install manifest (docs/loader-manifests.md,
 * "Canonical form"). Signatures and idempotency keys are computed over these
 * bytes, and Rust and the website must produce the same ones: the shared
 * fixtures in schemas/fixtures/canonical/ hold them to it.
 *
 * - UTF-8 JSON, no insignificant whitespace;
 * - object keys sorted by Unicode code point at every depth (which is UTF-8
 *   byte order, and what Rust's `str` ordering gives; it differs from JCS's
 *   UTF-16 order only for keys mixing astral and U+E000..U+FFFF characters);
 * - integers only: a fraction, an exponent result that is not a safe integer,
 *   NaN or Infinity is an error, never rounded;
 * - strings escaped minimally, as RFC 8785 JCS: `"` `\` and the controls
 *   below U+0020 only (`\b \t \n \f \r` by name, the rest as lowercase
 *   `\u00xx`). A lone surrogate is an error;
 * - the top-level `$schema` key removed (nested ones are data and are kept).
 *
 * The repository form ("pretty") is the same document with `$schema` first,
 * then the sorted keys, indented by four spaces, with a trailing newline.
 */

export class CanonicalError extends Error {}

const SCHEMA_KEY = '$schema'

/** Code point order: compare the UTF-8 bytes. */
export function compareKeys(a: string, b: string): number {
    return Buffer.compare(Buffer.from(a, 'utf8'), Buffer.from(b, 'utf8'))
}

export function escapeString(s: string): string {
    let out = '"'
    for (let i = 0; i < s.length; i++) {
        const c = s.charCodeAt(i)
        if (c >= 0xd800 && c <= 0xdbff) {
            const next = s.charCodeAt(i + 1)
            if (!(next >= 0xdc00 && next <= 0xdfff)) {
                throw new CanonicalError(`lone surrogate at index ${i}`)
            }
            out += s[i] + s[i + 1]
            i++
            continue
        }
        if (c >= 0xdc00 && c <= 0xdfff) {
            throw new CanonicalError(`lone surrogate at index ${i}`)
        }
        switch (c) {
            case 0x22:
                out += '\\"'
                break
            case 0x5c:
                out += '\\\\'
                break
            case 0x08:
                out += '\\b'
                break
            case 0x09:
                out += '\\t'
                break
            case 0x0a:
                out += '\\n'
                break
            case 0x0c:
                out += '\\f'
                break
            case 0x0d:
                out += '\\r'
                break
            default:
                out +=
                    c < 0x20
                        ? '\\u' + c.toString(16).padStart(4, '0')
                        : s[i]
        }
    }
    return out + '"'
}

function number(n: number, where: string): string {
    if (!Number.isSafeInteger(n)) {
        throw new CanonicalError(
            `${where || '/'}: ${n} is not an integer (the canonical form has no floats)`,
        )
    }
    // String(-0) is "0", as JCS wants.
    return String(n)
}

function isObject(v: unknown): v is Record<string, unknown> {
    return typeof v === 'object' && v !== null && !Array.isArray(v)
}

function sortedKeys(o: Record<string, unknown>): string[] {
    return Object.keys(o).sort(compareKeys)
}

function pointerToken(k: string): string {
    return k.replace(/~/g, '~0').replace(/\//g, '~1')
}

function write(v: unknown, where: string): string {
    if (v === null) return 'null'
    if (v === true) return 'true'
    if (v === false) return 'false'
    if (typeof v === 'number') return number(v, where)
    if (typeof v === 'string') return escapeString(v)
    if (Array.isArray(v)) {
        return '[' + v.map((x, i) => write(x, `${where}/${i}`)).join(',') + ']'
    }
    if (isObject(v)) {
        return (
            '{' +
            sortedKeys(v)
                .map(
                    (k) =>
                        escapeString(k) +
                        ':' +
                        write(v[k], `${where}/${pointerToken(k)}`),
                )
                .join(',') +
            '}'
        )
    }
    throw new CanonicalError(`${where || '/'}: ${typeof v} is not JSON`)
}

function withoutSchema(doc: unknown): unknown {
    if (!isObject(doc) || !(SCHEMA_KEY in doc)) return doc
    const rest: Record<string, unknown> = { ...doc }
    delete rest[SCHEMA_KEY]
    return rest
}

/** The canonical text of a parsed document. */
export function canonicalString(doc: unknown): string {
    return write(withoutSchema(doc), '')
}

/** The canonical bytes: what is signed and hashed. */
export function canonicalBytes(doc: unknown): Buffer {
    return Buffer.from(canonicalString(doc), 'utf8')
}

function pretty(v: unknown, indent: string, where: string, top: boolean): string {
    const inner = indent + '    '
    if (Array.isArray(v)) {
        if (v.length === 0) return '[]'
        return (
            '[\n' +
            v.map((x, i) => inner + pretty(x, inner, `${where}/${i}`, false)).join(',\n') +
            '\n' +
            indent +
            ']'
        )
    }
    if (isObject(v)) {
        const keys = sortedKeys(v)
        if (top && SCHEMA_KEY in v) {
            keys.splice(keys.indexOf(SCHEMA_KEY), 1)
            keys.unshift(SCHEMA_KEY)
        }
        if (keys.length === 0) return '{}'
        return (
            '{\n' +
            keys
                .map(
                    (k) =>
                        inner +
                        escapeString(k) +
                        ': ' +
                        pretty(v[k], inner, `${where}/${pointerToken(k)}`, false),
                )
                .join(',\n') +
            '\n' +
            indent +
            '}'
        )
    }
    return write(v, where)
}

/**
 * The repository form: `$schema` first (when present), every other key
 * sorted as in the canonical form, four-space indent, trailing newline.
 */
export function prettyString(doc: unknown): string {
    return pretty(doc, '', '', true) + '\n'
}

/** Parse JSON text. Duplicate keys are not detected: the last one wins. */
export function parse(text: string): unknown {
    return JSON.parse(text)
}
