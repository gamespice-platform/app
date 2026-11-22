import labels from '../../../schemas/step-labels.en.json'

/**
 * The one table of human labels per install step action. It is A1's
 * `schemas/step-labels.en.json`, synced by `catalog:schema:sync`, so the app
 * and the website's manifest panel say the same thing for the same step.
 */
const LABELS: Record<string, string> = labels

/** "Running the installer" for `runInstaller`; the action itself when unknown. */
export function stepLabel(action: string): string {
    return LABELS[action] ?? action
}

export function StepLabel({ action }: { action: string }) {
    return (
        <span className="font-medium" title={action}>
            {stepLabel(action)}
        </span>
    )
}
