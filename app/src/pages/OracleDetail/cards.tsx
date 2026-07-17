import { CLAIM_OPTION_NONE, pda } from '@kassandra-market/oracles'
import type { AiClaim, Fact, Market, Proposer } from '@kassandra-market/oracles'
import type { Address } from '@solana/web3.js'
import { AvatarBubble, Card } from '../../components/ui'
import { Chip } from '../../components/oracles/Chip'
import { Truncated } from '../../components/oracles/Truncated'
import { ClaimControl, CloseControl, VoteControl } from '../../components/oracles/actions'
import {
  buildClaimFactIxs,
  buildClaimFactVoteIxs,
  buildClaimProposerIxs,
  buildCloseAiClaimIxs,
  buildCloseMarketIxs,
} from '../../data/actions/claims'
import { formatKass, formatUsdc, hashHex, relativeDeadline } from '../../lib/oracleView'
import { oracleNonce, type SettleCtx } from './helpers'
import { Row } from './primitives'

export function FactCard({
  pubkey,
  fact,
  voting,
  settle,
  contest,
}: {
  pubkey: string
  fact: Fact
  /** When set (FactVoting phase), renders the per-fact vote control. */
  voting?: { oracle: string; kassMint: Address; refetch: () => void }
  /** When set (terminal phase), renders the fact-claim + fact-vote-claim controls. */
  settle?: SettleCtx
  /** When set (Challenge phase, contestable fact), a button that jumps to Manage to
   *  open a challenge market. Kept as a jump (not the flow) — the market lives in Manage. */
  contest?: () => void
}) {
  return (
    <Card className="flex flex-col gap-2">
      <div className="flex flex-wrap items-center gap-2">
        {fact.agreed ? (
          <Chip tone="confirmed">Agreed</Chip>
        ) : (
          <Chip tone="muted">Rejected</Chip>
        )}
        {fact.duplicate ? <Chip>Duplicate</Chip> : null}
        {fact.settled ? <Chip>Settled</Chip> : null}
      </div>
      <dl className="flex flex-col gap-1.5 font-inter text-[13px]">
        <div className="flex items-baseline justify-between gap-4">
          <dt className="text-driftwood">Content hash</dt>
          <dd>
            <Truncated value={hashHex(fact.contentHash)} copyable label="content hash" />
          </dd>
        </div>
        <div className="flex items-baseline justify-between gap-4">
          <dt className="text-driftwood">Fact account</dt>
          <dd>
            <Truncated value={pubkey} copyable label="fact address" />
          </dd>
        </div>
        <div className="flex items-baseline justify-between gap-4">
          <dt className="text-driftwood">Approve / duplicate stake</dt>
          <dd className="text-sepia">
            {formatKass(fact.approveStake)} / {formatKass(fact.duplicateStake)} KASS
          </dd>
        </div>
      </dl>
      <div>
        <div className="font-inter text-[11px] uppercase tracking-[0.06em] text-driftwood">
          URI (off-chain, untrusted)
        </div>
        {/* Untrusted content — rendered as inert text, never auto-fetched or linked. */}
        <p className="mt-0.5 break-all font-mono text-[12px] text-bronze">
          {fact.uri.length > 0 ? fact.uri : '—'}
        </p>
      </div>
      {contest ? (
        <button
          type="button"
          onClick={contest}
          className="self-start rounded-button border border-ember-orange/50 bg-ember-orange/10 px-3 py-1.5 font-inter text-[13px] font-medium text-ember-orange transition-colors hover:bg-ember-orange/20 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ember-orange/40 focus-visible:ring-offset-2 focus-visible:ring-offset-parchment active:scale-[0.98]"
        >
          Contest with a market →
        </button>
      ) : null}
      {voting ? (
        <VoteControl
          oracle={voting.oracle}
          kassMint={voting.kassMint}
          factPubkey={pubkey}
          refetch={voting.refetch}
        />
      ) : null}
      {settle ? (
        <>
          {/* The fact submitter claims the fact stake + reward (rent → them). */}
          <ClaimControl
            authority={fact.proposer.toString()}
            verb="Claim fact payout"
            successVerb="Claimed"
            description="Pull your fact stake + reward out of the vault and close this fact."
            refetch={settle.refetch}
            build={({ connection }) =>
              oracleNonce(settle.oracle).then((oracleNonce) =>
                buildClaimFactIxs({
                  connection,
                  oracleNonce,
                  fact: pubkey,
                  authority: fact.proposer,
                  kassMint: settle.kassMint,
                }),
              )
            }
          />
          {/* Any connected wallet that voted on this fact claims its own vote. */}
          <ClaimControl
            verb="Claim your fact vote"
            successVerb="Claimed"
            description="If you voted on this fact, claim your vote stake ± slash + reward."
            refetch={settle.refetch}
            build={async ({ address, connection }) => {
              const oracleNonceValue = await oracleNonce(settle.oracle)
              const factVote = (await pda.factVote(pubkey, address)).address
              return buildClaimFactVoteIxs({
                connection,
                oracleNonce: oracleNonceValue,
                factVote,
                fact: pubkey,
                voter: address,
                kassMint: settle.kassMint,
              })
            }}
          />
        </>
      ) : null}
    </Card>
  )
}

