import { useState, type FormEvent } from 'react'
import type { Market, Oracle } from '@kassandra/sdk'
import type { TransactionInstruction } from '@solana/web3.js'
import { Card } from '../../ui'
import { relativeDeadline } from '../../../lib/oracleView'
import { recallNonce } from '../../../lib/nonceStore'
import { resolveOracleNonce } from '../../../data/actions/finalize'
import { buildSettleChallengeIxs } from '../../../data/actions/challenge'
import { useWriteAction } from '../../../hooks/useWriteAction'
import { ChallengeComposeForm } from './ChallengeComposeForm'
import { ConnectGate } from './ConnectGate'
import { SubmitButton } from './formPrimitives'
import { WriteStatusRegion } from './WriteStatusRegion'

/** Recall the oracle's create nonce, else recover it via the pure PDA scan (RF1). */
function oracleNonce(oracle: string): Promise<bigint> {
  const recalled = recallNonce(oracle)
  return recalled !== null ? Promise.resolve(recalled) : resolveOracleNonce(oracle)
}

/** A JSON-paste crank sub-form: parse the composed-account payload → build → send. */
function JsonCrankForm({
  verb,
  successVerb,
  placeholder,
  build,
  refetch,
}: {
  verb: string
  successVerb: string
  placeholder: string
  /** Build the ixs from the parsed payload + the connected wallet address. */
  build: (payload: Record<string, unknown>, address: string) => Promise<TransactionInstruction[]>
  refetch: () => void
}) {
  const action = useWriteAction(refetch)
  const [text, setText] = useState('')
  const [error, setError] = useState<string | undefined>()

  const onSubmit = (e: FormEvent) => {
    e.preventDefault()
    setError(undefined)
    let payload: Record<string, unknown>
    try {
      payload = JSON.parse(text) as Record<string, unknown>
    } catch {
      setError('Could not parse the composed-account payload as JSON.')
      return
    }
    void action.run(() => build(payload, action.address!))
  }

  return (
    <ConnectGate connected={action.connected}>
      <form className="flex flex-col gap-3" onSubmit={onSubmit} noValidate>
        <textarea
          aria-label={`${verb} — composed account payload (JSON)`}
          rows={5}
          placeholder={placeholder}
          value={text}
          onChange={(e) => setText(e.target.value)}
          className="w-full rounded-tag border border-pebble bg-pure-card px-3 py-2 font-mono text-[12px] text-sepia placeholder:text-driftwood focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sepia/40 focus-visible:ring-offset-2 focus-visible:ring-offset-parchment"
        />
        <div className="flex items-center gap-3">
          <SubmitButton verb={verb} status={action.status} />
        </div>
        {error ? <p className="font-inter text-[12px] text-ember-orange">{error}</p> : null}
        <WriteStatusRegion status={action.status} successVerb={successVerb} />
      </form>
    </ConnectGate>
  )
}

/**
 * RF4 — the Challenge-phase control. The challenge round runs over an
 * EXTERNALLY-COMPOSED MetaDAO v0.4 market (a binary question, KASS/USDC
 * conditional vaults, two pass/fail AMMs) that a browser cannot assemble — so
 * this is a deliberately THIN surface: challenge STATUS + an advanced "open" and
 * "settle-crank" affordance that takes the composed account set as a pasted JSON
 * payload (the runner / an integrator emits it). It is NOT a full AMM trading UI.
 *
 *   - open: any wallet challenges an uncontested claim (funds the USDC escrow);
 *   - settle: permissionless, slot-based — after the market's TWAP window elapses,
 *     the swap-driven AMM TWAP verdict resolves the challenge (disqualify/survive).
 *
 * The connected wallet is the challenger for open (its USDC source is in the
 * payload). The Market's live status is shown on the Challenge-market card below.
 */
