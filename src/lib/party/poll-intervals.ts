/**
 * How often each party view asks the site again.
 *
 * Chosen against website-city's own cadence (a heartbeat of ~10 s, polled at
 * about half), and kept as named values so a settings screen can scale them
 * rather than a number being buried in a hook.
 */

/** The caller's party and pending invites. */
export const PARTY_POLL_MS = 5_000

/** The ready panel: countdown and who is ready change by the second. */
export const PARTY_READY_POLL_MS = 2_000
