import { useMemo, type ReactNode } from 'react'
import { Link, useParams } from 'react-router-dom'
import { CLAIM_OPTION_NONE, Phase, pda } from '@kassandra/sdk'
import type { AiClaim, Fact, Market, Oracle, Proposer } from '@kassandra/sdk'
import type { Address } from '@solana/web3.js'
import { AvatarBubble, Button, Card, EyebrowTag } from '../components/ui'
import { Chip } from '../components/oracles/Chip'
import { PhaseChip } from '../components/oracles/PhaseChip'
import { PhaseTimeline } from '../components/oracles/PhaseTimeline'
import { EconomicPanel } from '../components/oracles/EconomicPanel'
import { ChallengeMarketPanel } from '../components/oracles/ChallengeMarketPanel'
import { ChallengeTradeControls } from '../components/oracles/actions/ChallengeTradeControls'
import { Truncated } from '../components/oracles/Truncated'
import { ActivityFeed } from '../components/oracles/ActivityFeed'
import { verdictFor } from '../lib/phaseTimeline'
import { ClaimControl, CloseControl, OracleActions, VoteControl } from '../components/oracles/actions'
import { isIndexerConfigured } from '../data/indexer'
import { useOracleDetail } from '../hooks/useOracles'
import { useOracleMeta } from '../hooks/useOracleMeta'
import { OracleNotFoundError } from '../data/oracles'
import { recallNonce } from '../lib/nonceStore'
import { resolveOracleNonce } from '../data/actions/finalize'
import {
  buildClaimFactIxs,
  buildClaimFactVoteIxs,
  buildClaimProposerIxs,
  buildCloseAiClaimIxs,
  buildCloseMarketIxs,
} from '../data/actions/claims'
import { CLUSTER_LABELS, useCluster } from '../lib/cluster'
import {
  RESOLVED_OPTION_NONE,
  groupDigits,
  hashHex,
  relativeDeadline,
  windowLabel,
} from '../lib/oracleView'

/** A back-to-list link that preserves the mock query param. */
function BackLink({ search }: { search: string }) {
  return (
    <Link
      to={{ pathname: '/oracles', search }}
      className="inline-block font-inter text-[14px] text-sepia underline decoration-pebble underline-offset-4 hover:text-lavender-phosphor focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sepia/40 focus-visible:ring-offset-2 focus-visible:ring-offset-parchment"
    >
      ← All oracles
    </Link>
  )
}

/** A compact labelled statistic tile. */
function Stat({ label, value }: { label: string; value: ReactNode }) {
  return (
    <div className="rounded-card border border-pebble bg-pure-card p-4">
      <div className="font-inter text-[11px] uppercase tracking-[0.06em] text-driftwood">{label}</div>
      <div className="mt-1 font-serif text-subheading font-light text-sepia">{value}</div>
    </div>
  )
}

/**
 * The at-a-glance verdict banner — a calm h2 under the title (NOT a second h1).
 * Resolved reads a confirmed chestnut "Resolved · Option N"; a dead-end reads
 * muted stone; in-flight shows the current phase + a one-line "what's next".
 */
function VerdictBanner({ oracle }: { oracle: Oracle }) {
  const v = verdictFor(oracle)
  // In-flight stays a quiet bronze stripe — the header PhaseChip already carries
  // the single ember "Challenged" spark, so the banner never doubles it up.
  const accent =
    v.kind === 'resolved'
      ? 'border-l-chestnut'
      : v.kind === 'deadend'
        ? 'border-l-stone'
        : 'border-l-bronze'
  const titleClass =
    v.kind === 'resolved' ? 'text-chestnut' : v.kind === 'deadend' ? 'text-stone' : 'text-sepia'
  return (
    <div
      role="status"
      className={`mt-6 rounded-card border border-pebble border-l-4 ${accent} bg-pure-card py-4 pl-5 pr-4`}
    >
      <h2 className={`font-serif text-subheading font-light ${titleClass}`}>{v.title}</h2>
      <p className="mt-1 font-inter text-[13px] text-bronze">{v.detail}</p>
    </div>
  )
}

/** A section wrapper: a serif-lite heading + optional count, then its content. */
function Section({ title, count, children }: { title: string; count?: number; children: ReactNode }) {
  return (
    <section className="mt-14">
      <h2 className="font-serif text-heading-sm font-light text-sepia">
        {title}
        {count != null ? <span className="ml-2 font-inter text-[14px] text-driftwood">({count})</span> : null}
      </h2>
      <div className="mt-4">{children}</div>
    </section>
  )
}

