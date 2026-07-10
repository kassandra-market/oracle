import { useEffect, useState, type FormEvent } from 'react'
import type { Market, Oracle, Proposer } from '@kassandra-market/oracles'
import {
  PRICE_SCALE,
  marginProgress,
  twapPrice,
  willDisqualify,
  type AmmV04,
} from '../../../data/ammV04'
import {
  buildCrankTwapIxs,
  buildSwapIxs,
  crankRateLimited,
  swapEstimate,
  type Pool,
  type Side,
} from '../../../data/actions/challengeTrade'
import { buildSettleFromMarketIxs } from '../../../data/actions/challengeSettle'
import { useMarketAmms } from '../../../hooks/useMarketAmms'
import { useWriteAction } from '../../../hooks/useWriteAction'
import { isMockMode } from '../../../data/mockOracles'
import { groupDigits, relativeDeadline } from '../../../lib/oracleView'
import { recallNonce } from '../../../lib/nonceStore'
import { resolveOracleNonce } from '../../../data/actions/finalize'
import { Card } from '../../ui'
import { Chip } from '../Chip'
import { ConnectGate } from './ConnectGate'
import { Field, SubmitButton, TextInput } from './formPrimitives'
import { WriteStatusRegion } from './WriteStatusRegion'

/** Progress at which the settle verdict is treated as "near" the disqualify margin. */
const NEAR_MARGIN = 0.85

/** Recall the oracle's create nonce, else recover it via the pure PDA scan (RF1). */
function oracleNonce(oracle: string): Promise<bigint> {
  const recalled = recallNonce(oracle)
  return recalled !== null ? Promise.resolve(recalled) : resolveOracleNonce(oracle)
}

/** Parse a positive whole-number raw-unit amount for a swap. */
function parseRawAmount(raw: string): { value?: bigint; error?: string } {
  const t = raw.trim()
  if (t === '') return { error: 'Enter an amount (raw base units).' }
  if (!/^\d+$/.test(t)) return { error: 'Amount must be a whole number of base units.' }
  const value = BigInt(t)
  if (value <= 0n) return { error: 'Amount must be greater than zero.' }
  return { value }
}

/** Parse a slippage tolerance in percent (0..100) → basis points. */
function parseSlippageBps(raw: string): { bps: number; error?: string } {
  const t = raw.trim()
  if (t === '') return { bps: 50 } // default 0.5%
  const pct = Number(t)
  if (!Number.isFinite(pct) || pct < 0 || pct > 100) {
    return { bps: 50, error: 'Slippage must be 0–100%.' }
  }
  return { bps: Math.round(pct * 100) }
}

/**
 * The SWAP sub-form: choose pool + side + amountIn + slippage; preview the
 * expected out + price impact from the CU1-decoded reserves (constant-product);
 * submit → `buildSwapIxs` (wallet-signed). The chosen pool's decoded `AmmV04`
 * drives both the preview and the `minAmountOut` slippage floor.
 */
