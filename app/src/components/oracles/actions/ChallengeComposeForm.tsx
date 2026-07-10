import { useCallback, useState, type FormEvent } from 'react'
import type { Oracle } from '@kassandra-market/oracles'
import { Card } from '../../ui'
import { recallNonce } from '../../../lib/nonceStore'
import { resolveOracleNonce } from '../../../data/actions/finalize'
import {
  DEFAULT_BASE_RESERVE,
  DEFAULT_QUOTE_RESERVE,
  buildComposeAndOpenChallengeIxs,
  type ComposeStep,
} from '../../../data/actions/challengeCompose'
import { useComposeSequence, type StepStatus } from '../../../hooks/useComposeSequence'
import { useConnection } from '../../../lib/cluster'
import { isMockMode } from '../../../data/mockOracles'
import { ConnectGate } from './ConnectGate'
import { Field, SubmitButton, TextInput } from './formPrimitives'

/** Recall the oracle's create nonce, else recover it via the pure PDA scan (RF1). */
function oracleNonce(oracle: string): Promise<bigint> {
  const recalled = recallNonce(oracle)
  return recalled !== null ? Promise.resolve(recalled) : resolveOracleNonce(oracle)
}

/** Parse a positive whole-number raw-unit reserve; empty → the default. */
function parseReserve(raw: string, fallback: bigint): { value: bigint; error?: string } {
  const t = raw.trim()
  if (t === '') return { value: fallback }
  if (!/^\d+$/.test(t)) return { value: fallback, error: 'Enter a whole number of base units.' }
  const v = BigInt(t)
  if (v <= 0n) return { value: fallback, error: 'Must be greater than zero.' }
  return { value: v }
}

/** A KASS-DAO override (needed for the escrow's kass_price); empty is invalid. */
function parseAddress(raw: string): { value?: string; error?: string } {
  const t = raw.trim()
  if (t === '') return { error: 'The KASS DAO address is required (kass_price source).' }
  return { value: t }
}

/** The challenged proposer PDA (open_challenge derives its ai_claim/market); required. */
function parseProposer(raw: string): { value?: string; error?: string } {
  const t = raw.trim()
  if (t === '') return { error: 'The challenged proposer address is required.' }
  return { value: t }
}

/** Read a URL query param (the e2e/deep-link default for the proposer / DAO). */
function useParam(name: string): string {
  return typeof window === 'undefined'
    ? ''
    : (new URLSearchParams(window.location.search).get(name) ?? '')
}

/** A single step row in the staged-progress checklist. */
function StepRow({ label, status }: { label: string; status: StepStatus | undefined }) {
  const kind = status?.kind ?? 'pending'
  const mark =
    kind === 'done' ? '✓' : kind === 'running' ? '…' : kind === 'error' ? '✕' : '·'
  const tone =
    kind === 'done'
      ? 'text-chestnut'
      : kind === 'error'
        ? 'text-ember-orange'
        : kind === 'running'
          ? 'text-sepia'
          : 'text-driftwood'
  return (
    <li className="flex items-baseline gap-2 font-inter text-[13px]">
      <span aria-hidden className={`w-4 tabular-nums ${tone}`}>
        {mark}
      </span>
      <span className={kind === 'pending' ? 'text-driftwood' : 'text-sepia'}>{label}</span>
      <span className="sr-only">
        {kind === 'done'
          ? 'completed'
          : kind === 'running'
            ? 'in progress'
            : kind === 'error'
              ? 'failed'
              : 'pending'}
      </span>
      {kind === 'error' && status && 'message' in status ? (
        <span className="ml-1 text-ember-orange">— {status.message}</span>
      ) : null}
    </li>
  )
}

/**
 * CU3 — the CLIENT-SIDE "Open a challenge" form (replaces the RF4 JSON-paste open
 * path). The challenger sets the seed-liquidity params (sane defaults), then a
 * STAGED compose→open runs step by step — question → KASS vault → USDC vault →
 * fund+split → pass pool → fail pool → open challenge — each a wallet-signed tx
 * with per-step progress. A mid-sequence failure is shown inline and can be
 * RETRIED from the failed step (the idempotent ATA-creates + deterministic PDAs
 * make a resume safe). On success the market exists → the CU1 panel + CU2 trade
 * controls light up.
 *
 * Phase-gated by the caller (Challenge phase, NO existing market) + ConnectGate'd.
 */
