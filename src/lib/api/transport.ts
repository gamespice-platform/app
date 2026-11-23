import { z } from 'zod'

import { call } from '~/lib/ipc'

/**
 * The one way the frontend reaches `/api/app/v1`: through Rust, which owns the
 * bearer token and the base URL. Split out of `client.ts` so the party and chat
 * clients share it rather than each growing a copy.
 */

const RawJson = z.unknown()

export async function get<S extends z.ZodType>(
    path: string,
    schema: S,
    query?: Record<string, QueryValue>,
    auth = false
): Promise<z.infer<S>> {
    const raw = await call('api_get', RawJson, {
        path,
        query: query ? toPairs(query) : undefined,
        auth,
    })

    return parseOrThrow(path, schema, raw)
}

export async function send<S extends z.ZodType>(
    method: 'POST' | 'PATCH' | 'PUT' | 'DELETE',
    path: string,
    schema: S,
    body?: unknown,
    auth = true
): Promise<z.infer<S>> {
    const raw = await call('api_send', RawJson, { method, path, body, auth })

    return parseOrThrow(path, schema, raw)
}

function parseOrThrow<S extends z.ZodType>(
    path: string,
    schema: S,
    raw: unknown
): z.infer<S> {
    const parsed = schema.safeParse(raw)

    if (!parsed.success) {
        /*
         * A shape mismatch is a CONTRACT drift — this file and
         * website-city's `src/types/app-api/contract.ts` have diverged, or the
         * server is older than the app. Both are bugs worth failing loudly for
         * rather than papering over with optional chaining at every call site.
         */
        console.error(`[api] ${path} did not match the contract`, parsed.error, raw)

        throw new Error(
            'The server sent something this version of the app does not understand. Try updating.'
        )
    }

    return parsed.data
}

/** Values a query string can carry. Anything else is a programming error. */
export type QueryValue = string | number | boolean | null | undefined | (string | number)[]

/**
 * Flatten to the `[key, value][]` the Rust side encodes.
 *
 * Arrays repeat their key, matching the server's `parseQuery`. `undefined` and
 * `false` are dropped rather than sent: the API's booleans are all opt-IN
 * (`nsfw`, `onlineOnly`), so an explicit `false` and an absent key mean the
 * same thing and omitting keeps the URL short.
 */
function toPairs(query: Record<string, QueryValue>): [string, string][] {
    const out: [string, string][] = []

    for (const [key, value] of Object.entries(query)) {
        if (value == null || value === false || value === '') continue

        if (Array.isArray(value)) {
            for (const item of value) out.push([key, String(item)])
            continue
        }

        out.push([key, String(value)])
    }

    return out
}