/** Definition row for the readable-parameters + accounts blocks. */
function Row({ term, children }: { term: string; children: ReactNode }) {
  return (
    <div className="flex items-baseline justify-between gap-4 border-b border-pebble py-2 last:border-b-0">
      <dt className="font-inter text-[13px] text-driftwood">{term}</dt>
      <dd className="font-inter text-[14px] text-sepia">{children}</dd>
    </div>
  )
}

const emptyNote = (text: string) => (
  <p className="font-inter text-[14px] text-driftwood">{text}</p>
)

/**
 * Settlement context threaded to the claim / close controls once an oracle is
 * terminal (Resolved / InvalidDeadend). Present ⇒ render the payout controls.
 */
interface SettleCtx {
  oracle: string
  kassMint: Address
  refetch: () => void
}

/** Recall the oracle's create nonce, else recover it via the pure PDA scan (RF1). */
function oracleNonce(oracle: string): Promise<bigint> {
  const recalled = recallNonce(oracle)
  return recalled !== null ? Promise.resolve(recalled) : resolveOracleNonce(oracle)
}

function FactCard({
  pubkey,
  fact,
  voting,
  settle,
}: {
  pubkey: string
  fact: Fact
  /** When set (FactVoting phase), renders the per-fact vote control. */
  voting?: { oracle: string; kassMint: Address; refetch: () => void }
  /** When set (terminal phase), renders the fact-claim + fact-vote-claim controls. */
  settle?: SettleCtx
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
            {groupDigits(fact.approveStake)} / {groupDigits(fact.duplicateStake)}
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

function ProposerCard({
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
            <dd className="text-sepia">{groupDigits(proposer.bond)}</dd>
          </div>
          {proposer.slashedAmount > 0n ? (
            <div className="flex items-baseline justify-between gap-4">
              <dt className="text-driftwood">Slashed</dt>
              <dd className="text-sepia">{groupDigits(proposer.slashedAmount)}</dd>
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

function AiClaimCard({
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

function MarketCard({
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
        <Row term="Challenger USDC (base units)">{groupDigits(market.challengerUsdc)}</Row>
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

/**
 * The oracle detail view at `/oracles/:pubkey` — an editorial layout of one
 * decoded oracle + its facts, proposers, AI claims and challenge market
 * (consumes the FA2 data layer via `useOracleDetail`). Read-only. Loading /
 * error / not-found states.
 */
export default function OracleDetail() {
  const { pubkey } = useParams<{ pubkey: string }>()
  const { cluster } = useCluster()
  const search = typeof window !== 'undefined' ? window.location.search : ''
  const { data, loading, error, refetch } = useOracleDetail(pubkey)

  const notFound = error instanceof OracleNotFoundError

  return (
    <main className="mx-auto max-w-[1000px] px-6 py-16 md:py-20">
      <BackLink search={search} />

      {loading ? (
        <p className="mt-10 font-inter text-[15px] text-bronze" role="status">
          Reading the chain…
        </p>
      ) : notFound ? (
        <div className="mt-10 max-w-[560px]">
          <Card>
            <h1 className="font-serif text-heading-sm font-light text-sepia">Oracle not found</h1>
            <p className="mt-2 font-inter text-[15px] text-bronze">
              No Kassandra oracle lives at this address on{' '}
              <span className="font-medium text-sepia">{CLUSTER_LABELS[cluster]}</span>.
            </p>
            <p className="mt-2 break-all font-mono text-[12px] text-driftwood">{pubkey}</p>
          </Card>
        </div>
      ) : error ? (
        <div className="mt-10 max-w-[560px]">
          <Card>
            <h1 className="font-serif text-heading-sm font-light text-sepia">
              Couldn’t load this oracle
            </h1>
            <p className="mt-2 font-inter text-[15px] text-bronze">{error.message}</p>
            <div className="mt-5">
              <Button variant="GhostOutline" onClick={refetch}>
                Retry
              </Button>
            </div>
          </Card>
        </div>
      ) : data ? (
        <OracleBody detail={data} refetch={refetch} />
      ) : null}
    </main>
  )
}

/** The loaded oracle, laid out editorially. Split out so the states above stay readable. */
function OracleBody({
  detail,
  refetch,
}: {
  detail: NonNullable<ReturnType<typeof useOracleDetail>['data']>
  refetch: () => void
}) {
  const { pubkey, oracle, facts, proposers, aiClaims, market } = detail
  // On-chain plaintext subject + option labels (indexed from oracle_meta).
  const metaItems = useMemo(() => [pubkey], [pubkey])
  const meta = useOracleMeta(metaItems).get(pubkey)
  const options = meta?.options ?? []
  const resolved = oracle.phase === Phase.Resolved
  const hasResolvedOption = resolved && oracle.resolvedOption !== RESOLVED_OPTION_NONE
  const votingOpen = oracle.phase === Phase.FactVoting
  // The trade/crank/settle controls live only while the challenge round is open.
  const tradeOpen = oracle.phase === Phase.Challenge || oracle.phase === Phase.FinalRecompute
  // Terminal phases open the claim / close / sweep payout controls.
  const settleOpen = oracle.phase === Phase.Resolved || oracle.phase === Phase.InvalidDeadend
  const settle: SettleCtx | undefined = settleOpen
    ? { oracle: pubkey, kassMint: oracle.kassMint, refetch }
    : undefined

  return (
    <>
      {/* Header — the SUBJECT (verified question) + its options lead. */}
      <header className="mt-8">
        <EyebrowTag pill>Oracle</EyebrowTag>
        <h1 className="mt-3 font-serif text-heading font-light text-sepia">
          {meta?.subject ?? 'Oracle dispute'}
        </h1>
        {options.length > 0 && (
          <div className="mt-4 flex flex-wrap gap-2" aria-label="Options">
            {options.map((opt, i) => (
              <span
                key={i}
                className="rounded-tag border border-pebble bg-soft-cream px-2.5 py-1 font-inter text-[13px] text-bronze"
              >
                <span className="tabular-nums text-driftwood">{i}</span>
                <span className="mx-1 text-driftwood">·</span>
                {opt}
              </span>
            ))}
          </div>
        )}
        <div className="mt-4 flex flex-wrap items-center gap-x-4 gap-y-2 font-inter text-[13px] text-driftwood">
          <PhaseChip phase={oracle.phase} />
          <span>{relativeDeadline(oracle.deadline)}</span>
          <Truncated value={pubkey} copyable label="oracle address" />
        </div>
        {meta?.uri && (
          <div className="mt-3 flex items-baseline gap-2 font-inter text-[13px] text-driftwood">
            <span>Metadata</span>
            <a
              href={meta.uri}
              target="_blank"
              rel="noreferrer"
              className="text-chestnut underline decoration-dotted underline-offset-2"
            >
              extended JSON
            </a>
            <span className="text-driftwood/70" title="sha256 committed on-chain">
              (hash-verified)
            </span>
          </div>
        )}
        {resolved ? (
          <p className="mt-3 font-inter text-[14px] text-chestnut">
            {hasResolvedOption
              ? `Resolved to option ${oracle.resolvedOption}`
              : 'Resolved with no valid option (dead-end)'}
          </p>
        ) : null}
      </header>

      {/* At-a-glance verdict (h2 banner) + lifecycle timeline — additive header. */}
      <VerdictBanner oracle={oracle} />
      <PhaseTimeline oracle={oracle} />

      {/* Stats */}
      <div className="mt-8 grid grid-cols-2 gap-4 sm:grid-cols-3">
        <Stat label="Options" value={oracle.optionsCount} />
        <Stat label="Proposers" value={oracle.proposerCount} />
        <Stat label="Surviving" value={oracle.survivingCount} />
        <Stat label="Facts" value={oracle.factCount} />
        <Stat label="Settled facts" value={oracle.settledCount} />
        <Stat label="Open challenges" value={oracle.openChallengeCount} />
      </div>

      {/* Bond pool (raw base units, annotated) */}
      <div className="mt-4">
        <Card>
          <div className="font-inter text-[11px] uppercase tracking-[0.06em] text-driftwood">
            Bond pool
          </div>
          <div className="mt-1 font-serif text-heading-sm font-light text-sepia">
            {groupDigits(oracle.bondPool)}
          </div>
          <p className="mt-1 font-inter text-[12px] text-driftwood">
            KASS base units (raw, unscaled) · dispute-bond total {groupDigits(oracle.disputeBondTotal)}
          </p>
        </Card>
      </div>

      {/* Economic picture — flat proportion viz over the decoded economics. */}
      <EconomicPanel oracle={oracle} proposers={proposers.map((p) => p.proposer)} />

      {/* Participate — the wallet-signed write forms + permissionless finalize
          cranks, phase-gated (WF2/RF1). The finalize tails are the proposer /
          fact PDA pubkeys from the already-fetched detail. */}
      <OracleActions
        pubkey={pubkey}
        oracle={oracle}
        refetch={refetch}
        proposers={proposers.map((p) => p.pubkey)}
        facts={facts.map((f) => f.pubkey)}
        market={market}
      />

      {/* Parameters */}
      <Section title="Parameters">
        <Card>
          <dl className="flex flex-col">
            <Row term="Fact quorum">
              {oracle.thresholdNum.toString()} / {oracle.thresholdDen.toString()}
            </Row>
            <Row term="Market margin">
              {oracle.marketThresholdNum.toString()} / {oracle.marketThresholdDen.toString()}
            </Row>
            <Row term="Flip slash">
              {oracle.flipSlashNum.toString()} / {oracle.flipSlashDen.toString()}
            </Row>
            <Row term="Phase window">{windowLabel(oracle.phaseWindow)}</Row>
            <Row term="Proposal window">{windowLabel(oracle.proposalWindow)}</Row>
            <Row term="TWAP window">{windowLabel(oracle.twapWindow)}</Row>
          </dl>
        </Card>
      </Section>

      {/* Facts */}
      <Section title="Facts" count={facts.length}>
        {facts.length === 0 ? (
          emptyNote('No facts submitted for this oracle.')
        ) : (
          <div className="grid grid-cols-1 gap-4 lg:grid-cols-2">
            {facts.map((f) => (
              <FactCard
                key={f.pubkey}
                pubkey={f.pubkey}
                fact={f.fact}
                voting={
                  votingOpen ? { oracle: pubkey, kassMint: oracle.kassMint, refetch } : undefined
                }
                settle={settle}
              />
            ))}
          </div>
        )}
      </Section>

      {/* Proposers */}
      <Section title="Proposers" count={proposers.length}>
        {proposers.length === 0 ? (
          emptyNote('No proposers registered.')
        ) : (
          <div className="grid grid-cols-1 gap-4 lg:grid-cols-2">
            {proposers.map((p) => (
              <ProposerCard key={p.pubkey} pubkey={p.pubkey} proposer={p.proposer} settle={settle} />
            ))}
          </div>
        )}
      </Section>

      {/* AI Claims */}
      <Section title="AI claims" count={aiClaims.length}>
        {aiClaims.length === 0 ? (
          emptyNote('No AI claims submitted.')
        ) : (
          <div className="flex flex-col gap-4">
            {aiClaims.map((c) => (
              <AiClaimCard key={c.pubkey} pubkey={c.pubkey} aiClaim={c.aiClaim} settle={settle} />
            ))}
          </div>
        )}
      </Section>

      {/* Market — the existing card (accounts + close control) plus the CU1
          live visualization panel (prices / TWAP / margin / countdown), additive. */}
      <Section title="Challenge market">
        {market ? (
          <>
            <MarketCard pubkey={market.pubkey} market={market.market} settle={settle} />
            <ChallengeMarketPanel market={market.market} oracle={oracle} />
            {tradeOpen ? (
              <ChallengeTradeControls
                oraclePubkey={pubkey}
                oracle={oracle}
                market={market.market}
                proposers={proposers}
                refetch={refetch}
              />
            ) : null}
          </>
        ) : (
          emptyNote('No challenge market opened for this oracle.')
        )}
      </Section>

      {/* On-chain activity — indexed event history (renders only when the
          indexer backend is configured; otherwise absent). */}
      {isIndexerConfigured() ? (
        <Section title="Activity">
          <ActivityFeed oracle={pubkey} />
        </Section>
      ) : null}

      {/* Accounts */}
      <Section title="Accounts">
        <Card>
          <dl className="flex flex-col">
            <Row term="Creator">
              <Truncated value={oracle.creator.toString()} copyable label="creator" />
            </Row>
            <Row term="KASS mint">
              <Truncated value={oracle.kassMint.toString()} copyable label="KASS mint" />
            </Row>
            <Row term="USDC mint">
              <Truncated value={oracle.usdcMint.toString()} copyable label="USDC mint" />
            </Row>
            <Row term="Stake vault">
              <Truncated value={oracle.stakeVault.toString()} copyable label="stake vault" />
            </Row>
          </dl>
        </Card>
      </Section>
    </>
  )
}