function SwapForm({
  market,
  pools,
  refetch,
}: {
  market: Market
  pools: { pass: AmmV04 | null; fail: AmmV04 | null }
  refetch: () => void
}) {
  const action = useWriteAction(refetch)
  const [pool, setPool] = useState<Pool>('fail')
  const [side, setSide] = useState<Side>('buy')
  const [amountRaw, setAmountRaw] = useState('')
  const [slipRaw, setSlipRaw] = useState('0.5')

  const amm = pool === 'pass' ? pools.pass : pools.fail
  const parsed = parseRawAmount(amountRaw)
  const slip = parseSlippageBps(slipRaw)
  const est = swapEstimate(amm, side, parsed.value ?? 0n)
  const inLabel = side === 'buy' ? 'USDC (quote)' : 'KASS (base)'
  const outLabel = side === 'buy' ? 'KASS (base)' : 'USDC (quote)'
  const impactPct = Math.round(est.impact * 1000) / 10

  const onSubmit = (e: FormEvent) => {
    e.preventDefault()
    if (parsed.error || slip.error || parsed.value === undefined) return
    void action.run(() =>
      buildSwapIxs({
        connection: action.connection,
        market,
        pool,
        side,
        amountIn: parsed.value!,
        user: action.address!,
        slippageBps: slip.bps,
        amm,
      }),
    )
  }

  return (
    <ConnectGate connected={action.connected}>
      <form className="flex flex-col gap-3" onSubmit={onSubmit} noValidate>
        <div className="grid grid-cols-2 gap-3">
          <label className="flex flex-col gap-1.5">
            <span className="font-inter text-[13px] font-medium text-sepia">Pool</span>
            <select
              aria-label="Pool"
              value={pool}
              onChange={(e) => setPool(e.target.value as Pool)}
              className="rounded-tag border border-pebble bg-pure-card px-3 py-2 font-inter text-[14px] text-sepia focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sepia/40"
            >
              <option value="pass">Pass pool</option>
              <option value="fail">Fail pool</option>
            </select>
          </label>
          <label className="flex flex-col gap-1.5">
            <span className="font-inter text-[13px] font-medium text-sepia">Side</span>
            <select
              aria-label="Side"
              value={side}
              onChange={(e) => setSide(e.target.value as Side)}
              className="rounded-tag border border-pebble bg-pure-card px-3 py-2 font-inter text-[14px] text-sepia focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sepia/40"
            >
              <option value="buy">Buy (USDC → KASS)</option>
              <option value="sell">Sell (KASS → USDC)</option>
            </select>
          </label>
        </div>

        <Field
          label={`Amount in — ${inLabel}`}
          hint="Raw base units of the input mint."
          error={amountRaw !== '' ? parsed.error : undefined}
        >
          {(ids) => (
            <TextInput
              ids={ids}
              inputMode="numeric"
              placeholder="e.g. 1000000"
              value={amountRaw}
              onChange={(e) => setAmountRaw(e.target.value)}
            />
          )}
        </Field>

        <Field label="Slippage %" error={slip.error}>
          {(ids) => (
            <TextInput
              ids={ids}
              inputMode="decimal"
              placeholder="0.5"
              value={slipRaw}
              onChange={(e) => setSlipRaw(e.target.value)}
            />
          )}
        </Field>

        {/* Expected-out + price-impact preview (constant-product, CU1 reserves). */}
        <div className="rounded-tag border border-pebble bg-pure-card px-3 py-2 font-inter text-[12px]">
          {amm === null ? (
            <p className="text-driftwood">Pool not readable — no estimate.</p>
          ) : parsed.value === undefined ? (
            <p className="text-driftwood">Enter an amount to preview the expected output.</p>
          ) : (
            <dl className="flex flex-col gap-1">
              <div className="flex items-baseline justify-between gap-3">
                <dt className="text-driftwood">Expected out — {outLabel}</dt>
                <dd className="tabular-nums text-sepia">≈ {groupDigits(est.expectedOut)}</dd>
              </div>
              <div className="flex items-baseline justify-between gap-3">
                <dt className="text-driftwood">Price impact</dt>
                <dd
                  className={`tabular-nums ${est.impact >= 0.1 ? 'text-ember-orange' : 'text-sepia'}`}
                >
                  ≈ {impactPct}%
                </dd>
              </div>
            </dl>
          )}
        </div>

        <div className="flex items-center gap-3">
          <SubmitButton
            verb="Swap"
            status={action.status}
            disabled={parsed.value === undefined || Boolean(slip.error)}
          />
        </div>
        <WriteStatusRegion status={action.status} successVerb="Swapped" />
      </form>
    </ConnectGate>
  )
}

/**
 * The CRANK sub-form: a permissionless per-pool button folding the current price
 * into the pool's TWAP observation → `buildCrankTwapIxs`. Disabled + hinted when
 * the pool was cranked within the last 150 slots (the on-chain rate limit).
 */
function CrankForm({
  market,
  pool,
  amm,
  currentSlot,
  refetch,
}: {
  market: Market
  pool: Pool
  amm: AmmV04 | null
  currentSlot: bigint | null
  refetch: () => void
}) {
  const action = useWriteAction(refetch)
  const limited = crankRateLimited(amm, currentSlot)
  const label = pool === 'pass' ? 'Pass' : 'Fail'

  return (
    <ConnectGate connected={action.connected}>
      <form
        className="flex flex-col gap-2"
        onSubmit={(e) => {
          e.preventDefault()
          void action.run(() => buildCrankTwapIxs({ market, pool }))
        }}
        noValidate
      >
        <div className="flex items-center gap-3">
          <SubmitButton verb={`Crank ${label} TWAP`} status={action.status} disabled={limited} />
          {limited ? (
            <span className="font-inter text-[12px] text-bronze">
              Recently cranked — wait ~150 slots.
            </span>
          ) : null}
        </div>
        <WriteStatusRegion status={action.status} successVerb="Cranked" />
      </form>
    </ConnectGate>
  )
}

/**
 * The ONE-CLICK settle sub-form (no JSON paste). The full 21-account settle set
 * is DERIVED client-side from the decoded {@link Market} + {@link Oracle} (+ the
 * challenged proposer's authority, read off the fetched proposers) via
 * {@link buildSettleFromMarketIxs}; the connected wallet just presses "Settle".
 * The oracle nonce is recalled (or PDA-scanned) exactly like every other write.
 */
