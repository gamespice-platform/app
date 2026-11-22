import { z } from 'zod'

import { call } from '~/lib/ipc'

import {
    InspectReportSchema,
    InstallReportSchema,
    ParsedExternalSchema,
    ResolvedExternalSchema,
    SchemeReportSchema,
} from './schemas'

/**
 * Typed wrappers for the external-mod commands. No command takes a path:
 * `inspectPath` takes the vault token minted when the user dropped the file.
 */
export const externalIpc = {
    /** Recognise a pasted link. Local, no network. */
    parse: (text: string) => call('external_parse', ParsedExternalSchema, { text }),

    /** Look the link up at its provider and suggest a game or sandbox. */
    resolve: (text: string, opts?: { loader?: string; gameVersion?: string }) =>
        call('external_resolve', ResolvedExternalSchema, {
            text,
            loader: opts?.loader,
            gameVersion: opts?.gameVersion,
        }),

    /** Download, verify and register. Call only on the user's own paste or confirm. */
    install: (
        text: string,
        opts?: {
            appId?: number
            sandboxId?: number
            loader?: string
            gameVersion?: string
        }
    ) =>
        call('external_install', InstallReportSchema, {
            request: { text, ...opts },
        }),

    /** Remove a mod that was added from a link. Resolves to sandboxes needing a redeploy. */
    remove: (localId: number) =>
        call('external_remove', z.array(z.number()), { localId }),

    inspectPath: (token: string, appId?: number) =>
        call('external_inspect_path', InspectReportSchema, { token, appId }),

    schemeStatus: () => call('external_scheme_status', z.array(SchemeReportSchema)),

    /** Never replaces another app's handler unless `overrideExisting` is true. */
    schemeSet: (scheme: string, enable: boolean, overrideExisting = false) =>
        call('external_scheme_set', SchemeReportSchema, {
            scheme,
            enable,
            overrideExisting,
        }),
}

/** The event a deep link for an external scheme arrives on. */
export const EXTERNAL_LINK_EVENT = 'tmc://external-link'