export function ChallengeComposeForm({
  oraclePubkey,
  oracle,
  refetch,
}: {
  /** The oracle PDA (base58). */
  oraclePubkey: string
  /** The decoded oracle (its KASS/USDC mints seed the vaults). */
  oracle: Oracle
  /** Refetch the oracle detail once the market opens. */
  refetch: () => void
}) {
  const { connection } = useConnection()
  const seq = useComposeSequence(refetch)
  const [baseRaw, setBaseRaw] = useState('')
  const [quoteRaw, setQuoteRaw] = useState('')
  const [daoRaw, setDaoRaw] = useState(useParam('kassDao'))
  const [proposerRaw, setProposerRaw] = useState(useParam('proposer'))
  const [steps, setSteps] = useState<ComposeStep[] | null>(null)
  const [buildError, setBuildError] = useState<string | undefined>()

  const base = parseReserve(baseRaw, DEFAULT_BASE_RESERVE)
  const quote = parseReserve(quoteRaw, DEFAULT_QUOTE_RESERVE)
  const dao = parseAddress(daoRaw)
  const proposer = parseProposer(proposerRaw)
  const inputInvalid =
    Boolean(base.error) || Boolean(quote.error) || Boolean(dao.error) || Boolean(proposer.error)

  // Labels rendered in the checklist (index-aligned to the built steps, or the
  // canonical order before a build so the checklist is visible up front).
  const labels = steps
    ? steps.map((s) => s.label)
    : [
        'Create question',
        'Create KASS vault',
        'Create USDC vault',
        'Fund + split conditional tokens',
        'Seed pass pool',
        'Seed fail pool',
        'Open challenge',
      ]

  const compose = useCallback(
    async (address: string): Promise<ComposeStep[]> => {
      const nonce = await oracleNonce(oraclePubkey)
      const { steps: built } = await buildComposeAndOpenChallengeIxs({
        connection,
        oracleNonce: nonce,
        // The CHALLENGED proposer PDA (open_challenge derives its ai_claim/market);
        // the connected wallet is only the challenger/funder.
        proposer: proposer.value!,
        challenger: address,
        kassMint: oracle.kassMint,
        usdcMint: oracle.usdcMint,
        kassDao: dao.value!,
        baseReserve: base.value,
        quoteReserve: quote.value,
      })
      return built
    },
    [
      oraclePubkey,
      connection,
      oracle.kassMint,
      oracle.usdcMint,
      proposer.value,
      dao.value,
      base.value,
      quote.value,
    ],
  )

  const onSubmit = async (e: FormEvent) => {
    e.preventDefault()
    setBuildError(undefined)
    if (inputInvalid || seq.busy) return
    try {
      // In mock mode the compose builder can't derive real keys — drive the
      // staged sequence UI with the canonical labels only.
      if (isMockMode()) {
        const stub = labels.map(
          (label, i): ComposeStep => ({ id: `mock-${i}`, label, ixs: [] as never }),
        )
        setSteps(stub)
        seq.reset(stub.length)
        await seq.run(stub, 0)
        return
      }
      const built = await compose(seq.address!)
      setSteps(built)
      seq.reset(built.length)
      await seq.run(built, 0)
    } catch (err) {
      setBuildError(err instanceof Error ? err.message : String(err))
    }
  }

  const onRetry = async () => {
    if (!steps || seq.busy) return
    await seq.run(steps, seq.resumeFrom)
  }

  const hasError = seq.statuses.some((s) => s.kind === 'error')
  const started = steps !== null && seq.statuses.length > 0

  return (
    <Card className="mt-4 flex flex-col gap-4">
      <div>
        <h3 className="font-serif text-subheading font-light text-sepia">Open a challenge</h3>
        <p className="mt-1 font-inter text-[13px] text-driftwood">
          Compose the full MetaDAO v0.4 market from the browser — no runner JSON. This stakes USDC to
          challenge an uncontested claim; the connected wallet is the challenger and pays for each
          step.
        </p>
      </div>

      <ConnectGate connected={seq.connected}>
        <form className="flex flex-col gap-3" onSubmit={onSubmit} noValidate>
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
            <Field
              label="Seed base (conditional-KASS)"
              hint={`Raw base units per pool. Default ${DEFAULT_BASE_RESERVE.toString()}.`}
              error={base.error}
            >
              {(ids) => (
                <TextInput
                  ids={ids}
                  inputMode="numeric"
                  placeholder={DEFAULT_BASE_RESERVE.toString()}
                  value={baseRaw}
                  onChange={(e) => setBaseRaw(e.target.value)}
                />
              )}
            </Field>
            <Field
              label="Seed quote (conditional-USDC)"
              hint={`Raw base units per pool. Default ${DEFAULT_QUOTE_RESERVE.toString()}.`}
              error={quote.error}
            >
              {(ids) => (
                <TextInput
                  ids={ids}
                  inputMode="numeric"
                  placeholder={DEFAULT_QUOTE_RESERVE.toString()}
                  value={quoteRaw}
                  onChange={(e) => setQuoteRaw(e.target.value)}
                />
              )}
            </Field>
          </div>

          <Field
            label="Challenged proposer"
            hint="The Proposer PDA whose AI claim you're challenging (open_challenge derives its market)."
            error={proposer.error && proposerRaw !== '' ? proposer.error : undefined}
          >
            {(ids) => (
              <TextInput
                ids={ids}
                placeholder="Proposer PDA (base58)"
                value={proposerRaw}
                onChange={(e) => setProposerRaw(e.target.value)}
              />
            )}
          </Field>

          <Field
            label="KASS DAO address"
            hint="The futarchy Dao (protocol.kass_dao) — the escrow's kass_price source."
            error={dao.error && daoRaw !== '' ? dao.error : undefined}
          >
            {(ids) => (
              <TextInput
                ids={ids}
                placeholder="Dao PDA (base58)"
                value={daoRaw}
                onChange={(e) => setDaoRaw(e.target.value)}
              />
            )}
          </Field>

          {/* Staged-progress checklist. */}
          {started ? (
            <ol className="flex flex-col gap-1.5 rounded-tag border border-pebble bg-pure-card px-3 py-3" aria-label="Compose progress">
              {labels.map((label, i) => (
                <StepRow key={label} label={label} status={seq.statuses[i]} />
              ))}
            </ol>
          ) : null}

          <div className="flex items-center gap-3">
            {!started || (!hasError && !seq.allDone) ? (
              <SubmitButton
                verb={started ? 'Composing…' : 'Compose & open challenge'}
                status={seq.busy ? { kind: 'signing' } : { kind: 'idle' }}
                disabled={inputInvalid || seq.busy}
              />
            ) : null}
            {hasError && !seq.busy ? (
              <button
                type="button"
                onClick={onRetry}
                className="rounded-tag bg-chestnut px-4 py-2 font-inter text-[14px] font-medium text-parchment hover:opacity-90 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sepia/40 focus-visible:ring-offset-2 focus-visible:ring-offset-parchment"
              >
                Retry from step {seq.resumeFrom + 1}
              </button>
            ) : null}
          </div>

          {buildError ? (
            <p role="alert" className="font-inter text-[12px] text-ember-orange">
              {buildError}
            </p>
          ) : null}
          {seq.allDone ? (
            <p role="status" className="font-inter text-[13px] text-chestnut">
              Challenge opened — the market is live. The visualization and trade controls below will
              light up on refresh.
            </p>
          ) : hasError ? (
            <p role="status" className="font-inter text-[12px] text-bronze">
              A step failed. Completed steps are safe to keep — retry resumes from the failed step.
            </p>
          ) : null}
        </form>
      </ConnectGate>
    </Card>
  )
}

export default ChallengeComposeForm