export function ProposerCard({
  pubkey,
  proposer,
  settle,
}: {
  pubkey: string
  proposer: Proposer
  /** When set (terminal phase), renders the proposer-claim control (authority-gated). */
  settle?: SettleCtx
}) {
  const authority = proposer.authority.toString()
  const hasClaim = proposer.claimOption !== CLAIM_OPTION_NONE
  return (
    <Card className="flex items-start gap-4">
      <AvatarBubble name={authority} size={44} />
      <div className="flex min-w-0 flex-1 flex-col gap-2">
        <div className="flex flex-wrap items-center gap-2">
          {proposer.disqualified ? <Chip tone="muted">Disqualified</Chip> : null}
          {proposer.slashed ? <Chip>Slashed</Chip> : null}
          {proposer.flipped ? <Chip>Flipped</Chip> : null}
          {proposer.aiFinalized ? <Chip>AI finalized</Chip> : null}
          {!proposer.disqualified && !proposer.slashed && !proposer.flipped ? (
            <Chip tone="confirmed">Held</Chip>
          ) : null}
        </div>
        <dl className="flex flex-col gap-1.5 font-inter text-[13px]">
          <div className="flex items-baseline justify-between gap-4">
            <dt className="text-driftwood">Authority</dt>
            <dd>
              <Truncated value={authority} copyable label="proposer authority" />
            </dd>
          </div>
          <div className="flex items-baseline justify-between gap-4">
            <dt className="text-driftwood">Option</dt>
            <dd className="text-sepia">
              {proposer.originalOption}
              {hasClaim && proposer.claimOption !== proposer.originalOption
                ? ` → ${proposer.claimOption}`
                : null}
            </dd>
          </div>
          <div className="flex items-baseline justify-between gap-4">
            <dt className="text-driftwood">Bond</dt>
            <dd className="text-sepia">{formatKass(proposer.bond)} KASS</dd>
          </div>
          {proposer.slashedAmount > 0n ? (
            <div className="flex items-baseline justify-between gap-4">
              <dt className="text-driftwood">Slashed</dt>
              <dd className="text-sepia">{formatKass(proposer.slashedAmount)} KASS</dd>
            </div>
          ) : null}
          <div className="flex items-baseline justify-between gap-4">
            <dt className="text-driftwood">Proposer account</dt>
            <dd>
              <Truncated value={pubkey} copyable label="proposer address" />
            </dd>
          </div>
        </dl>
        {settle ? (
          <ClaimControl
            authority={authority}
            verb="Claim proposer payout"
            successVerb="Claimed"
            description="Pull your bond (± slash) + reward out of the vault and close this proposer."
            refetch={settle.refetch}
            build={({ connection }) =>
              oracleNonce(settle.oracle).then((oracleNonce) =>
                buildClaimProposerIxs({
                  connection,
                  oracleNonce,
                  proposer: pubkey,
                  authority: proposer.authority,
                  kassMint: settle.kassMint,
                }),
              )
            }
          />
        ) : null}
      </div>
    </Card>
  )
}

