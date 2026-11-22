import Select from '~/components/select'
import type { GameLoaderWireT } from '~/lib/api/contract'
import { findLoader, loaderOptions } from '~/lib/catalog/picker'
import { ct } from '~/lib/catalog/text'

/**
 * The loader, "None (vanilla)" first, from the game's own catalog. A value
 * typed before the catalog existed and matching no key stays selected as
 * "Unknown loader", so nothing the user had is dropped.
 */
export default function LoaderPicker({
    loaders,
    value,
    onChange,
    disabled,
}: {
    loaders: GameLoaderWireT[]
    value: string | null
    onChange: (next: string | null) => void
    disabled?: boolean
}) {
    // `Forge` stored by hand still selects the `forge` row.
    const selected = findLoader(loaders, value)?.key ?? value ?? ''

    return (
        <Select
            label={ct('catalog.loader.label')}
            fullWidth
            disabled={disabled}
            value={selected}
            options={loaderOptions(loaders, value)}
            onChange={(next) => onChange(next || null)}
        />
    )
}