function SettleButton({
  oracleKey,
  oracle,
  market,
  proposerAuthority,
  refetch,
}: {
  oracleKey: string
  oracle: Oracle
  market: Market
  /** The challenged proposer's wallet authority (owner of proposerUsdc). */
  proposerAuthority: string | undefined
  refetch: () => void
}) {
  const action = useWriteAction(refetch)

  const onSubmit = (e: FormEvent) => {
    e.preventDefault()
    void action.run(async () => {
      const nonce = await oracleNonce(oracleKey)
      return buildSettleFromMarketIxs({
        connection: action.connection,
        oracleNonce: nonce,
        market,
        oracle,
        proposerAuthority: proposerAuthority!,
        payer: action.address ?? undefined,
      })
    })
  }

  return (
    <ConnectGate connected={action.connected}>
      <form className="flex flex-col gap-3" onSubmit={onSubmit} noValidate>
        <div className="flex items-center gap-3">
          <SubmitButton
            verb="Settle challenge"
            status={action.status}
            disabled={proposerAuthority === undefined}
          />
        </div>
        {proposerAuthority === undefined ? (
          <p className="font-inter text-[12px] text-ember-orange">
            The challenged proposer isn&apos;t loaded yet — reload the oracle to settle.
          </p>
        ) : null}
        <WriteStatusRegion status={action.status} successVerb="Settled" />
      </form>
    </ConnectGate>
  )
}

/** Format a PRICE_SCALE-scaled TWAP (1e12) as a human decimal price string. */
function formatTwap(scaled: bigint | null): string {
  return scaled === null ? '—' : (Number(scaled) / Number(PRICE_SCALE)).toFixed(4)
}

/**
 * CU2 — the challenge-market TRADE / CRANK / SETTLE controls, rendered beside the
 * CU1 read viz in the detail's Challenge-market section. Three grouped write
 * surfaces over the same externally-composed MetaDAO v0.4 pools CU1 decodes:
 *
 *   - Swap: pool + side + amount + slippage, with a constant-product expected-out
 *     + price-impact preview from the CU1-decoded reserves;
 *   - Crank TWAP: permissionless per-pool, disabled/hinted when cranked within the
 *     on-chain 150-slot rate limit (uses the CU1 `lastUpdatedSlot` + current slot);
 *   - Settle: permissionless, enabled ONLY after `market.twapEnd` && !settled, with
 *     a live fail-vs-pass TWAP verdict preview (via CU1's `marginProgress`).
 *
 * The write controls are ConnectGate'd (the CU1 read viz stays visible
 * disconnected); phase gating is the caller's (only rendered in the Challenge
 * phase). The single ember accent is reserved for a genuine over-margin verdict
 * / high impact — no new embers beyond CU1's margin accent.
 */
