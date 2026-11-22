/**
 * "Never ask me again" for games that are started by handing the server's
 * address to an externally installed client (the dialog's Connect mode).
 *
 * Per game, per device, in localStorage: it only skips a confirmation, grants
 * nothing, and `playConnect` in Rust still checks the link's scheme.
 */
const key = (appId: number) => `gs.connect.noask.${appId}`

export function getConnectNoAsk(appId: number): boolean {
    try {
        return localStorage.getItem(key(appId)) === '1'
    } catch {
        return false
    }
}

export function setConnectNoAsk(appId: number, on: boolean): void {
    try {
        if (on) localStorage.setItem(key(appId), '1')
        else localStorage.removeItem(key(appId))
    } catch {
        // Storage blocked: the dialog simply keeps asking.
    }
}
