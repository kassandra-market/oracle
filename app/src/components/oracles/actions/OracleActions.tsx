import { Phase, type Market, type Oracle } from '@kassandra-market/oracles'
import { Card } from '../../ui'
import { phaseView } from '../../../lib/oracleView'
import { recallNonce } from '../../../lib/nonceStore'
import {
  MAX_LEGACY_TAIL,
  buildAdvancePhaseIxs,
  buildFinalizeAiClaimsIxs,
  buildFinalizeFactsIxs,
  buildFinalizeOracleIxs,
  buildFinalizeProposalsIxs,
} from '../../../data/actions/finalize'
import { ProposeForm } from './ProposeForm'
import { SubmitFactForm } from './SubmitFactForm'
import { SubmitAiClaimForm } from './SubmitAiClaimForm'
import { ChallengeControl } from './ChallengeControl'
import { FinalizeControl } from './FinalizeControl'
import { SweepControl } from './SweepControl'

/** Muted "participation closed / redirected" note for the non-form phases. */
function Note({ children }: { children: React.ReactNode }) {
  return (
    <Card>
      <p className="font-inter text-[14px] text-driftwood">{children}</p>
    </Card>
  )
}

/**
 * The "Participate" surface on the oracle detail page. Phase-gated: the propose
 * form in Proposal, the submit-fact form in FactProposal, a pointer to the
 * per-fact vote controls in FactVoting, and a muted closed-note otherwise — PLUS
 * a permissionless {@link FinalizeControl} CRANK in every pre-Resolved phase that
 * advances the oracle (finalize proposals / advance phase / finalize facts /
 * finalize AI claims / finalize oracle). The per-fact {@link VoteControl}s live
 * on the fact cards themselves. Read-only browsing is intact when disconnected.
 *
 * The finalize tails (proposer / fact PDAs) come from the already-fetched oracle
 * detail; `proposers`/`facts` are those pubkey lists.
 */
export function OracleActions({
  pubkey,
  oracle,
  refetch,
  proposers = [],
  facts = [],
  market,
}: {
  pubkey: string
  oracle: Oracle
  refetch: () => void
  /** Proposer-PDA pubkeys (the finalize proposer tail). */
  proposers?: string[]
  /** Fact-PDA pubkeys (the finalize-facts tail). */
  facts?: string[]
  /** The first challenge market (if open) — the RF4 challenge status + settle-crank. */
  market?: { pubkey: string; market: Market }
}) {
  const phaseLabel = phaseView(oracle.phase).label
  const kassMint = oracle.kassMint
  // The full-set finalizes overflow a legacy tx past MAX_LEGACY_TAIL proposers.
  const proposersNearCap = proposers.length > MAX_LEGACY_TAIL
  // finalize_facts uses the fact tail, or the proposers in the no-facts dead-end.
  const factsTail = facts.length > 0 ? facts : proposers

  return (
    <section className="mt-14">
      <h2 className="font-serif text-heading-sm font-light text-sepia">Participate</h2>
      <div className="mt-4 flex flex-col gap-4">
        {oracle.phase === Phase.Proposal ? (
          <>
            <ProposeForm pubkey={pubkey} oracle={oracle} refetch={refetch} />
            <FinalizeControl
              title="Finalize proposals"
              description="Once the proposal window has closed, crank the oracle into the dispute round (or resolve it if the proposals agree)."
              verb="Finalize proposals"
              successVerb="Finalized"
              nearCap={proposersNearCap}
              refetch={refetch}
              build={() => buildFinalizeProposalsIxs({ oracle: pubkey, proposers })}
            />
          </>
        ) : oracle.phase === Phase.FactProposal ? (
          <>
            <SubmitFactForm pubkey={pubkey} oracle={oracle} refetch={refetch} />
            <FinalizeControl
              title="Advance phase"
              description="Once the fact-proposal window has closed, tick the oracle into fact voting."
              verb="Advance phase"
              successVerb="Advanced"
              refetch={refetch}
              build={() => buildAdvancePhaseIxs({ oracle: pubkey })}
            />
          </>
        ) : oracle.phase === Phase.FactVoting ? (
          <>
            <Note>
              This oracle is in fact voting — approve or flag facts using the controls on each fact
              in the Facts section below.
            </Note>
            <FinalizeControl
              title="Finalize facts"
              description="Once the fact-voting window has closed, settle the facts and crank the oracle into the AI-claim round."
              verb="Finalize facts"
              successVerb="Finalized"
              refetch={refetch}
              build={() =>
                buildFinalizeFactsIxs({
                  oracle: pubkey,
                  kassMint,
                  facts: factsTail,
                  oracleNonce: recallNonce(pubkey) ?? undefined,
                })
              }
            />
          </>
        ) : oracle.phase === Phase.AiClaim ? (
          <>
            <SubmitAiClaimForm pubkey={pubkey} oracle={oracle} refetch={refetch} />
            <FinalizeControl
              title="Finalize AI claims"
              description="Once the AI-claim window has closed, finalize the submitted claims and crank the oracle into the challenge round."
              verb="Finalize AI claims"
              successVerb="Finalized"
              refetch={refetch}
              build={() => buildFinalizeAiClaimsIxs({ oracle: pubkey, proposers })}
            />
          </>
        ) : oracle.phase === Phase.Challenge || oracle.phase === Phase.FinalRecompute ? (
          <>
            <ChallengeControl pubkey={pubkey} oracle={oracle} market={market} refetch={refetch} />
            <FinalizeControl
              title="Finalize oracle"
              description="Once the challenge window has closed (and no challenge markets remain open), finalize the oracle to its resolved option."
              verb="Finalize oracle"
              successVerb="Finalized"
              nearCap={proposersNearCap}
              refetch={refetch}
              build={() =>
                buildFinalizeOracleIxs({
                  oracle: pubkey,
                  kassMint,
                  proposers,
                  oracleNonce: recallNonce(pubkey) ?? undefined,
                })
              }
            />
          </>
        ) : oracle.phase === Phase.Resolved || oracle.phase === Phase.InvalidDeadend ? (
          <>
            <Note>
              This oracle is settled — participants claim their KASS payouts using the controls on
              each proposer, fact and AI-claim card below.
            </Note>
            <SweepControl oracle={pubkey} oracleAccount={oracle} refetch={refetch} />
          </>
        ) : (
          <Note>Participation is closed — this oracle is in the {phaseLabel} phase.</Note>
        )}
      </div>
    </section>
  )
}

export default OracleActions