export function ChallengeControl({
  pubkey,
  oracle,
  market,
  refetch,
}: {
  /** The oracle PDA (base58). */
  pubkey: string
  /** The decoded oracle. */
  oracle: Oracle
  /** The first challenge market for this oracle, if one is open. */
  market?: { pubkey: string; market: Market }
  refetch: () => void
}) {
  const twapEnd = market?.market.twapEnd
  const nowUnix = BigInt(Math.floor(Date.now() / 1000))
  const settleOpen = market !== undefined && !market.market.settled && twapEnd !== undefined && nowUnix >= twapEnd

  const settlePlaceholder =
    '{ "aiClaim": "…", "proposer": "…", "question": "…", "passAmm": "…", "failAmm": "…", "cvEventAuthority": "…", "kassVault": "…", "kassVaultUnderlying": "…", "passKassMint": "…", "failKassMint": "…", "oraclePassKass": "…", "oracleFailKass": "…", "proposerUsdc": "…", "challengerUsdcDest": "…", "challengerKass": "…" }'

  return (
    <Card className="flex flex-col gap-4">
      <div>
        <h3 className="font-serif text-subheading font-light text-sepia">Challenge round</h3>
        <p className="mt-1 font-inter text-[13px] text-driftwood">
          Open challenges: <span className="text-sepia">{oracle.openChallengeCount}</span>
          {market ? (
            <>
              {' · '}a market is {market.market.settled ? 'settled' : 'open'} (see the challenge-market
              card below)
            </>
          ) : null}
        </p>
        <p className="mt-1 font-inter text-[12px] text-driftwood">
          Open a challenge directly from the browser — the full MetaDAO v0.4 market is composed
          client-side (no runner JSON). Settle still takes the composed account set as a pasted
          payload (the runner emits it).
        </p>
      </div>

      {/* Open a challenge — the client-side staged compose→open (CU3). Only when
          no market is open yet (opening a second challenge is not the flow). */}
      {market === undefined ? (
        <ChallengeComposeForm oraclePubkey={pubkey} oracle={oracle} refetch={refetch} />
      ) : null}

      {/* Settle-crank */}
      <div className="border-t border-pebble pt-3">
        <h4 className="font-inter text-[13px] font-medium text-sepia">Settle challenge</h4>
        {market === undefined ? (
          <p className="mt-0.5 font-inter text-[12px] text-driftwood">
            No challenge market is open — nothing to settle.
          </p>
        ) : market.market.settled ? (
          <p className="mt-0.5 font-inter text-[12px] text-driftwood">
            This market is already settled.
          </p>
        ) : !settleOpen ? (
          <p className="mt-0.5 font-inter text-[12px] text-bronze">
            Settle opens after the market’s TWAP window ({twapEnd ? relativeDeadline(twapEnd) : '—'}).
          </p>
        ) : (
          <>
            <p className="mt-0.5 font-inter text-[12px] text-driftwood">
              Permissionless — the swap-driven AMM TWAP verdict resolves the challenge. Any connected
              wallet can crank this; it only pays the fee.
            </p>
            <div className="mt-3">
              <JsonCrankForm
                verb="Settle challenge"
                successVerb="Settled"
                placeholder={settlePlaceholder}
                refetch={refetch}
                build={async (payload) => {
                  const nonce = await oracleNonce(pubkey)
                  return buildSettleChallengeIxs({
                    oracleNonce: nonce,
                    aiClaim: payload.aiClaim as string,
                    proposer: payload.proposer as string,
                    question: payload.question as string,
                    passAmm: payload.passAmm as string,
                    failAmm: payload.failAmm as string,
                    cvEventAuthority: payload.cvEventAuthority as string,
                    kassVault: payload.kassVault as string,
                    kassVaultUnderlying: payload.kassVaultUnderlying as string,
                    passKassMint: payload.passKassMint as string,
                    failKassMint: payload.failKassMint as string,
                    oraclePassKass: payload.oraclePassKass as string,
                    oracleFailKass: payload.oracleFailKass as string,
                    proposerUsdc: payload.proposerUsdc as string,
                    challengerUsdcDest: payload.challengerUsdcDest as string,
                    challengerKass: payload.challengerKass as string,
                  })
                }}
              />
            </div>
          </>
        )}
      </div>
    </Card>
  )
}

export default ChallengeControl
