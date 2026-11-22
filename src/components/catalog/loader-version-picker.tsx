import Select from '~/components/select'
import type { LoaderVersionWireT } from '~/lib/api/contract'
import { checkCustomVersion, loaderVersionOptions } from '~/lib/catalog/picker'
import { ct } from '~/lib/catalog/text'

/**
 * A loader version, already narrowed to the chosen game version's builds by
 * `catalog_loader_versions`. "Custom…" takes a build the list lacks.
 */
export default function LoaderVersionPicker({
    items,
    value,
    onChange,
    loading,
    disabled,
}: {
    items: LoaderVersionWireT[]
    value: string | null
    onChange: (next: string | null) => void
    loading?: boolean
    disabled?: boolean
}) {
    return (
        <Select
            label={ct('catalog.loaderVersion.label')}
            fullWidth
            disabled={disabled || loading}
            value={value ?? ''}
            placeholder={loading ? ct('catalog.loaderVersion.loading') : undefined}
            options={loaderVersionOptions(items, value)}
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
