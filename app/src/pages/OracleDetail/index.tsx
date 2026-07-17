import { useEffect, useMemo, useState } from 'react'
import { useParams } from 'react-router-dom'
import { Phase } from '@kassandra-market/oracles'
import { Button, Card, EyebrowTag, Tabs, TabPanel, type TabItem } from '../../components/ui'
import { PhaseChip } from '../../components/oracles/PhaseChip'
import { PhaseTimeline } from '../../components/oracles/PhaseTimeline'
import { OracleEconomics } from '../../components/oracles/OracleEconomics'
import { ChallengeMarketPanel } from '../../components/oracles/ChallengeMarketPanel'
import { ChallengeControl } from '../../components/oracles/actions/ChallengeControl'
import { ChallengeTradeControls } from '../../components/oracles/actions/ChallengeTradeControls'
import { Truncated } from '../../components/oracles/Truncated'
import { ActivityFeed } from '../../components/oracles/ActivityFeed'
import { OracleActions } from '../../components/oracles/actions'
import { isIndexerConfigured } from '../../data/indexer'
import { useOracleDetail } from '../../hooks/useOracles'
import { useOracleMeta } from '../../hooks/useOracleMeta'
import { OracleNotFoundError } from '../../data/oracles'
import { CLUSTER_LABELS, useCluster } from '../../lib/cluster'
import { RESOLVED_OPTION_NONE, relativeDeadline, windowLabel } from '../../lib/oracleView'
import type { SettleCtx } from './helpers'
import { BackLink, Row, Section, VerdictBanner } from './primitives'
import { AiClaimCard, FactCard, MarketCard, ProposerCard } from './cards'

const emptyNote = (text: string) => (
  <p className="font-inter text-[14px] text-driftwood">{text}</p>
)

/** Phases whose window elapsing unlocks a permissionless advance/finalize crank. */
const ADVANCEABLE_PHASES = new Set<Phase>([
  Phase.Proposal,
  Phase.FactProposal,
  Phase.FactVoting,
  Phase.AiClaim,
  Phase.Challenge,
  Phase.FinalRecompute,
])

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

      {/* `data` WINS over a transient `loading`: a post-write refetch flips
          `loading` true while keeping the prior data, so preferring data here
          keeps OracleBody mounted (its active tab + form state survive) instead
          of blanking to the skeleton and remounting on every write. The loading
          text therefore shows only on the FIRST load (data still undefined); an
          errored refetch clears data (useAsync) and falls through to not-found /
          error. */}
      {data ? (
        <OracleBody detail={data} refetch={refetch} />
      ) : loading ? (
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
  // A fact is contestable while the challenge round is open and no market exists yet.
  const contestOpen = tradeOpen && market === undefined

  // Coarse wall-clock tick (no per-second timer needed for a tab dot) so the
  // "can advance" indicator lights on its own once the phase window elapses.
  const [nowSec, setNowSec] = useState(() => Math.floor(Date.now() / 1000))
  useEffect(() => {
    const id = setInterval(() => setNowSec(Math.floor(Date.now() / 1000)), 15_000)
    return () => clearInterval(id)
  }, [])
  // The oracle can be cranked forward once an advanceable phase's window has closed.
  // (`phase` is `Phase | undefined` — an unreadable/unknown phase can't advance.)
  const canAdvance =
    oracle.phase !== undefined &&
    ADVANCEABLE_PHASES.has(oracle.phase) &&
    oracle.deadline <= BigInt(nowSec)

  // Four sections: the at-a-glance Overview (lifecycle + economics), the Facts
  // column, the Manage participate surface (forms + crank + the challenge market
  // when there's contestation), and the Details record (proposers, AI claims,
  // activity, parameters, accounts). The Manage tab flags a dot when there's an
  // open challenge (chestnut) or the oracle is ready to advance (ember).
  const indexerOn = isIndexerConfigured()
  const tabs = useMemo<TabItem[]>(() => {
    const detailsCount = proposers.length + aiClaims.length
    return [
      { id: 'overview', label: 'Overview' },
      { id: 'facts', label: 'Facts', count: facts.length },
      { id: 'manage', label: 'Manage', dot: market ? 'chestnut' : canAdvance ? 'ember' : null },
      { id: 'details', label: 'Details', count: detailsCount || undefined },
    ]
  }, [facts.length, proposers.length, aiClaims.length, market, canAdvance])

  const [tab, setTab] = useState('overview')
  const activeTab = tabs.some((t) => t.id === tab) ? tab : 'overview'

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
              ? `Resolved to “${options[oracle.resolvedOption]?.trim() || `option ${oracle.resolvedOption}`}”`
              : 'Resolved with no valid option (dead-end)'}
          </p>
        ) : null}
      </header>

      <div className="mt-8">
        <Tabs items={tabs} value={activeTab} onChange={setTab} ariaLabel="Oracle sections" />
      </div>

      {/* Overview — at-a-glance: the lifecycle (verdict + timeline) and the revamped
          economics. Actions (advance/finalize + forms) live in the Manage tab. */}
      <TabPanel
        id="overview"
        active={activeTab === 'overview'}
        className="tab-enter mt-6 flex flex-col gap-6"
      >
        <VerdictBanner oracle={oracle} />
        <PhaseTimeline oracle={oracle} />
        <OracleEconomics oracle={oracle} proposers={proposers.map((p) => p.proposer)} />
      </TabPanel>

      {/* Facts — the column of fact cards: details + state chips, the per-fact vote
          (FactVoting) / claim (terminal) controls, and a jump to Manage to contest a
          surviving fact with a market while the challenge round is open. */}
      <TabPanel id="facts" active={activeTab === 'facts'} className="tab-enter mt-6">
        {facts.length === 0 ? (
          emptyNote('No facts submitted for this oracle.')
        ) : (
          <div className="grid grid-cols-1 gap-4 lg:grid-cols-2">
            {facts.map((f) => (
              <FactCard
                key={f.pubkey}
                pubkey={f.pubkey}
                fact={f.fact}
                voting={votingOpen ? { oracle: pubkey, kassMint: oracle.kassMint, refetch } : undefined}
                settle={settle}
                contest={contestOpen && f.fact.agreed ? () => setTab('manage') : undefined}
              />
            ))}
          </div>
        )}
      </TabPanel>

      {/* Manage — the phase-gated participate forms, plus the challenge market when
          there's contestation: an open market (card + read panel + trade), or the
          challenge round open to compose one. */}
      <TabPanel
        id="manage"
        active={activeTab === 'manage'}
        className="tab-enter mt-6 flex flex-col gap-6"
      >
        <OracleActions
          pubkey={pubkey}
          oracle={oracle}
          refetch={refetch}
          proposers={proposers.map((p) => p.pubkey)}
          facts={facts.map((f) => f.pubkey)}
        />
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
        ) : tradeOpen ? (
          <ChallengeControl pubkey={pubkey} oracle={oracle} market={undefined} refetch={refetch} />
        ) : null}
      </TabPanel>

      {/* Details — proposers, AI claims, activity (indexer-gated), and the readable
          parameters + account bindings. */}
      <TabPanel id="details" active={activeTab === 'details'} className="tab-enter mt-6">
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

        {indexerOn ? (
          <Section title="Activity">
            <ActivityFeed oracle={pubkey} />
          </Section>
        ) : null}

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
      </TabPanel>
    </>
  )
}
