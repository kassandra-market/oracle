/**
 * Render coverage for the market-detail TAB STRUCTURE. The data hook is mocked to
 * return a ready Active market, and we assert (via `renderToStaticMarkup`) the
 * grouped tab bar: an Active market exposes a Trade tab AND a Liquidity tab (the
 * Liquidity tab must be present for Active markets, not just Funding), plus the
 * always-on Manage / Details — and NO standalone Overview tab (its funding /
 * implied-price / oracle panels now live in Liquidity + Details). The default tab
 * is Trade, so only the Trade panel body renders; the heavy Liquidity/Manage
 * panels stay dormant. `TradePanel` is stubbed (it needs wallet/indexer context).
 */
import { vi } from 'vitest'
import { MarketStatus } from '@kassandra-market/markets'
import { Phase } from '@kassandra-market/oracles'

const PUB = 'Market11111111111111111111111111111111111111'

const activeDetail = {
  pubkey: PUB,
  market: {
    status: MarketStatus.Active,
    outcomeIndex: 0,
    settled: false,
    openContributions: 0,
    totalContributed: 500_000_000n,
    minLiquidity: 1_000_000_000n,
    feeBps: 100,
    feeCollected: false,
    oracle: { toString: () => 'Orac1e1111111111111111111111111111111111111' },
  },
  oracle: { optionsCount: 2, phase: Phase.Challenge, resolvedOption: -1 },
  reserves: { base: 640_000_000n, quote: 360_000_000n },
  contributions: [],
}

vi.mock('../src/market/hooks/useMarketDetail', () => ({
  useMarketDetail: () => ({
    data: activeDetail,
    loading: false,
    error: undefined,
    refetch: () => {},
    refetchAfterWrite: () => {},
  }),
  useConfig: () => ({ data: undefined, loading: false, error: undefined, refetch: () => {} }),
}))

// Stub the Trade surface — it's the default panel now, but it pulls in wallet +
// indexer context this lightweight structural test doesn't provide.
vi.mock('../src/components/markets/actions/TradePanel', () => ({
  TradePanel: () => null,
}))

import React from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { MemoryRouter, Route, Routes } from 'react-router-dom'
import { describe, expect, it } from 'vitest'

import MarketDetail from '../src/pages/MarketDetail'

function render(): string {
  return renderToStaticMarkup(
    <MemoryRouter initialEntries={[`/markets/${PUB}`]}>
      <Routes>
        <Route path="/markets/:pubkey" element={<MarketDetail />} />
      </Routes>
    </MemoryRouter>,
  )
}

describe('MarketDetail tabs', () => {
  it('exposes Trade + Liquidity + Manage + Details tabs and no Overview tab', () => {
    const html = render()
    for (const label of ['Trade', 'Liquidity', 'Manage', 'Details']) {
      expect(html).toMatch(new RegExp(`role="tab"[^>]*>(?:(?!</button>).)*${label}`))
    }
    expect(html).not.toMatch(/role="tab"[^>]*>(?:(?!<\/button>).)*Overview/)
  })

  it('defaults to the Trade panel; other panels stay dormant', () => {
    const html = render()
    expect(html).toContain('role="tabpanel" id="panel-trade"')
    // Inactive panels render null — the Liquidity panel body is absent by default.
    expect(html).not.toContain('role="tabpanel" id="panel-liquidity"')
  })
})