/** A hash row styled like a TriggerPreviewCard sub-card (cream, driftwood label, mono value). */
function HashRow({ label, value }: { label: string; value: Uint8Array }) {
  return (
    <div className="rounded-tag border border-pebble bg-soft-cream px-3 py-2">
      <div className="font-inter text-[11px] uppercase tracking-[0.06em] text-driftwood">{label}</div>
      <div className="mt-0.5">
        <Truncated value={hashHex(value)} head={8} tail={6} copyable label={label} />
      </div>
    </div>
  )
}

export function AiClaimCard({
  pubkey,
  aiClaim,
  settle,
}: {
  pubkey: string
  aiClaim: AiClaim
  /** When set (terminal phase), renders the permissionless close control. */
  settle?: SettleCtx
}) {
  return (
    <Card className="flex flex-col gap-3">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <span className="font-serif text-subheading font-light text-sepia">
          Claim · option {aiClaim.option}
        </span>
        {aiClaim.challenged ? <Chip tone="ember">Challenged</Chip> : <Chip>Uncontested</Chip>}
      </div>
      <div className="flex items-baseline justify-between gap-4 font-inter text-[13px]">
        <span className="text-driftwood">Submitter</span>
        <Truncated value={aiClaim.authority.toString()} copyable label="submitter" />
      </div>
      <div className="flex items-baseline justify-between gap-4 font-inter text-[13px]">
        <span className="text-driftwood">Claim account</span>
        <Truncated value={pubkey} copyable label="AI claim address" />
      </div>
      <div className="grid grid-cols-1 gap-2 sm:grid-cols-3">
        <HashRow label="Model" value={aiClaim.modelId} />
        <HashRow label="Params" value={aiClaim.paramsHash} />
        <HashRow label="I/O" value={aiClaim.ioHash} />
      </div>
      {settle ? (
        <CloseControl
          verb="Close AI claim"
          successVerb="Closed"
          description="Permissionless — reap this AI claim; its rent refunds to the submitter."
          refetch={settle.refetch}
          build={() =>
            buildCloseAiClaimIxs({
              oracle: settle.oracle,
              aiClaim: pubkey,
              rentRecipient: aiClaim.authority,
            })
          }
        />
      ) : null}
    </Card>
  )
}

export function MarketCard({
  pubkey,
  market,
  settle,
}: {
  pubkey: string
  market: Market
  /** When set (terminal phase) and the market is settled, renders the close control. */
  settle?: SettleCtx
}) {
  return (
    <Card className="flex flex-col gap-3">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <span className="font-serif text-subheading font-light text-sepia">Challenge market</span>
        {market.settled ? <Chip tone="confirmed">Settled</Chip> : <Chip>Open</Chip>}
      </div>
      <dl className="flex flex-col gap-1.5">
        <Row term="Market account">
          <Truncated value={pubkey} copyable label="market address" />
        </Row>
        <Row term="Challenger">
          <Truncated value={market.challenger.toString()} copyable label="challenger" />
        </Row>
        <Row term="Challenger USDC">{formatUsdc(market.challengerUsdc)} USDC</Row>
        <Row term="TWAP window">{relativeDeadline(market.twapEnd)}</Row>
        <Row term="Question">
          <Truncated value={market.question.toString()} copyable label="question" />
        </Row>
      </dl>
      {settle && market.settled ? (
        <CloseControl
          verb="Close market"
          successVerb="Closed"
          description="Permissionless — reap this settled market + escrow; rent refunds to the challenger."
          refetch={settle.refetch}
          build={() =>
            oracleNonce(settle.oracle).then((oracleNonce) =>
              buildCloseMarketIxs({
                oracleNonce,
                market: pubkey,
                rentRecipient: market.challenger,
              }),
            )
          }
        />
      ) : null}
    </Card>
  )
}
