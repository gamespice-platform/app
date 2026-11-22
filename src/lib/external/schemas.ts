import { z } from 'zod'

/**
 * Shapes returned by the external-mod commands (`src-tauri/core/src/external`).
 * Mirrors the Rust serde output exactly; keep the two in step.
 */

export const ProviderIdSchema = z.enum([
    'modrinth',
    'thunderstore',
    'github',
    'direct',
    'curseforge',
    'nexus',
    'modio',
    'steam',
])
export type ProviderIdT = z.infer<typeof ProviderIdSchema>

export const CredentialStateSchema = z.discriminatedUnion('state', [
    z.object({ state: z.literal('notRequired') }),
    z.object({ state: z.literal('configured') }),
    z.object({ state: z.literal('notConfigured'), reason: z.string() }),
])
export type CredentialStateT = z.infer<typeof CredentialStateSchema>

export const ExternalRefSchema = z.object({
    provider: ProviderIdSchema,
    project: z.string().nullable(),
    version: z.string().nullable(),
    game: z.string().nullable(),
    class: z.string().nullable(),
    url: z.string(),
    extras: z.record(z.string(), z.string()).default({}),
})
export type ExternalRefT = z.infer<typeof ExternalRefSchema>

export const ParsedExternalSchema = z.object({
    reference: ExternalRefSchema,
    provider: z.object({
        provider: ProviderIdSchema,
        label: z.string(),
        state: CredentialStateSchema,
    }),
})
export type ParsedExternalT = z.infer<typeof ParsedExternalSchema>

const HashesSchema = z.object({
    sha512: z.string().nullable(),
    sha256: z.string().nullable(),
    sha1: z.string().nullable(),
    md5: z.string().nullable(),
})

export const ResolvedSchema = z.object({
    provider: ProviderIdSchema,
    projectId: z.string(),
    versionId: z.string(),
    title: z.string(),
    version: z.string(),
    author: z.string().nullable(),
    game: z.object({
        game: z.string().nullable(),
        loaders: z.array(z.string()),
        gameVersions: z.array(z.string()),
    }),
    files: z.array(
        z.object({
            url: z.string(),
            name: z.string(),
            size: z.number().nullable(),
            hashes: HashesSchema,
            primary: z.boolean(),
        })
    ),
    deps: z.array(
        z.object({
            project: z.string(),
            version: z.string().nullable(),
            kind: z.enum(['required', 'optional', 'incompatible', 'embedded']),
        })
    ),
    pageUrl: z.string().nullable(),
    /** Set when the app cannot download this itself: open this page instead. */
    manualOnly: z.string().nullable(),
    licence: z.string().nullable(),
})
export type ResolvedT = z.infer<typeof ResolvedSchema>

export const CandidateSchema = z.object({
    appId: z.number(),
    gameName: z.string(),
    sandboxId: z.number().nullable(),
    sandboxName: z.string().nullable(),
    score: z.number(),
    warnings: z.array(z.string()),
})
export type CandidateT = z.infer<typeof CandidateSchema>

export const ResolvedExternalSchema = z.object({
    resolved: ResolvedSchema,
    candidates: z.array(CandidateSchema),
})

export const InstalledSchema = z.object({
    localId: z.number(),
    name: z.string(),
    verified: z.enum(['sha512', 'sha256', 'size', 'none']),
    files: z.number(),
})

export const AddOutcomeSchema = z.discriminatedUnion('kind', [
    InstalledSchema.extend({ kind: z.literal('installed') }),
    z.object({ kind: z.literal('openPage'), url: z.string(), reason: z.string() }),
    z.object({
        kind: z.literal('unavailable'),
        provider: ProviderIdSchema,
        reason: z.string(),
    }),
])
export type AddOutcomeT = z.infer<typeof AddOutcomeSchema>

export const InstallReportSchema = z.object({
    outcome: AddOutcomeSchema,
    sandboxId: z.number().nullable(),
})

const MetaFormatSchema = z.enum([
    'modrinthIndex',
    'fabric',
    'quilt',
    'forgeToml',
    'neoforgeToml',
    'mcmodInfo',
    'thunderstore',
    'curseforgeManifest',
    'prism',
    'plugin',
])

export const ExternalMetaSchema = z.object({
    format: MetaFormatSchema.nullable(),
    id: z.string().nullable(),
    name: z.string().nullable(),
    version: z.string().nullable(),
    authors: z.array(z.string()),
    description: z.string().nullable(),
    game: z.string().nullable(),
    loader: z.string().nullable(),
    gameVersion: z.string().nullable(),
    deps: z.array(z.string()),
    isPack: z.boolean(),
})
export type ExternalMetaT = z.infer<typeof ExternalMetaSchema>

export const FileTreeSchema = z.object({
    nodes: z.array(
        z.object({
            path: z.string(),
            name: z.string(),
            isDir: z.boolean(),
            size: z.number(),
            depth: z.number(),
        })
    ),
    suggested: z.array(z.string()),
    totalFiles: z.number(),
    totalBytes: z.number(),
    truncated: z.boolean(),
})
export type FileTreeT = z.infer<typeof FileTreeSchema>

export const InspectionSchema = z.discriminatedUnion('kind', [
    z.object({
        kind: z.literal('found'),
        meta: ExternalMetaSchema,
        file: z.string(),
    }),
    z.object({ kind: z.literal('needsSelection'), tree: FileTreeSchema }),
])

export const InspectReportSchema = z.object({
    inspection: InspectionSchema,
    candidates: z.array(CandidateSchema),
})

/** Payload of the `tmc://external-link` event: a link waiting for the user's yes. */
export const PendingSchema = z.discriminatedUnion('kind', [
    z.object({
        kind: z.literal('confirm'),
        provider: ProviderIdSchema,
        label: z.string(),
        url: z.string(),
        reference: ExternalRefSchema,
    }),
    z.object({ kind: z.literal('rejected'), url: z.string(), reason: z.string() }),
])
export type PendingT = z.infer<typeof PendingSchema>

export const SchemeStatusSchema = z.discriminatedUnion('state', [
    z.object({ state: z.literal('registered'), by: z.string(), ours: z.boolean() }),
    z.object({ state: z.literal('free') }),
    z.object({ state: z.literal('unknown') }),
])
export type SchemeStatusT = z.infer<typeof SchemeStatusSchema>

export const SchemeReportSchema = z.object({
    scheme: z.string(),
    status: SchemeStatusSchema,
})

/** Where a provider's credential comes from. Never the credential itself. */
export const CredentialSourceSchema = z.enum(['user', 'site', 'none'])
export const CredentialBackendSchema = z.enum(['keychain', 'encryptedFile', 'memory'])

export const ProviderCredentialSchema = z.object({
    provider: ProviderIdSchema,
    label: z.string(),
    configured: z.boolean(),
    source: CredentialSourceSchema,
    acceptsUserKey: z.boolean(),
    note: z.string(),
})

export const CredentialReportSchema = z.object({
    backend: CredentialBackendSchema,
    /** True when the OS keychain is not in use (encrypted-file mode). */
    fallback: z.boolean(),
    providers: z.array(ProviderCredentialSchema),
})
export type CredentialReportT = z.infer<typeof CredentialReportSchema>
