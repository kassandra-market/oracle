import { useCallback, useEffect, useMemo, useRef, useState, type FormEvent } from 'react'
import { Link, useNavigate } from 'react-router-dom'
import { Address } from '@solana/web3.js'
import { decodeProtocol, pda } from '@kassandra/sdk'
import { useWalletModal } from '@solana/wallet-adapter-react-ui'
import { Card, EyebrowTag } from '../components/ui'
import { Field, SubmitButton, TextInput } from '../components/oracles/actions/formPrimitives'
import { WriteStatusRegion } from '../components/oracles/actions/WriteStatusRegion'
import { useWriteAction } from '../hooks/useWriteAction'
import { hashHex } from '../lib/oracleView'
import { rememberNonce } from '../lib/nonceStore'
import { isMockMode } from '../data/mockOracles'
import {
  buildCreateOracleIxs,
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
      navigate(`/oracles/${built.oracle.toString()}`)
    }
  })

  // A stable, freshly-random nonce per page session (the Oracle PDA seed).
  const [nonce] = useState<bigint>(() => randomNonce())

  const [question, setQuestion] = useState('')
  const [optionsCount, setOptionsCount] = useState(2)
  const [deadline, setDeadline] = useState(() =>
    toDatetimeLocal(new Date(Date.now() + 24 * 3600 * 1000)),
  )
  const [kassMint, setKassMint] = useState(mock ? MOCK_KASS : '')
  const [usdcMint, setUsdcMint] = useState(mock ? MOCK_USDC : '')
  const [mintsLoading, setMintsLoading] = useState(!mock)

  const [errors, setErrors] = useState<Record<string, string | undefined>>({})
  const [promptHashHex, setPromptHashHex] = useState<string>('')

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

  // Live SHA-256 preview of the question (the on-chain prompt_hash).
  useEffect(() => {
    let cancelled = false
    if (question.trim().length === 0) {
      setPromptHashHex('')
      return
    }
    void crypto.subtle
      .digest('SHA-256', new TextEncoder().encode(question))
      .then((d) => {
        if (!cancelled) setPromptHashHex(hashHex(new Uint8Array(d)))
      })
    return () => {
      cancelled = true
    }
  }, [question])

  const validate = useCallback((): boolean => {
    const next: Record<string, string | undefined> = {}
    if (question.trim().length === 0) next.question = 'Enter a question for the oracle.'
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
  }, [question, deadline, kassMint, usdcMint])

  const onSubmit = (e: FormEvent) => {
    e.preventDefault()
    if (!validate()) return
    const deadlineUnix = datetimeLocalToUnix(deadline)
    void action.run(async () => {
      const built = await buildCreateOracleIxs({
        connection: action.connection,
        nonce,
        question,
        optionsCount,
        deadline: deadlineUnix,
        creator: action.address!,
        kassMint: kassMint.trim(),
        usdcMint: usdcMint.trim(),
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
          Pose a question, set how many options it can resolve to, and a deadline. The question
          text is hashed on-chain as the oracle's prompt; proposers stake KASS behind an answer.
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
                label="Prompt hash (SHA-256, on-chain)"
                hint={
                  promptHashHex
                    ? undefined
                    : 'Type a question — its SHA-256 is committed as the oracle prompt.'
                }
              >
                {() => (
                  <p className="break-all rounded-tag border border-pebble bg-soft-cream px-3 py-2 font-mono text-[12px] text-bronze">
                    {promptHashHex || '—'}
                  </p>
                )}
              </Field>

              <Field label="Options" hint="How many categorical answers this oracle can resolve to.">
                {(ids) => (
                  <select
                    id={ids.id}
                    aria-describedby={ids.describedById}
                    value={optionsCount}
                    onChange={(e) => setOptionsCount(Number(e.target.value))}
                    className={selectClass}
                  >
                    {Array.from({ length: 9 }, (_, i) => i + 2).map((n) => (
                      <option key={n} value={n}>
                        {n} options
                      </option>
                    ))}
                  </select>
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
