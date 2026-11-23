import { IpcError } from '~/lib/ipc'

/**
 * Whether a call failed because the site has no app route for it, as opposed to
 * failing for a reason the member can act on.
 *
 * Chat, live chat, party browse/search/stats and moderation are tRPC-only on
 * the site today (session cookie). Their clients here target the REST paths
 * website-city would add; until it does, the request is a 404 (or 405 on a path
 * that exists for another method), and the UI should say "not available in the
 * app yet" rather than show a failure.
 */
export function isUnavailable(e: unknown): boolean {
    if (!(e instanceof IpcError)) return false

    /*
     * Rust surfaces only the envelope's message. A path the site does not route
     * answers with Next's HTML page, which Rust reports as a `network` error
     * naming the status: "Unexpected response from the server (404 Not Found)."
     */
    return (
        e.code === 'network' &&
        /Unexpected response from the server \((404|405|501)\b/.test(e.message)
    )
}
