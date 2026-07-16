/**
 * Render coverage for surfacing the human-readable QUESTION + option labels
 * (on-chain oracle metadata) across the market surfaces, with a graceful
 * pubkey/index fallback when metadata is absent:
 *   - MarketCard: subject title + "Pays YES on <label>" (else short pubkey).
 *   - CategoricalCard: subject title + option-label rows.
 *   - MarketDetail header: the question as h1 + the bound option label in words.
 */
import { vi } from 'vitest'
import { MarketStatus } from '@kassandra-market/markets'
import { Phase } from '@kassandra-market/oracles'

const ORACLE = 'Orac1e1111111111111111111111111111111111111'
const META = { subject: 'Will it rain in Paris on Bastille Day?', options: ['Rain', 'No rain'] }

// MarketDetail reads the question via useOracleMeta — mock it to a ready map.
vi.mock('../src/hooks/useOracleMeta', () => ({
  useOracleMeta: () => new Map([[ORACLE, META]]),
}))

const PUB0 = 'Market00000000000000000000000000000000000000'
const detail = {
  pubkey: PUB0,
  market: {
    status: MarketStatus.Active,
    outcomeIndex: 0,
    settled: false,
    openContributions: 0,
    totalContributed: 5n,
    minLiquidity: 10n,
    feeBps: 0,
    feeCollected: false,
    oracle: { toString: () => ORACLE },
  },
  oracle: { optionsCount: 2, phase: Phase.Challenge, resolvedOption: -1 },
  reserves: { base: 6n, quote: 4n },
  contributions: [],
}

vi.mock('../src/market/hooks/useMarketDetail', () => ({
  useMarketDetail: () => ({
    data: detail,
    loading: false,
    error: undefined,
    refetch: () => {},
    refetchAfterWrite: () => {},
  }),
  useConfig: () => ({ data: undefined, loading: false, error: undefined, refetch: () => {} }),
}))

// The MarketDetail default tab is Trade; stub the trade surface (it needs wallet +
// indexer context) so the header-focused render doesn't crash.
vi.mock('../src/components/markets/actions/TradePanel', () => ({
  TradePanel: () => null,
}))

import React from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { MemoryRouter, Route, Routes } from 'react-router-dom'
import { describe, expect, it } from 'vitest'

import { MarketCard } from '../src/components/markets/MarketCard'
import { CategoricalCard } from '../src/components/markets/CategoricalCard'
import MarketDetail from '../src/pages/MarketDetail'

function inRouter(node: React.ReactElement): string {
  return renderToStaticMarkup(<MemoryRouter>{node}</MemoryRouter>)
}

function summary(outcomeIndex: number, pubkey: string) {
  return {
    pubkey,
    market: {
      status: MarketStatus.Active,
      outcomeIndex,
      oracle: { toString: () => ORACLE },
      totalContributed: 5n,
      minLiquidity: 10n,
    },
    reserves: { base: 6n, quote: 4n },
  } as never
}

describe('MarketCard question/label', () => {
  it('leads with the question and names the bound outcome in words', () => {
    const html = inRouter(<MarketCard summary={summary(1, PUB0)} meta={META} />)
    expect(html).toContain('Will it rain in Paris on Bastille Day?')
    expect(html).toContain('No rain') // options[1] is the bound outcome
    // The title is the question (a serif h3), not the mono pubkey truncation.
    expect(html).toMatch(/<h3[^>]*font-serif[^>]*>Will it rain/)
    expect(html).not.toMatch(/<h3[^>]*font-mono/)
  })

  it('degrades to the short pubkey + numeric outcome without metadata', () => {
    const html = inRouter(<MarketCard summary={summary(1, PUB0)} />)
    expect(html).toContain('Outcome 1')
    expect(html).toMatch(/Market0000/) // truncated pubkey title
  })
})

describe('CategoricalCard question/labels', () => {
  it('titles by the question and labels each outcome row', () => {
    const group = {
      oracle: ORACLE,
      optionsCount: 2,
      markets: [summary(0, 'Ma0'), summary(1, 'Ma1')],
    } as never
    const html = inRouter(<CategoricalCard group={group} meta={META} />)
    expect(html).toContain('Will it rain in Paris on Bastille Day?')
    expect(html).toContain('Rain')
    expect(html).toContain('No rain')
  })
})

describe('MarketDetail header question', () => {
  it('renders the question as the title and the bound label in the binding text', () => {
    const html = renderToStaticMarkup(
      <MemoryRouter initialEntries={[`/markets/${PUB0}`]}>
        <Routes>
          <Route path="/markets/:pubkey" element={<MarketDetail />} />
        </Routes>
      </MemoryRouter>,
    )
    expect(html).toMatch(/<h1[^>]*>Will it rain in Paris on Bastille Day\?<\/h1>/)
    // The outcome this market pays YES on is named in words (options[0] = "Rain").
    expect(html).toContain('“Rain”')
  })
})
