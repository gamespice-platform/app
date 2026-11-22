/**
 * What the importer accepts, in one place so every screen that names formats
 * says the same thing. Rust decides for real (`is_importable`, the game's own
 * install rules); this is only the wording.
 */

/** Archives that are unpacked when the game's rules do not take them as files. */
export const ARCHIVE_EXTENSIONS = ['.zip', '.7z', '.tar', '.tar.gz', '.tgz'] as const

/** Modpacks, recognised by their manifest rather than their name. */
export const PACK_EXTENSIONS = ['.mrpack'] as const

/** `.zip, .7z, .tar, .tar.gz, .tgz`, for a sentence. */
export function archiveList(): string {
    return ARCHIVE_EXTENSIONS.join(', ')
}

/** Whether a file name looks like an archive or modpack the importer unpacks. */
export function looksLikeArchive(name: string): boolean {
    const lower = name.toLowerCase()

    return [...ARCHIVE_EXTENSIONS, ...PACK_EXTENSIONS].some((ext) =>
        lower.endsWith(ext)
    )
}
