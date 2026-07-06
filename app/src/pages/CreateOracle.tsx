import { useCallback, useEffect, useMemo, useRef, useState, type FormEvent } from 'react'
import { Link, useNavigate } from 'react-router-dom'
import { Address } from '@solana/web3.js'
import { decodeProtocol, pda } from '@kassandra/sdk'
import { useWalletModal } from '@solana/wallet-adapter-react-ui'
import { Card, EyebrowTag } from '../components/ui'
import { Field, SubmitButton, TextInput } from '../components/oracles/actions/formPrimitives'
import { WriteStatusRegion } from '../components/oracles/actions/WriteStatusRegion'
import { useWriteAction } from '../hooks/useWriteAction'
import { rememberNonce } from '../lib/nonceStore'
import { isMockMode } from '../data/mockOracles'
import { postOracleMetadata } from '../data/indexer'
import {
  buildCreateOracleIxs,
  defaultPromptTemplate,
  randomNonce,
  type CreateOracleBuild,
} from '../data/actions/create'

const selectClass =
  'w-full rounded-tag border border-pebble bg-pure-card px-3 py-2 font-inter text-[14px] ' +
  'text-sepia focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sepia/40 ' +
  'focus-visible:ring-offset-2 focus-visible:ring-offset-parchment'

const textareaClass =
  'w-full rounded-tag border border-pebble bg-pure-card px-3 py-2 font-inter text-[14px] ' +
  'text-sepia placeholder:text-driftwood focus-visible:outline-none focus-visible:ring-2 ' +
  'focus-visible:ring-sepia/40 focus-visible:ring-offset-2 focus-visible:ring-offset-parchment ' +
  'aria-[invalid=true]:border-ember-orange/60'

/** Pad to 2 digits — used by the hand-rolled datetime-local formatter. */
function pad(n: number): string {
  return n < 10 ? `0${n}` : String(n)
}

/** Format a `Date` as a `datetime-local` value (`YYYY-MM-DDTHH:mm`), local time. */
function toDatetimeLocal(d: Date): string {
  return (
    `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}` +
    `T${pad(d.getHours())}:${pad(d.getMinutes())}`
  )
}

/** A `datetime-local` string → unix SECONDS (local-time interpreted). NaN if unparseable. */
function datetimeLocalToUnix(value: string): number {
  const ms = new Date(value).getTime()
  return Number.isNaN(ms) ? NaN : Math.floor(ms / 1000)
}

// Valid-base58 KASS/USDC placeholders for the offline `?mock` render (no protocol
// on-chain) — chosen so the client-side address validation passes and the
// submitting/success states are drivable via `?mock&wallet=connected`.
const MOCK_KASS = 'So11111111111111111111111111111111111111112'
const MOCK_USDC = 'EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v'

/**
 * The create-oracle page at `/oracles/new` — a Auros form that opens a new
 * optimistic-oracle dispute. Hashes the question into the on-chain `prompt_hash`,
 * defaults the mints from the on-chain Protocol singleton (pasteable override),
 * and on a confirmed create navigates to the new oracle's detail. Gated on a
 * connected wallet; read-only browsing is unaffected.
 */
