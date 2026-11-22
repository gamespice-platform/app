/*
 * `npm run manifests:check [-- --release]` (docs/loader-manifests.md).
 *
 * For every app folder under manifests/ it checks:
 * - each `*.manifest.json` against schemas/loader-manifest.v1.schema.json,
 *   then the lint rules the schema cannot express (lintManifest below);
 * - that each file is the pretty form of its own canonical form;
 * - that the file sits where its id says;
 * - each `.sig` against manifests/keys/exec-keys.json;
 * - catalog.snapshot.json against schemas/game-catalog.v1.schema.json;
 * - that schemas/step-labels.en.json labels every step action, and no other;
 * - the lint fixtures under schemas/fixtures/lint: every ok/ file passes and
 *   every bad/ file fails with exactly the code its name gives.
 *
 * `--release` also fails on every `x-tmc-placeholder` (manifests, signatures
 * and keys) and on the test key id, listing each one.
 *
 * Problems carry a stable `code`. The lint fixtures are named after it
 * (`schemas/fixtures/lint/bad/<code>[--<case>].json`), so the website's and
 * Rust's linters can share them.
 */

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

import Ajv2020Module from 'ajv/dist/2020.js'

import { CanonicalError, canonicalString, parse, prettyString } from './canonical.ts'
import { type ExecKey, TEST_KEY_ID, execKeysProblems, verifySignature } from './sign.ts'

export type Problem = { file?: string; code: string; where: string; message: string }

export const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..')

const IDENT = /^[A-Za-z][A-Za-z0-9]{0,31}$/
const KEY = /^[a-z0-9-]{1,32}$/
/** Anything that looks like a name in braces is a placeholder and must be known. */
const TOKEN = /\{([A-Za-z][A-Za-z0-9_.-]*)\}/g

type Json = null | boolean | number | string | Json[] | { [k: string]: Json }
type Obj = { [k: string]: Json }
type Validate = ((doc: unknown) => boolean) & {
    errors?: { instancePath: string; schemaPath: string; message?: string; params?: unknown }[] | null
}

// ajv is CommonJS: under Node's ESM loader the class is `.default`.
const Ajv2020 = ((Ajv2020Module as unknown as { default?: unknown }).default ??
    Ajv2020Module) as new (opts: object) => {
    compile: (schema: object) => Validate
    getSchema: (ref: string) => Validate | undefined
}

export type Schemas = {
    manifest: Validate
    /** One validator per step action: `step` is a oneOf, whose errors list every branch. */
    steps: Map<string, Validate>
    catalog: Validate
    manifestSchema: Obj
}

export function loadSchemas(root = ROOT): Schemas {
    const read = (f: string) => parse(fs.readFileSync(path.join(root, 'schemas', f), 'utf8')) as Obj
    const manifestSchema = read('loader-manifest.v1.schema.json')
    const ajv = new Ajv2020({ allErrors: true, strictTypes: false, strictTuples: false })
    const manifest = ajv.compile(manifestSchema)
    const steps = new Map<string, Validate>()
    for (const [def, action] of stepDefs(manifestSchema)) {
        const v = ajv.getSchema(`${String(manifestSchema.$id)}#/$defs/${def}`)
        if (v) steps.set(action, v)
    }
    return { manifest, steps, catalog: ajv.compile(read('game-catalog.v1.schema.json')), manifestSchema }
}

function stepDefs(manifestSchema: Obj): [string, string][] {
    const defs = manifestSchema.$defs as Record<string, Obj>
    return Object.keys(defs)
        .filter((k) => /^step[A-Z]/.test(k))
        .map((k): [string, unknown] => [k, ((defs[k].properties as Obj | undefined)?.action as Obj | undefined)?.const])
        .filter((e): e is [string, string] => typeof e[1] === 'string')
}

/** Every step action the schema declares (its `step*` definitions' `action` const). */
export function stepActions(manifestSchema: Obj): string[] {
    return stepDefs(manifestSchema).map(([, action]) => action)
}

function isObj(v: unknown): v is Obj {
    return typeof v === 'object' && v !== null && !Array.isArray(v)
}

