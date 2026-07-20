/**
 * Render coverage for the market-detail page's EXPANDED Trade-tab visibility:
 * a still-Funding outcome's OWN page now still exposes a Trade tab as soon as
 * ANY sibling outcome in its categorical group is Active — since GroupTradePanel
 * lets that sibling be traded from here without navigating to its own page. This
 * is the flip side of `marketDetailTabs.render.test.tsx` (which covers the
 * lone/Active-current-market case); `TradePanel` is stubbed here too (it needs
 * wallet/indexer context this structural test doesn't provide) — the outcome
 * selector's own render behavior is covered by `groupTradePanel.render.test.tsx`.
 */
import { vi } from 'vitest'
import { MarketStatus } from '@kassandra-market/markets'
import { Phase } from '@kassandra-market/oracles'

const PUB = 'Market0111111111111111111111111111111111111'
const SIBLING_PUB = 'Market2111111111111111111111111111111111111'
const ORACLE = 'Orac1e1111111111111111111111111111111111111'

const fundingDetail = {
  pubkey: PUB,
  market: {
    status: MarketStatus.Funding,
    outcomeIndex: 0,
    settled: false,
    openContributions: 0,
    totalContributed: 100_000_000n,
    minLiquidity: 500_000_000n,
    feeBps: 100,
    feeCollected: false,
    oracle: { toString: () => ORACLE },
  },
  oracle: { optionsCount: 3, phase: Phase.Challenge, resolvedOption: -1 },
  reserves: null,
  contributions: [],
}

const activeSibling = {
  pubkey: SIBLING_PUB,
  market: { oracle: { toString: () => ORACLE }, outcomeIndex: 2, status: MarketStatus.Active },
  reserves: { base: 600_000_000n, quote: 400_000_000n },
  oracleOptionsCount: 3,
}

vi.mock('../src/market/hooks/useMarketDetail', () => ({
  useMarketDetail: () => ({
    data: fundingDetail,
    loading: false,
    error: undefined,
    refetch: () => {},
    refetchAfterWrite: () => {},
  }),
  useConfig: () => ({ data: undefined, loading: false, error: undefined, refetch: () => {} }),
}))
vi.mock('../src/market/hooks/useOracleGroup', () => ({
  useOracleGroup: () => ({
    siblings: [fundingDetail, activeSibling],
    isGroup: true,
    funding: [fundingDetail],
    active: [activeSibling],
    claimable: [],
    depositable: [fundingDetail],
    loading: false,
    refetch: () => {},
  }),
}))
vi.mock('../src/components/markets/actions/TradePanel', () => ({
  TradePanel: () => null,
}))
// The Liquidity tab (default here, since the current outcome is Funding) reads
// the connected wallet for "Your stake"; render it disconnected (no
// WalletProvider in this static render).
vi.mock('@solana/wallet-adapter-react', () => ({
  useWallet: () => ({ publicKey: null }),
}))
vi.mock('../src/components/markets/actions/GroupLiquidityPanel', () => ({
  GroupLiquidityPanel: () => null,
}))
vi.mock('../src/components/markets/actions/MarketActions', () => ({
  MarketLiquidityActions: () => null,
  MarketLifecycleActions: () => null,
}))

import React from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { MemoryRouter, Route, Routes } from 'react-router-dom'
import { describe, expect, it } from 'vitest'

import MarketDetail from '../src/pages/MarketDetail'

function render(query = ''): string {
  return renderToStaticMarkup(
    <MemoryRouter initialEntries={[`/markets/${PUB}${query}`]}>
      <Routes>
        <Route path="/markets/:pubkey" element={<MarketDetail />} />
      </Routes>
    </MemoryRouter>,
  )
}

describe('MarketDetail tabs — Funding market with an Active sibling', () => {
  it('exposes a Trade tab even though the CURRENT outcome is still Funding', () => {
    const html = render()
    expect(html).toMatch(/role="tab"[^>]*>(?:(?!<\/button>).)*Trade/)
  })

  it('still defaults to the Liquidity panel, not Trade — Trade is one click away, not forced', () => {
    const html = render()
    expect(html).toContain('role="tabpanel" id="panel-liquidity"')
    expect(html).not.toContain('role="tabpanel" id="panel-trade"')
  })

  it('opens the Trade panel when explicitly selected via ?tab=trade', () => {
    const html = render('?tab=trade')
    expect(html).toContain('role="tabpanel" id="panel-trade"')
  })
})