export default function CreateOracle() {
  const navigate = useNavigate()
  const mock = isMockMode()
  const builtRef = useRef<CreateOracleBuild | null>(null)

  const action = useWriteAction(() => {
    const built = builtRef.current
    if (built) {
      // Remember the (random) nonce so the finalize UI can crank this oracle
      // later — the nonce isn't stored on-chain and is beyond the PDA scan.
      rememberNonce(built.oracle.toString(), built.nonce)
      // Host the extended metadata JSON at the oracle's on-chain `uri` (served
      // once indexed, gated by uri_hash). Best-effort — never blocks navigation.
      if (built.metadata) {
        void postOracleMetadata(built.oracle.toString(), built.metadata.jsonString)
      }
      navigate(`/oracles/${built.oracle.toString()}`)
    }
  })

  // A stable, freshly-random nonce per page session (the Oracle PDA seed).
  const [nonce] = useState<bigint>(() => randomNonce())

  const [question, setQuestion] = useState('')
  // Option LABELS (min 2). These drive options_count AND are published (with the
  // subject) as a memo so the browse/detail views can show them.
  const [options, setOptions] = useState<string[]>(['Yes', 'No'])
  const setOption = (i: number, v: string) =>
    setOptions((o) => o.map((x, j) => (j === i ? v : x)))
  const addOption = () => setOptions((o) => (o.length < 12 ? [...o, ''] : o))
  const removeOption = (i: number) =>
    setOptions((o) => (o.length > 2 ? o.filter((_, j) => j !== i) : o))
  const [deadline, setDeadline] = useState(() =>
    toDatetimeLocal(new Date(Date.now() + 24 * 3600 * 1000)),
  )
  const [kassMint, setKassMint] = useState(mock ? MOCK_KASS : '')
  const [usdcMint, setUsdcMint] = useState(mock ? MOCK_USDC : '')
  const [mintsLoading, setMintsLoading] = useState(!mock)

  const [errors, setErrors] = useState<Record<string, string | undefined>>({})
  // Advanced (off-chain metadata) — collapsed by default; promptTemplate defaults
  // to a sensible value derived from the question.
  const [showAdvanced, setShowAdvanced] = useState(false)
  const [promptTemplate, setPromptTemplate] = useState('')
  const [interpretation, setInterpretation] = useState('')
  const [category, setCategory] = useState('')

  // Default the mints from the Protocol singleton (kass/usdc mints). Best-effort:
  // on any RPC/decoding failure we simply leave them blank for the user to paste.
  useEffect(() => {
    if (mock) return
    let cancelled = false
    void (async () => {
      try {
        const protocolPda = (await pda.protocol()).address
        const info = await action.connection.getAccountInfo(protocolPda)
        if (!info || info.data.length === 0) return
        const p = decodeProtocol(info.data)
        if (cancelled) return
        setKassMint((cur) => (cur ? cur : p.kassMint.toString()))
        setUsdcMint((cur) => (cur ? cur : p.usdcMint.toString()))
      } catch {
        // Leave the mints blank — the user can paste them.
      } finally {
        if (!cancelled) setMintsLoading(false)
      }
    })()
    return () => {
      cancelled = true
    }
  }, [mock, action.connection])

  const validate = useCallback((): boolean => {
    const next: Record<string, string | undefined> = {}
    if (question.trim().length === 0) next.question = 'Enter a question for the oracle.'
    if (options.length < 2) next.options = 'Provide at least 2 options.'
    else if (options.some((o) => o.trim().length === 0))
      next.options = 'Every option needs a label.'
    const unix = datetimeLocalToUnix(deadline)
    if (Number.isNaN(unix)) next.deadline = 'Pick a valid date and time.'
    else if (unix <= Math.floor(Date.now() / 1000))
      next.deadline = 'Deadline must be in the future.'
    for (const [field, value] of [
      ['kassMint', kassMint],
      ['usdcMint', usdcMint],
    ] as const) {
      if (value.trim().length === 0) next[field] = 'Required.'
      else {
        try {
          new Address(value.trim())
        } catch {
          next[field] = 'Not a valid base58 address.'
        }
      }
    }
    setErrors(next)
    return Object.values(next).every((v) => !v)
  }, [question, options, deadline, kassMint, usdcMint])

  const onSubmit = (e: FormEvent) => {
    e.preventDefault()
    if (!validate()) return
    const deadlineUnix = datetimeLocalToUnix(deadline)
    void action.run(async () => {
      const built = await buildCreateOracleIxs({
        connection: action.connection,
        nonce,
        question,
        options: options.map((o) => o.trim()),
        deadline: deadlineUnix,
        creator: action.address!,
        kassMint: kassMint.trim(),
        usdcMint: usdcMint.trim(),
        appOrigin: window.location.origin,
        promptTemplate: promptTemplate.trim() || undefined,
        interpretation: interpretation.trim() || undefined,
        category: category.trim() || undefined,
      })
      builtRef.current = built
      return built.ixs
    })
  }

  const oraclePreview = useMemo(() => nonce.toString(), [nonce])

  return (
    <main className="mx-auto max-w-[720px] px-6 py-16 md:py-20">
      <Link
        to="/oracles"
        className="inline-block font-inter text-[14px] text-sepia underline decoration-pebble underline-offset-4 hover:text-lavender-phosphor focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sepia/40 focus-visible:ring-offset-2 focus-visible:ring-offset-parchment"
      >
        ← All oracles
      </Link>

      <header className="mt-8">
        <EyebrowTag pill>Create</EyebrowTag>
        <h1 className="mt-3 font-serif text-heading font-light text-sepia">Open an oracle</h1>
        <p className="mt-3 font-inter text-[15px] text-bronze">
          Pose a question, label the options it can resolve to, and set a deadline. The question and
          labels are stored on-chain as the oracle's metadata; proposers stake KASS behind an answer.
        </p>
      </header>

      <div className="mt-10">
        {action.connected ? (
          <Card className="flex flex-col gap-5">
            <form className="flex flex-col gap-5" onSubmit={onSubmit} noValidate>
              <Field label="Question" error={errors.question}>
                {(ids) => (
                  <textarea
                    id={ids.id}
                    aria-describedby={ids.describedById}
                    aria-invalid={ids.invalid}
                    rows={3}
                    className={textareaClass}
                    placeholder="e.g. Did the SpaceX Starship reach orbit before 2027?"
                    value={question}
                    onChange={(e) => setQuestion(e.target.value)}
                  />
                )}
              </Field>


              <Field
                label="Options"
                hint="The categorical answers this oracle can resolve to (min 2). Labels are published so they show when browsing."
                error={errors.options}
              >
                {(ids) => (
                  <div id={ids.id} aria-describedby={ids.describedById} className="flex flex-col gap-2">
                    {options.map((opt, i) => (
                      <div key={i} className="flex items-center gap-2">
                        <span className="w-5 shrink-0 text-right font-inter text-[12px] tabular-nums text-driftwood">
                          {i}
                        </span>
                        <input
                          type="text"
                          value={opt}
                          onChange={(e) => setOption(i, e.target.value)}
                          placeholder={`Option ${i} label`}
                          aria-label={`Option ${i} label`}
                          className={`${selectClass} flex-1`}
                        />
                        <button
                          type="button"
                          onClick={() => removeOption(i)}
                          disabled={options.length <= 2}
                          aria-label={`Remove option ${i}`}
                          className="rounded-tag border border-pebble px-2 py-2 font-inter text-[13px] text-bronze transition-colors hover:border-driftwood hover:text-sepia disabled:cursor-not-allowed disabled:opacity-40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sepia/40 focus-visible:ring-offset-2 focus-visible:ring-offset-parchment"
                        >
                          Remove
                        </button>
                      </div>
                    ))}
                    {options.length < 12 && (
                      <button
                        type="button"
                        onClick={addOption}
                        className="self-start rounded-tag border border-pebble px-3 py-2 font-inter text-[13px] text-bronze transition-colors hover:border-driftwood hover:text-sepia focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sepia/40 focus-visible:ring-offset-2 focus-visible:ring-offset-parchment"
                      >
                        + Add option
                      </button>
                    )}
                  </div>
                )}
              </Field>

              <Field label="Deadline" hint="When proposing opens (your local time)." error={errors.deadline}>
                {(ids) => (
                  <input
                    id={ids.id}
                    type="datetime-local"
                    aria-describedby={ids.describedById}
                    aria-invalid={ids.invalid}
                    className={selectClass}
                    value={deadline}
                    onChange={(e) => setDeadline(e.target.value)}
                  />
                )}
              </Field>

              <Field
                label="KASS mint"
                hint={mintsLoading ? 'Loading protocol default…' : 'Defaults to the protocol KASS mint.'}
                error={errors.kassMint}
              >
                {(ids) => (
                  <TextInput
                    ids={ids}
                    className="font-mono text-[12px]"
                    placeholder="KASS mint address"
                    value={kassMint}
                    onChange={(e) => setKassMint(e.target.value)}
                  />
                )}
              </Field>

              <Field
                label="USDC mint"
                hint={mintsLoading ? 'Loading protocol default…' : 'Defaults to the protocol USDC mint.'}
                error={errors.usdcMint}
              >
                {(ids) => (
                  <TextInput
                    ids={ids}
                    className="font-mono text-[12px]"
                    placeholder="USDC mint address"
                    value={usdcMint}
                    onChange={(e) => setUsdcMint(e.target.value)}
                  />
                )}
              </Field>

              <p className="font-inter text-[12px] text-driftwood">
                Nonce <span className="font-mono text-bronze">{oraclePreview}</span> — the oracle's
                on-chain address is derived from it.
              </p>

              {/* Advanced — extended off-chain metadata (hosted JSON, bound by an
                  on-chain hash). All optional; promptTemplate defaults from the question. */}
              <div className="border-t border-pebble pt-4">
                <button
                  type="button"
                  onClick={() => setShowAdvanced((v) => !v)}
                  className="font-inter text-[13px] text-sepia underline decoration-pebble underline-offset-4 hover:text-lavender-phosphor"
                  aria-expanded={showAdvanced}
                >
                  {showAdvanced ? '− Hide advanced' : '+ Advanced (resolution rules)'}
                </button>

                {showAdvanced && (
                  <div className="mt-4 flex flex-col gap-5">
                    <Field
                      label="Prompt template (AI-runner interpretation)"
                      hint="How the AI runner should interpret + resolve the question. Defaults from the question when blank."
                    >
                      {(ids) => (
                        <textarea
                          id={ids.id}
                          aria-describedby={ids.describedById}
                          rows={3}
                          className={textareaClass}
                          placeholder={defaultPromptTemplate(question.trim() || 'the question')}
                          value={promptTemplate}
                          onChange={(e) => setPromptTemplate(e.target.value)}
                        />
                      )}
                    </Field>

                    <Field
                      label="Interpretation (human resolution rules)"
                      hint="Optional plain-language rules a human reviewer would apply."
                    >
                      {(ids) => (
                        <textarea
                          id={ids.id}
                          aria-describedby={ids.describedById}
                          rows={2}
                          className={textareaClass}
                          placeholder="e.g. YES only if an official source confirms it before the deadline."
                          value={interpretation}
                          onChange={(e) => setInterpretation(e.target.value)}
                        />
                      )}
                    </Field>

                    <Field label="Category" hint="Optional tag (e.g. Crypto, Sports, Politics).">
                      {(ids) => (
                        <TextInput
                          ids={ids}
                          placeholder="Crypto"
                          value={category}
                          onChange={(e) => setCategory(e.target.value)}
                        />
                      )}
                    </Field>
                  </div>
                )}
              </div>

              <div className="flex items-center gap-3">
                <SubmitButton verb="Create oracle" status={action.status} />
              </div>
              <WriteStatusRegion status={action.status} successVerb="Created" />
            </form>
          </Card>
        ) : (
          <ConnectPrompt />
        )}
      </div>
    </main>
  )
}

/** Disconnected gate with copy tailored to creating an oracle. */
function ConnectPrompt() {
  const { setVisible } = useWalletModal()
  return (
    <Card className="flex flex-wrap items-center gap-3">
      <p className="font-inter text-[15px] text-driftwood">Connect a wallet to create an oracle.</p>
      <button
        type="button"
        onClick={() => setVisible(true)}
        className="rounded-button border border-pebble bg-soft-cream px-4 py-2 font-inter text-[13px] text-sepia hover:bg-pebble/60 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-pebble focus-visible:ring-offset-2 focus-visible:ring-offset-parchment"
      >
        Connect wallet
      </button>
    </Card>
  )
}
