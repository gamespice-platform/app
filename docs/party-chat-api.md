Party, direct-message chat and live-chat data layer for the GameSpice app, and
the screens built on it (`/party`, `/party/browse`, `/messages`, the live-chat
panel).

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
`unreadCount`, `requestCount`, `respond` (accept or decline a message request),
`block` (`on = false` unblocks).

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
badge 15 s, live chat 4 s. The Requests tab badge polls `requestCount` (15 s);
the requests list is polled only while that folder is open, and a thread opened
outside the listed folder is polled alone (`useChatThread`). Pages added with
"Load more" are re-read whenever the first page refreshes.

Live chat polls ask only for messages `after` the newest held, which cannot show
edits, deletions, reactions or moderator hides on older lines. So every 8th poll
(`LIVE_CHAT_FULL_EVERY`), and after every post, edit, delete or reaction, the
whole newest page is fetched instead and reconciled by id (`reconcileNewest`):
changed rows are replaced and rows missing from the page's range are dropped.

Mutations on the party screens go through `usePartyAction`
(`src/components/party/party-ui.tsx`), which on settle invalidates the `social`
and `play` react-query keys; the party and ready polls are also refreshed by the
screen's `onChanged`. The party heartbeat (10 s) deliberately keeps running while
the window is hidden so a minimised app does not let the party time out.

Hooks: `usePartyMine`, `usePartyInvites`, `usePartyReady`, `useChatUnread`,
`useChatThreads`, `useChatConversation`, `useLiveChat`.

## Site work needed

Session-free `/api/app/v1` routes for the proposed paths above, with the
envelope and string-id/ISO-date conventions the existing party routes use.

## Chat screens

`/messages` (`src/routes/messages.tsx`): inbox, message requests (accept or
decline), block, conversation with polling; `/messages?to=<userId>` drafts a
first message (the Friends rows link there). `<LiveChat target={...} />`
(`src/components/chat/live-chat.tsx`) is the reusable room for party, server
and game targets; the Monitor side panel mounts it as a Chat tab. Pure rules
(composer keys, grouping, line states, rate-limit wait) are in
`src/lib/chat/chat-logic.ts`. `chatApi.respond` and `chatApi.block` are proposed
routes like the rest. Read receipts are not drawn: the thread payload carries
no per-message read state.

Blocking asks for confirmation. The thread payload carries no blocked flag and
there is no blocked-list route, so the Unblock button appears only after blocking
in the open conversation; unblocking from elsewhere needs a site route.
