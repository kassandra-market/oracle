import { Phase, type Oracle } from '@kassandra-market/oracles'
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
import { FinalizeControl } from './FinalizeControl'
import { SweepControl } from './SweepControl'

/** Muted note for phases whose participation happens elsewhere (or is closed). */
function Note({ children }: { children: React.ReactNode }) {
  return (
    <Card>
      <p className="font-inter text-[14px] text-driftwood">{children}</p>
    </Card>
  )
}

/**
 * The Manage-tab PARTICIPATION surface — the wallet-signed write FORMS plus the
 * permissionless phase-advance CRANK, phase-gated and co-located so both live in
 * one place: propose + finalize-proposals in Proposal, submit-fact + advance-to-
 * fact-voting in FactProposal, submit-AI-claim + finalize-AI-claims in AiClaim, the
 * finalize-facts / finalize-oracle cranks in FactVoting / Challenge, and the sweep
 * once settled. Voting is per-fact (on the Facts tab); the challenge market is
 * composed beside this in Manage; the Overview mirrors the crank's countdown and
 * routes here when it unlocks. Read-only browsing is intact when disconnected.
 *
 * The finalize tails (proposer / fact PDAs) come from the already-fetched detail.
 */
export function OracleActions({
  pubkey,
  oracle,
  refetch,
  proposers = [],
  facts = [],
}: {
  pubkey: string
  oracle: Oracle
  refetch: () => void
  /** Proposer-PDA pubkeys (the finalize proposer tail). */
  proposers?: string[]
  /** Fact-PDA pubkeys (the finalize-facts tail). */
  facts?: string[]
}) {
  const kassMint = oracle.kassMint
  // The full-set finalizes overflow a legacy tx past MAX_LEGACY_TAIL proposers.
  const proposersNearCap = proposers.length > MAX_LEGACY_TAIL
  // finalize_facts uses the fact tail, or the proposers in the no-facts dead-end.
  const factsTail = facts.length > 0 ? facts : proposers

  switch (oracle.phase) {
    case Phase.Proposal:
      return (
        <div className="flex flex-col gap-4">
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
        </div>
      )

    case Phase.FactProposal:
      return (
        <div className="flex flex-col gap-4">
          <SubmitFactForm pubkey={pubkey} oracle={oracle} refetch={refetch} />
          <FinalizeControl
            title="Advance to fact voting"
            description="Once the fact-proposal window has closed, tick the oracle into fact voting."
            verb="Advance to fact voting"
            successVerb="Advanced"
            refetch={refetch}
            build={() => buildAdvancePhaseIxs({ oracle: pubkey })}
          />
        </div>
      )

    case Phase.FactVoting:
      return (
        <div className="flex flex-col gap-4">
          <Note>
            This oracle is in fact voting — approve or flag each fact using the controls on its card
            in the Facts tab.
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
        </div>
      )

    case Phase.AiClaim:
      return (
        <div className="flex flex-col gap-4">
          <SubmitAiClaimForm pubkey={pubkey} oracle={oracle} refetch={refetch} />
          <FinalizeControl
            title="Finalize AI claims"
            description="Once the AI-claim window has closed, finalize the submitted claims and crank the oracle into the challenge round."
            verb="Finalize AI claims"
            successVerb="Finalized"
            refetch={refetch}
            build={() => buildFinalizeAiClaimsIxs({ oracle: pubkey, proposers })}
          />
        </div>
      )

    case Phase.Challenge:
    case Phase.FinalRecompute:
      return (
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
      )

    case Phase.Resolved:
    case Phase.InvalidDeadend:
      return (
        <div className="flex flex-col gap-4">
          <Note>
            This oracle is settled — claim your KASS payouts from the fact and proposer cards, then
            anyone can sweep the remainder.
          </Note>
          <SweepControl oracle={pubkey} oracleAccount={oracle} refetch={refetch} />
        </div>
      )

    default:
      return (
        <Note>Participation is closed — this oracle is in the {phaseView(oracle.phase).label} phase.</Note>
      )
  }
}

export default OracleActions
