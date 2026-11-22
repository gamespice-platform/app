import { useState } from 'react'
import { Link } from 'react-router-dom'
import { FiAlertTriangle, FiFolder } from 'react-icons/fi'

import { Skeleton } from '~/components/skeleton'
import { Section } from '~/components/form'
import { useFolderPicker } from '~/components/folder-picker'
import { isIpcError } from '~/lib/ipc'
import { useSettings } from '~/lib/settings/provider'

/**
 * The one games setting that is not about a game: where installers put files
 * before unpacking them. Per-game settings (install folder, Find my games)
 * live in Library -> Apps.
 *
 * `setDownloadDir` is a dedicated Rust command that validates the folder as a
 * jail anchor; `settings_patch` refuses the field. The picker is an
 * affordance, not a control.
 */
export default function DownloadsSettingsRoute() {
    const { app, setDownloadDir } = useSettings()
    const { pick, element: picker } = useFolderPicker()
    const [error, setError] = useState<string | null>(null)

    if (!app) return <Skeleton className="h-40 w-full" />

    const choose = async () => {
        const dir = await pick({
            title: 'Choose a download folder',
            hint: 'Installers put files here before unpacking them.',
            initial: app.downloadDir,
        })

        if (dir === null) return

        setError(null)

        try {
            await setDownloadDir(dir)
        } catch (err) {
            setError(isIpcError(err) ? err.message : 'That folder cannot be used.')
        }
    }

    return (
        <>
            {error && (
                <p className="flex items-start gap-2 rounded-lg border border-danger p-3 text-xs text-danger">
                    <FiAlertTriangle className="mt-0.5 size-3.5 shrink-0" />
                    {error}
                </p>
            )}

            <Section
                title="Downloads"
                hint="Where installers put files before unpacking them. Defaults to the app's own cache."
            >
                <div className="flex items-center justify-between gap-3 px-3 py-2.5">
                    <p className="selectable min-w-0 truncate text-xs text-muted">
                        {app.downloadDir ?? 'App cache (cleared on launch)'}
                    </p>

                    <button
                        type="button"
                        onClick={() => void choose()}
                        aria-label="Choose download folder"
                        className="shrink-0 rounded-lg border border-border p-2 text-muted hover:text-foreground"
                    >
                        <FiFolder className="size-4" />
                    </button>
                </div>
            </Section>

            <p className="text-xs text-muted">
                Install folders and Find my games are per game:{' '}
                <Link to="/my-apps" className="underline">
                    Library &rarr; Apps
                </Link>
                .
            </p>

            {picker}
        </>
    )
}
