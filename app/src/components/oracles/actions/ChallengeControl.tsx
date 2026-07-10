import type { Market, Oracle } from '@kassandra-market/oracles'
import { Card } from '../../ui'
import { ChallengeComposeForm } from './ChallengeComposeForm'

/**
 * RF4 — the Challenge-phase control. The challenge round runs over a MetaDAO
 * v0.4 market (a binary question, KASS/USDC conditional vaults, two pass/fail
 * AMMs). This surface hosts challenge STATUS + the CLIENT-SIDE compose→open flow
 * (CU3's {@link ChallengeComposeForm}, no runner JSON): any wallet challenges an
 * uncontested claim by composing the whole market from the browser and funding
 * the USDC escrow.
 *
 * TRADE / CRANK / SETTLE live on the Challenge-market card below
 * ({@link ChallengeTradeControls}) — settle there is ONE-CLICK, its account set
 * derived from the decoded Market (no JSON paste anywhere in the challenge UI).
 * The Market's live status is shown on that same card.
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
          client-side (no runner JSON). Trade, crank, and one-click settle live on the
          challenge-market card below.
        </p>
      </div>

      {/* Open a challenge — the client-side staged compose→open (CU3). Only when
          no market is open yet (opening a second challenge is not the flow). */}
      {market === undefined ? (
        <ChallengeComposeForm oraclePubkey={pubkey} oracle={oracle} refetch={refetch} />
      ) : null}
    </Card>
  )
}

export default ChallengeControl
