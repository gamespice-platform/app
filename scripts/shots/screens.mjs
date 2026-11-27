/**
 * The screen list: what capture.mjs shoots and the order compose.mjs tells the
 * story in. Add a screen here (route, copy, what to wait for) and both steps
 * pick it up. `enabled: false` keeps a slot in the tour without shooting it,
 * for a screen that is not on this branch yet.
 *
 * slot     file stem: <slot>-<light|dark>.png in out/raw, and the website's
 *          public/app/shots/<slot>-<scheme>-<800|1600>.webp
 * route    hash route in the app
 * wait     a selector that is visible once the screen has its data
 * size     optional { width, height } viewport instead of SIZE (keep 16:10)
 * gpu      needs real WebGL (the 3D avatar): capture on a GPU box
 * init     optional: a function run in the page before the app loads (localStorage, window.__shotsSettings)
 * act      optional: async (page) => {} run after load, before the shot
 * settle   optional ms to wait before the shot (default 2500, or SETTLE)
 * callouts optional: [{ at: locator, label }]; capture records where `at` is, compose draws the label there
 */
export const screens = [
    {
        // /play and /games redirect to /library, the app's start page.
        slot: 'play', route: '/library', wait: 'main',
        title: 'Play', caption: 'Your games, sandboxes and friends online, one click from launch.',
    },
    {
        // The website's hero; a different screen from `play` so the /app page
        // does not show the same picture twice.
        slot: 'hub', route: '/games/playlot', wait: 'main', tour: false,
        title: 'Game hub', caption: 'Each GameSpice game has a hub with play, solo, its live servers and your avatar.',
    },
    {
        slot: 'mods', route: '/browse/mod', wait: 'main',
        title: 'Discover mods', caption: 'Browse maps, mods and packs from every community.',
    },
    {
        slot: 'modpage', route: '/view/mod/2', wait: 'main',
        title: 'Install in one click', caption: 'Every mod page shows what it changes and installs into the right sandbox.',
        callouts: [{ at: 'text=Install into a sandbox >> xpath=ancestor::*[.//button][1]', label: 'Pick a sandbox, press Install' }],
    },
    {
        slot: 'workshop', route: '/workshop', wait: 'main',
        title: 'Workshop', caption: 'Subscribe to map and mod packs and they stay up to date; edit your own.',
    },
    {
        slot: 'sandboxes', route: '/sandboxes', wait: 'main',
        title: 'Sandboxes', caption: 'Keep mod sets apart: one game, as many setups as you like.',
        act: async (page) => { await page.getByText('Party night', { exact: true }).first().click() },
    },
    {
        slot: 'servers', route: '/browse/server', wait: 'main',
        title: 'Servers with live latency', caption: 'Real game queries and a running ping trace for every server.',
        act: async (page) => { await page.getByText('Ember Ridge | Casual 24/7').first().click() },
        callouts: [{ at: '[role="img"][aria-label^="Latency over"]', label: 'Ping measured from your machine' }],
    },
    {
        slot: 'favourites', route: '/browse/server?fav=1', wait: 'text=Ember Ridge | Casual 24/7',
        title: 'Favourite servers', caption: 'Star a server to keep it here, synced to your account or kept on this device.',
    },
    {
        // The real CORE-SS :: US Coop (Left 4 Dead 2, prod item 1552); the graph and roster are seeded (seed.ts).
        // 1440x900 keeps the 16:10 shape compose.mjs assumes.
        slot: 'monitor', route: '/monitor/addr?host=74.91.124.246&port=27015&app=1&name=CORE-SS%20%3A%3A%20US%20Coop',
        wait: 'text=Players (20)', size: { width: 1440, height: 900 },
        // The browser context is shared, so undo the layout the alerts shot leaves in localStorage.
        init: () => localStorage.setItem('tmc.monitor.layout', 'graph'),
        title: 'Server monitor', caption: 'A live latency graph, the player list and RCON in one window.',
    },
    {
        // The same server on the Notifications tab. seed.ts changes the roster and map while the page watches
        // (__shotsMonitorLive), so these are alerts the rules really raised; a 1 s probe gets there in ~25 s.
        slot: 'alerts', route: '/monitor/addr?host=74.91.124.246&port=27015&app=1&name=CORE-SS%20%3A%3A%20US%20Coop',
        wait: 'text=Recent alerts', size: { width: 1440, height: 900 }, settle: 30000,
        init: () => {
            const rule = (id, kind, threshold = 0, seconds = 0, cooldownSecs = 10) => ({ id, kind, enabled: true, threshold, seconds, cooldownSecs })
            localStorage.setItem('gs.monitor.alerts.74.91.124.246:27015', JSON.stringify({
                enabled: true, desktop: true, historyLimit: 100,
                rules: [rule('r1', 'join'), rule('r2', 'leave'), rule('r3', 'map', 0, 0, 5), rule('r4', 'above', 20, 0, 300),
                    rule('r5', 'latency', 150, 30, 300), rule('r6', 'offline', 0, 5, 60)],
            }))
            localStorage.setItem('tmc.monitor.layout', 'alerts')
            globalThis.__shotsSettings = { latencyIntervalMs: 1000 }
            globalThis.__shotsMonitorLive = true
        },
        title: 'Server alerts', caption: 'Rules for joins, leaves, map changes and downtime, with desktop notifications.',
    },
    {
        slot: 'downloads', route: '/downloads', wait: 'main',
        title: 'Downloads', caption: 'A queue you control: priorities, pauses and speed limits.',
    },
    {
        slot: 'library', route: '/installed-mods', wait: 'main',
        title: 'Library', caption: 'Installed mods stay up to date on their own.',
    },
    {
        slot: 'stores', route: '/stores', wait: 'text=Map archive',
        title: 'Stores', caption: 'Named download folders on any drive, kept for good or emptied when the app closes.',
    },
    {
        slot: 'avatar', route: '/avatar', wait: 'canvas', gpu: true,
        title: 'Avatar studio', caption: 'Build your look once and wear it in every game.',
    },
    {
        slot: 'friends', route: '/friends', wait: 'main',
        title: 'Friends', caption: 'See who is online and what they are playing.',
    },
    {
        slot: 'settings', route: '/settings', wait: 'main',
        title: 'Settings', caption: 'Themes, folders, plugins and logging, all in one place.',
    },
    { slot: 'store', route: '/store', wait: 'main', title: 'Store', caption: 'Every game\'s store: outfits, packs and points in one place.' },
    { slot: 'stats', route: '/stats', wait: 'main', title: 'Stats', caption: 'Your playtime and progress across games.' },
]

/** The narrative order of the presentation (slots; missing or disabled are skipped). */
export const tour = ['play', 'mods', 'modpage', 'workshop', 'sandboxes', 'servers', 'favourites', 'monitor', 'alerts', 'downloads', 'library', 'stores', 'avatar', 'friends', 'store', 'stats', 'settings']

export const themes = ['light', 'dark']
export const SIZE = { width: 1600, height: 1000 }
