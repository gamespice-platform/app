import Select from '~/components/select'
import type { GameVersionWireT } from '~/lib/api/contract'
import { checkCustomVersion, versionOptions } from '~/lib/catalog/picker'
import { ct } from '~/lib/catalog/text'

/**
 * A game version from the game's catalog (the last `versionListCount`,
 * newest first), with a search box and a "Custom…" row for a version the
 * list does not have. `null` is "Any version".
 */
export default function VersionPicker({
    items,
    value,
    onChange,
    disabled,
}: {
    items: GameVersionWireT[]
    value: string | null
    onChange: (next: string | null) => void
    disabled?: boolean
}) {
    return (
        <Select
            label={ct('catalog.version.label')}
            fullWidth
            disabled={disabled}
            value={value ?? ''}
            options={versionOptions(items, value)}
            onChange={(next) => onChange(next || null)}
            search={ct('catalog.version.search')}
            custom={{
                label: ct('catalog.version.customRow'),
                queryLabel: ct('catalog.version.custom'),
                check: checkCustomVersion,
                onCustom: (text) => onChange(text.trim()),
            }}
        />
    )
}
