Party, direct-message chat and live-chat data layer for the GameSpice app. This
is the data layer only; screens come later.

## Reachability

The app calls `/api/app/v1` through Rust with a device bearer token. The site's
party, chat and live-chat features are tRPC procedures behind a session cookie,
which the device token does not carry. Only the party game-client REST routes
are therefore reachable today. Everything else is wrapped against the REST path
the site would need to add (marked "proposed"). Until that route exists the call
fails with a `network` error naming 404, and `isUnavailable(e)`
(`src/lib/api/unavailable.ts`) turns that into "not available in the app yet".

Schemas live in `src/lib/api/party-schemas.ts` and `chat-schemas.ts`, separate
from `contract.ts`, which is a verbatim mirror of the site contract and is never
hand-edited. When the contract sync brings these in, delete the local copies.

## Party (`src/lib/api/party.ts`, `partyApi`)

Reachable now: `mine`, `one`, `invites`, `readyState`, `create`, `join`, `leave`,
`end`, `heartbeat`, `invite`, `respondToInvite`, `kick`, `setRole`,
`transferHost`, `setReady`, `connected`, `start`, `forceStart`, `cancelStart`,
`reserveOffer`, `reserve`, `cancelReservation`.

Proposed, no app route: `browse`, `searchStart`, `searchState`, `searchCancel`,
`stats`, `moderate`.

## Direct-message chat (`src/lib/api/chat.ts`, `chatApi`)

All proposed, no app route: `threads`, `thread`, `history`, `send`, `read`,
`unreadCount`, `requestCount`.

## Live chat (`src/lib/api/live-chat.ts`, `liveChatApi`)

All proposed, no app route: `channel`, `history`, `post`, `edit`, `remove`,
`react`.

## Not wrapped

Party media and other tRPC-only admin procedures, and chat attachments upload,
have no app route and no wrapper yet.

## Live updates

The site itself polls (react-query `refetchInterval`); there are no websockets
or SSE. The app does the same through `usePoll` (`src/lib/hooks/use-poll.ts`):
it pauses while the window is hidden, backs off exponentially on errors
(`pollDelay`, ceiling configurable, 0 means unbounded), and stops when the route
is unavailable. Intervals are constants in `src/lib/party/poll-intervals.ts` and
`src/lib/chat/poll-intervals.ts`: party 5 s, ready panel 2 s, active DM 3 s, DM
badge 15 s, live chat 4 s.

Hooks: `usePartyMine`, `usePartyInvites`, `usePartyReady`, `useChatUnread`,
`useChatThreads`, `useChatConversation`, `useLiveChat`.

## Site work needed

Session-free `/api/app/v1` routes for the proposed paths above, with the
envelope and string-id/ISO-date conventions the existing party routes use.