function ptr(parts: (string | number)[]): string {
    return parts.length
        ? '/' + parts.map((p) => String(p).replace(/~/g, '~0').replace(/\//g, '~1')).join('/')
        : ''
}

type AjvError = NonNullable<Validate['errors']>[number]

/**
 * Schema errors, readable. A step that fails reports every `oneOf` branch, so
 * a step whose `action` names a known branch is re-validated against that
 * branch alone and only its errors are kept.
 */
export function schemaProblems(validate: Validate, doc: unknown, steps?: Map<string, Validate>): Problem[] {
    if (validate(doc)) return []
    const list = isObj(doc) && Array.isArray(doc.steps) ? doc.steps : []
    const errors: AjvError[] = []
    const redone = new Set<number>()
    for (const e of validate.errors ?? []) {
        const m = /^\/steps\/(\d+)(\/|$)/.exec(e.instancePath)
        const i = m ? Number(m[1]) : -1
        const step = list[i]
        const branch = isObj(step) && typeof step.action === 'string' ? steps?.get(step.action) : undefined
        if (!branch) {
            errors.push(e)
        } else if (!redone.has(i)) {
            redone.add(i)
            branch(step)
            for (const b of branch.errors ?? []) errors.push({ ...b, instancePath: `/steps/${i}${b.instancePath}` })
        }
    }
    const seen = new Set<string>()
    const out: Problem[] = []
    for (const e of errors) {
        const extra = isObj(e.params)
            ? ['additionalProperty', 'unevaluatedProperty', 'missingProperty', 'allowedValues', 'allowedValue']
                  .filter((k) => k in (e.params as Obj))
                  .map((k) => ` (${JSON.stringify((e.params as Obj)[k])})`)
                  .join('')
            : ''
        const message = `${e.message ?? 'invalid'}${extra}`
        const key = `${e.instancePath} ${message}`
        if (seen.has(key)) continue
        seen.add(key)
        out.push({ code: 'schema', where: e.instancePath, message })
    }
    return out
}

export type LintContext = {
    /** Ids of the manifests that exist for this app (for `requires`). */
    knownIds: Set<string>
    /** The parsed `.sig`, or undefined when there is none. */
    sig?: unknown
    keys: ExecKey[]
}

function hostAllowed(host: string, hosts: string[]): boolean {
    return hosts.some((h) =>
        h.startsWith('*.') ? host.endsWith(h.slice(1)) && host.length > h.length - 1 : host === h,
    )
}

/** The lint rules the schema cannot express. Run on a schema-valid manifest. */
export function lintManifest(doc: Obj, ctx: LintContext): Problem[] {
    const out: Problem[] = []
    const add = (code: string, where: (string | number)[], message: string) =>
        out.push({ code, where: ptr(where), message })

    const app = typeof doc.app === 'string' ? doc.app : ''
    const isLoader = doc.kind === 'loader'
    const artifacts = isObj(doc.artifacts) ? doc.artifacts : {}
    const requirements = isObj(doc.requirements) ? doc.requirements : {}
    const runtimes = new Set(
        (Array.isArray(requirements.runtimes) ? requirements.runtimes : [])
            .map((r) => (isObj(r) ? r.kind : null))
            .filter((k): k is string => typeof k === 'string'),
    )
    const steps = Array.isArray(doc.steps) ? doc.steps : []
    const hosts = (Array.isArray(doc.downloadHosts) ? doc.downloadHosts : []).filter(
        (h): h is string => typeof h === 'string',
    )
    const executed = new Map<string, (string | number)[]>()

    // Placeholders: the closed set, each in its allowed positions.
    const scan = (s: string, at: (string | number)[]) => {
        const step = at[0] === 'steps' ? steps[at[1] as number] : undefined
        const installerField =
            isObj(step) && step.action === 'runInstaller' ? String(at[2]) : undefined
        const inPatch = at[0] === 'deployPatches' && at[2] !== 'path' && at[2] !== 'when'
        for (const m of s.matchAll(TOKEN)) {
            const name = m[1]
            const [head, rest, ...more] = name.split('.')
            const position = (where: string) =>
                add(
                    'placeholder-position',
                    at,
                    `{${name}} is only allowed in ${where}`,
                )
            if (more.length || (rest === undefined && name.includes('.'))) {
                add('placeholder-unknown', at, `{${name}} is not in the closed set`)
            } else if (
                rest === undefined &&
                ['gameVersion', 'platform', 'side', 'launchVersionId'].includes(head)
            ) {
                // everywhere
            } else if (head === 'var' && rest !== undefined && IDENT.test(rest)) {
                // everywhere
            } else if (rest === undefined && (head === 'loader' || head === 'loaderVersion')) {
                if (!isLoader) position('loader manifests')
            } else if (head === 'artifact' && rest !== undefined && IDENT.test(rest)) {
                if (installerField !== 'args' && installerField !== 'program') {
                    position('runInstaller program and args')
                } else if (!isObj(artifacts[rest])) {
                    add('artifact-undeclared', at, `{${name}}: no artifact "${rest}" is declared`)
                } else {
                    executed.set(rest, at)
                }
            } else if (head === 'runtime' && rest !== undefined && KEY.test(rest)) {
                if (installerField !== 'program') position('runInstaller program')
                else if (!runtimes.has(rest)) {
                    add('runtime-undeclared', at, `{${name}}: "${rest}" is not in requirements.runtimes`)
                }
            } else if (rest === undefined && (head === 'layerFiles' || head === 'layerWork')) {
                if (installerField !== 'args' && installerField !== 'cwd') {
                    position('runInstaller args and cwd')
                }
            } else if (
                rest === undefined &&
                ['sandboxId', 'sandboxName', 'targetDir'].includes(head)
            ) {
                if (!inPatch) position('deployPatches')
            } else {
                add('placeholder-unknown', at, `{${name}} is not in the closed set`)
            }
        }
    }
    const walk = (v: Json, at: (string | number)[]) => {
        if (typeof v === 'string') scan(v, at)
        else if (Array.isArray(v)) v.forEach((x, i) => walk(x, [...at, i]))
        else if (isObj(v)) {
            for (const [k, x] of Object.entries(v)) {
                if (k === 'x-tmc-placeholder' || (at.length === 0 && k === '$schema')) continue
                walk(x, [...at, k])
            }
        }
    }
    walk(doc, [])

    // Artifact references, and the hosts of every fixed URL.
    const checkHost = (url: Json, at: (string | number)[]) => {
        if (typeof url !== 'string') return
        const authority = /^https:\/\/([^/?#]*)/.exec(url)?.[1] ?? ''
        const host = authority.toLowerCase()
        if (!/^[a-z0-9.-]+$/.test(host)) {
            add('host-not-allowed', at, `the host of ${url} must be a literal host name`)
        } else if (!hostAllowed(host, hosts)) {
            add('host-not-allowed', at, `${host} is not in downloadHosts`)
        }
    }
    for (const [key, decl] of Object.entries(artifacts)) {
        if (isObj(decl)) checkHost(decl.urlTemplate, ['artifacts', key, 'urlTemplate'])
    }
    steps.forEach((step, i) => {
        if (!isObj(step)) return
        if (typeof step.artifact === 'string' && !isObj(artifacts[step.artifact])) {
            add('artifact-undeclared', ['steps', i, 'artifact'], `no artifact "${step.artifact}" is declared`)
        }
        checkHost(step.url, ['steps', i, 'url'])
        if (Array.isArray(step.mirrors)) step.mirrors.forEach((m, j) => checkHost(m, ['steps', i, 'mirrors', j]))
    })

    // An executed artifact's URL is fixed by the (signed) template.
    const exec = steps.some((s) => isObj(s) && s.action === 'runInstaller')
    for (const [key, at] of executed) {
        const decl = artifacts[key]
        if (exec && isObj(decl) && typeof decl.urlTemplate !== 'string') {
            add(
                'exec-artifact-no-url-template',
                at,
                `artifact "${key}" is executed, so it must declare its URL as urlTemplate`,
            )
        }
    }

    if (isLoader && doc.id !== `${app}/loader/${String(doc.loader)}`) {
        add('loader-id-mismatch', ['id'], `a loader's id must be ${app}/loader/${String(doc.loader)}`)
    }

    ;(Array.isArray(doc.requires) ? doc.requires : []).forEach((r, i) => {
        if (typeof r !== 'string') return
        if (!r.startsWith(`${app}/`)) {
            add('requires-other-app', ['requires', i], `${r} belongs to another app than ${app}`)
        } else if (!ctx.knownIds.has(r)) {
            add('requires-unknown', ['requires', i], `no manifest ${r} exists`)
        }
    })

    if (exec || ctx.sig !== undefined) {
        if (ctx.sig === undefined) {
            add('signature-missing', [], 'a manifest with runInstaller needs a .sig next to it')
        } else {
            const bad = verifySignature(doc, ctx.sig, ctx.keys)
            if (bad) add(bad.code, [], bad.message)
        }
    }
    return out
}

/** Every `x-tmc-placeholder` in a document, with where it is. */
export function placeholderMarkers(doc: unknown): { where: string; note: string }[] {
    const out: { where: string; note: string }[] = []
    const walk = (v: unknown, at: (string | number)[]) => {
        if (Array.isArray(v)) v.forEach((x, i) => walk(x, [...at, i]))
        else if (isObj(v)) {
            for (const [k, x] of Object.entries(v)) {
                if (k === 'x-tmc-placeholder') out.push({ where: ptr([...at, k]), note: String(x) })
                else walk(x, [...at, k])
            }
        }
    }
    walk(doc, [])
    return out
}

function readJson(file: string, problems: Problem[], rel: string): unknown {
    let text: string
    let doc: unknown
    try {
        text = fs.readFileSync(file, 'utf8')
        doc = parse(text)
    } catch (e) {
        problems.push({ file: rel, code: 'json', where: '', message: (e as Error).message })
        return undefined
    }
    try {
        canonicalString(doc)
        if (prettyString(doc) !== text) {
            problems.push({
                file: rel,
                code: 'not-pretty',
                where: '',
                message:
                    'not the pretty form of its canonical form ($schema first, then keys sorted by code point, 4-space indent, trailing newline)',
            })
        }
    } catch (e) {
        if (!(e instanceof CanonicalError)) throw e
        problems.push({ file: rel, code: 'canonical', where: '', message: e.message })
    }
    return doc
}

function listManifests(dir: string): string[] {
    const out: string[] = []
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
        const full = path.join(dir, entry.name)
        if (entry.isDirectory()) out.push(...listManifests(full))
        else if (entry.name.endsWith('.manifest.json')) out.push(full)
    }
    return out.sort()
}

function listSigs(dir: string): string[] {
    const out: string[] = []
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
        const full = path.join(dir, entry.name)
        if (entry.isDirectory()) out.push(...listSigs(full))
        else if (entry.name.endsWith('.sig')) out.push(full)
    }
    return out
}

export const FIXTURES = 'schemas/fixtures/lint'

export type FixtureResult = {
    file: string
    /** The code a bad fixture must produce (its file name up to `--`); null for ok/. */
    expected: string | null
    problems: Problem[]
}

/**
 * Lints every fixture under schemas/fixtures/lint/: `ok/*.json` must pass,
 * `bad/<code>[--<case>].json` must fail with `<code>` and nothing else.
 * `requires` resolves against the ids of ok/, signatures against
 * lint/keys/exec-keys.json.
 */
export function lintFixtures(root = ROOT, schemas = loadSchemas(root)): FixtureResult[] {
    const dir = path.join(root, FIXTURES)
    const keys = parse(fs.readFileSync(path.join(dir, 'keys/exec-keys.json'), 'utf8')) as ExecKey[]
    const list = (sub: string) =>
        fs
            .readdirSync(path.join(dir, sub))
            .filter((f) => f.endsWith('.json'))
            .sort()
            .map((f) => path.join(dir, sub, f))
    const ok = list('ok')
    const bad = list('bad')
    const knownIds = new Set(
        ok.map((f) => (parse(fs.readFileSync(f, 'utf8')) as Obj).id).filter((i): i is string => typeof i === 'string'),
    )
    return [...ok, ...bad].map((file) => {
        const rel = path.relative(root, file)
        const problems: Problem[] = []
        const doc = readJson(file, problems, rel)
        if (!problems.length) problems.push(...schemaProblems(schemas.manifest, doc, schemas.steps))
        if (!problems.length && isObj(doc)) {
            let sig: unknown
            if (fs.existsSync(`${file}.sig`)) sig = parse(fs.readFileSync(`${file}.sig`, 'utf8'))
            problems.push(...lintManifest(doc, { knownIds, sig, keys }))
        }
        const expected = file.startsWith(path.join(dir, 'bad')) ? path.basename(file, '.json').split('--')[0] : null
        return { file: rel, expected, problems: problems.map((p) => ({ ...p, file: rel })) }
    })
}

/** A fixture's problems, as a check failure: an ok fixture failed, or a bad one did not fail as named. */
export function fixtureProblems(results: FixtureResult[]): Problem[] {
    const out: Problem[] = []
    for (const r of results) {
        const codes = [...new Set(r.problems.map((p) => p.code))]
        const wrong = r.expected === null ? codes.length > 0 : codes.length !== 1 || codes[0] !== r.expected
        if (wrong) {
            out.push({
                file: r.file,
                code: 'fixture',
                where: '',
                message:
                    `expected ${r.expected === null ? 'no problems' : `only [${r.expected}]`}, got ` +
                    (r.problems.length ? r.problems.map((p) => `[${p.code}] ${p.where} ${p.message}`).join('; ') : 'none'),
            })
        }
    }
    return out
}

export type CheckResult = { problems: Problem[]; markers: Problem[]; files: number }

/** The whole repository check. `release` turns markers and the test key into problems. */
export function checkRepository(root = ROOT, release = false): CheckResult {
    const problems: Problem[] = []
    const markers: Problem[] = []
    const rel = (f: string) => path.relative(root, f)
    const schemas = loadSchemas(root)
    const manifestsDir = path.join(root, 'manifests')
    let files = 0

    const noteMarkers = (doc: unknown, file: string) => {
        for (const m of placeholderMarkers(doc)) {
            markers.push({ file, code: 'placeholder-marker', where: m.where, message: m.note })
        }
    }

    const keysFile = path.join(manifestsDir, 'keys/exec-keys.json')
    const keys = (readJson(keysFile, problems, rel(keysFile)) ?? []) as ExecKey[]
    files++
    for (const m of execKeysProblems(keys)) {
        problems.push({ file: rel(keysFile), code: 'exec-keys', where: '', message: m })
    }
    noteMarkers(keys, rel(keysFile))
    if (Array.isArray(keys) && keys.some((k) => k?.keyId === TEST_KEY_ID)) {
        markers.push({
            file: rel(keysFile),
            code: 'test-key',
            where: '',
            message: `${TEST_KEY_ID} is listed; a release must carry only the owner's exec keys`,
        })
    }

    const labelsFile = path.join(root, 'schemas/step-labels.en.json')
    const labels = readJson(labelsFile, problems, rel(labelsFile))
    files++
    const actions = stepActions(schemas.manifestSchema).sort()
    const labelled = isObj(labels) ? Object.keys(labels).filter((k) => k !== '$schema').sort() : []
    if (JSON.stringify(actions) !== JSON.stringify(labelled)) {
        problems.push({
            file: rel(labelsFile),
            code: 'step-labels',
            where: '',
            message: `must label exactly the step actions ${actions.join(', ')}`,
        })
    }

    const apps = fs
        .readdirSync(manifestsDir, { withFileTypes: true })
        .filter((d) => d.isDirectory() && d.name !== 'keys' && d.name !== 'runtimes')
        .map((d) => d.name)
        .sort()
    for (const app of apps) {
        const appDir = path.join(manifestsDir, app)
        const loaded = listManifests(appDir).map((file) => {
            files++
            return { file, doc: readJson(file, problems, rel(file)) }
        })
        const knownIds = new Set(
            loaded.map((m) => (isObj(m.doc) ? m.doc.id : null)).filter((i): i is string => typeof i === 'string'),
        )
        for (const { file, doc } of loaded) {
            if (doc === undefined) continue
            const f = rel(file)
            noteMarkers(doc, f)
            const schemaErrors = schemaProblems(schemas.manifest, doc, schemas.steps)
            problems.push(...schemaErrors.map((p) => ({ ...p, file: f })))
            if (schemaErrors.length || !isObj(doc)) continue
            const relInApp = path.relative(appDir, file).split(path.sep).join('/')
            const expected =
                relInApp === 'version.manifest.json'
                    ? `${app}/version`
                    : /^loaders\/([^/]+)\.manifest\.json$/.exec(relInApp)?.[1]
                      ? `${app}/loader/${/^loaders\/([^/]+)\.manifest\.json$/.exec(relInApp)?.[1]}`
                      : null
            if (doc.app !== app || doc.id !== expected) {
                problems.push({
                    file: f,
                    code: 'file-id-mismatch',
                    where: '/id',
                    message: expected
                        ? `this file must hold ${expected} (app ${app})`
                        : 'manifests live at <app>/version.manifest.json or <app>/loaders/<loader>.manifest.json',
                })
            }
            let sig: unknown
            if (fs.existsSync(`${file}.sig`)) {
                files++
                sig = readJson(`${file}.sig`, problems, `${f}.sig`)
                noteMarkers(sig, `${f}.sig`)
                if (isObj(sig) && sig.keyId === TEST_KEY_ID) {
                    markers.push({
                        file: `${f}.sig`,
                        code: 'test-key',
                        where: '/keyId',
                        message: `signed by ${TEST_KEY_ID}; re-sign with the owner's exec key`,
                    })
                }
            }
            const lint = lintManifest(doc, { knownIds, sig, keys: Array.isArray(keys) ? keys : [] })
            problems.push(...lint.map((p) => ({ ...p, file: f })))
        }
        for (const sigFile of listSigs(appDir)) {
            if (!fs.existsSync(sigFile.slice(0, -'.sig'.length)) || !sigFile.endsWith('.manifest.json.sig')) {
                problems.push({ file: rel(sigFile), code: 'signature-orphan', where: '', message: 'no manifest next to this .sig' })
            }
        }
        const snapshotFile = path.join(appDir, 'catalog.snapshot.json')
        if (fs.existsSync(snapshotFile)) {
            files++
            const snap = readJson(snapshotFile, problems, rel(snapshotFile))
            if (snap !== undefined) {
                problems.push(...schemaProblems(schemas.catalog, snap).map((p) => ({ ...p, file: rel(snapshotFile) })))
                const slug = isObj(snap) && isObj(snap.app) ? snap.app.slug : undefined
                if (slug !== app) {
                    problems.push({ file: rel(snapshotFile), code: 'file-id-mismatch', where: '/app/slug', message: `must be ${app}` })
                }
            }
        }
    }
    const fixtures = lintFixtures(root, schemas)
    files += fixtures.length
    problems.push(...fixtureProblems(fixtures))
    return { problems: release ? [...problems, ...markers] : problems, markers, files }
}

function main(argv: string[]): number {
    const release = argv.includes('--release')
    const { problems, markers, files } = checkRepository(ROOT, release)
    for (const p of problems) {
        console.error(`${p.file}${p.where ? ` ${p.where}` : ''}: [${p.code}] ${p.message}`)
    }
    if (problems.length) {
        console.error(`manifests:check${release ? ' --release' : ''}: ${problems.length} problem(s) in ${files} files`)
        return 1
    }
    console.log(
        `manifests:check: ${files} files OK` +
            (markers.length ? `; ${markers.length} placeholder marker(s), which --release refuses` : ''),
    )
    return 0
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
    process.exitCode = main(process.argv.slice(2))
}