export function ChallengeTradeControls({
  oraclePubkey,
  oracle,
  market,
  proposers,
  refetch,
}: {
  /** The oracle PDA (base58). */
  oraclePubkey: string
  /** The decoded oracle (its margin threshold drives the verdict preview). */
  oracle: Oracle
  /** The challenge {@link Market} being traded. */
  market: Market
  /**
   * The oracle's decoded proposers (keyed by PDA) — the one-click settle reads the
   * challenged proposer's `authority` (owner of the proposer USDC payout) off the
   * proposer whose pubkey == `market.proposer`.
   */
  proposers: { pubkey: string; proposer: Proposer }[]
  /** Refetch the oracle detail on a successful write. */
  refetch: () => void
}) {
  const { pass, fail, refetch: refetchAmms } = useMarketAmms(market)
  const [currentSlot, setCurrentSlot] = useState<bigint | null>(null)
  const { connection } = useWriteAction()

  // Best-effort current slot for the crank rate-limit hint (mock: skip).
  useEffect(() => {
    if (isMockMode()) return
    let active = true
    const getSlot = (connection as unknown as { getSlot?: () => Promise<number> }).getSlot
    if (typeof getSlot !== 'function') return
    getSlot.call(connection).then(
      (s: number) => {
        if (active) setCurrentSlot(BigInt(s))
      },
      () => {},
    )
    return () => {
      active = false
    }
  }, [connection, market.passAmm])

  const onWrite = () => {
    refetch()
    refetchAmms()
  }

  const passTwap = pass ? twapPrice(pass) : null
  const failTwap = fail ? twapPrice(fail) : null
  const progress = marginProgress(
    failTwap,
    passTwap,
    oracle.marketThresholdNum,
    oracle.marketThresholdDen,
  )
  const twapReady = passTwap !== null && failTwap !== null
  // The verdict uses the exact bigint on-chain boundary (strict `>`), not the
  // float `progress >= 1` — at exact equality the proposer survives on-chain.
  const wouldDisqualify =
    twapReady &&
    willDisqualify(
      failTwap,
      passTwap,
      oracle.marketThresholdNum,
      oracle.marketThresholdDen,
    )
  const near = twapReady && progress >= NEAR_MARGIN

  const nowUnix = BigInt(Math.floor(Date.now() / 1000))
  const settleOpen = !market.settled && nowUnix >= market.twapEnd

  // The challenged proposer's wallet authority (owner of the proposer USDC payout
  // the settle handler asserts) — off the proposer whose PDA == market.proposer.
  const marketProposer = market.proposer.toString()
  const proposerAuthority = proposers
    .find((p) => p.pubkey === marketProposer)
    ?.proposer.authority.toString()

  return (
    <Card className="mt-4 flex flex-col gap-5">
      <div>
        <h3 className="font-serif text-subheading font-light text-sepia">Trade &amp; settle</h3>
        <p className="mt-1 font-inter text-[13px] text-driftwood">
          Trade the pass/fail conditional pools, crank their TWAP, and settle the challenge — the
          swap-driven TWAP is what decides the verdict.
        </p>
      </div>

      {market.settled ? (
        <p className="font-inter text-[13px] text-bronze">
          This market is settled — trading and cranking are closed.
        </p>
      ) : (
        <>
          {/* Swap */}
          <section className="border-t border-pebble pt-4">
            <h4 className="font-inter text-[13px] font-medium text-sepia">Swap a pool</h4>
            <div className="mt-3">
              <SwapForm market={market} pools={{ pass, fail }} refetch={onWrite} />
            </div>
          </section>

          {/* Crank */}
          <section className="border-t border-pebble pt-4">
            <h4 className="font-inter text-[13px] font-medium text-sepia">Crank TWAP</h4>
            <p className="mt-0.5 font-inter text-[12px] text-driftwood">
              Permissionless — folds the current price into a pool&apos;s TWAP (once per ~150 slots).
            </p>
            <div className="mt-3 grid grid-cols-1 gap-4 sm:grid-cols-2">
              <CrankForm
                market={market}
                pool="pass"
                amm={pass}
                currentSlot={currentSlot}
                refetch={onWrite}
              />
              <CrankForm
                market={market}
                pool="fail"
                amm={fail}
                currentSlot={currentSlot}
                refetch={onWrite}
              />
            </div>
          </section>
        </>
      )}

      {/* Settle + verdict preview */}
      <section className="border-t border-pebble pt-4">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <h4 className="font-inter text-[13px] font-medium text-sepia">Settle challenge</h4>
          {twapReady ? (
            <Chip tone={wouldDisqualify ? 'ember' : 'confirmed'}>
              {wouldDisqualify ? 'Would DISQUALIFY' : 'Would SURVIVE'}
            </Chip>
          ) : null}
        </div>

        {/* Verdict preview (CU1 marginProgress against the on-chain threshold). */}
        <div className="mt-2 rounded-tag border border-pebble bg-pure-card px-3 py-2 font-inter text-[12px]">
          <dl className="flex flex-col gap-1">
            <div className="flex items-baseline justify-between gap-3">
              <dt className="text-driftwood">Pass TWAP</dt>
              <dd className="tabular-nums text-sepia">{formatTwap(passTwap)}</dd>
            </div>
            <div className="flex items-baseline justify-between gap-3">
              <dt className="text-driftwood">Fail TWAP</dt>
              <dd className="tabular-nums text-sepia">{formatTwap(failTwap)}</dd>
            </div>
            <div className="flex items-baseline justify-between gap-3">
              <dt className="text-driftwood">
                Margin {oracle.marketThresholdNum.toString()}/{oracle.marketThresholdDen.toString()}
              </dt>
              <dd className={`tabular-nums ${near ? 'text-ember-orange' : 'text-sepia'}`}>
                {twapReady ? `${Math.round(progress * 100)}%` : '—'}
              </dd>
            </div>
          </dl>
          <p className="mt-1.5 text-driftwood">
            {!twapReady
              ? 'TWAP forming — the verdict is not yet meaningful (pre start-delay).'
              : wouldDisqualify
                ? 'The fail TWAP has cleared the margin — settling would disqualify the proposer.'
                : 'The fail TWAP is within the margin — settling would let the proposer survive.'}
          </p>
        </div>

        {market.settled ? (
          <p className="mt-3 font-inter text-[12px] text-driftwood">This market is already settled.</p>
        ) : !settleOpen ? (
          <p className="mt-3 font-inter text-[12px] text-bronze">
            Settle opens after the market&apos;s TWAP window ({relativeDeadline(market.twapEnd)}).
          </p>
        ) : (
          <div className="mt-3">
            <p className="font-inter text-[12px] text-driftwood">
              Permissionless — any connected wallet can crank the settle. The full account set is
              derived from the market; no paste needed.
            </p>
            <div className="mt-3">
              <SettleButton
                oracleKey={oraclePubkey}
                oracle={oracle}
                market={market}
                proposerAuthority={proposerAuthority}
                refetch={onWrite}
              />
            </div>
          </div>
        )}
      </section>
    </Card>
  )
}

export default ChallengeTradeControls
