/**
 * Query hook over the CU1 read layer for a challenge market's pass/fail v0.4
 * `Amm` pools. Same tiny `useEffect`+state shape as `useOracles` (TanStack Query
 * is deliberately NOT a dependency): loading/data with an unmount guard, a manual
 * `refetch`, and a light periodic poll so the live prices/TWAP refresh while the
 * TWAP window is open (NO websocket). `null` market ⇒ no fetch, empty result.
 *
 * Mock affordance: under {@link isMockMode} the pools resolve from
 * {@link mockMarketAmms} WITHOUT touching the connection — the same pattern the
 * oracle hooks use — so the panel is reviewable offline via `?mock`.
 */
import { useCallback, useEffect, useRef, useState } from 'react'
import type { Market } from '@kassandra-market/oracles'
import { useConnection } from '../lib/cluster'
import { fetchMarketAmms, type AmmV04 } from '../data/ammV04'
import { isMockMode, mockMarketAmms } from '../data/mockOracles'

/** Poll interval (ms) while a live, unsettled market is on screen. */
const POLL_MS = 15_000

export interface MarketAmmsState {
  pass: AmmV04 | null
  fail: AmmV04 | null
  loading: boolean
  /** Re-run the fetch immediately (e.g. after a trade/crank, or a retry). */
  refetch: () => void
}

/**
 * Fetch + decode the pass/fail pools for `market`. Re-runs on mount, whenever
 * the market/connection changes, on `refetch`, and on a {@link POLL_MS} interval
 * (only for a live, unsettled market — settled markets and mock mode don't poll).
 */
export function useMarketAmms(market: Market | undefined): MarketAmmsState {
  const { connection } = useConnection()
  const [pass, setPass] = useState<AmmV04 | null>(null)
  const [fail, setFail] = useState<AmmV04 | null>(null)
  const [loading, setLoading] = useState<boolean>(market != null)
  const [nonce, setNonce] = useState(0)

  const refetch = useCallback(() => setNonce((n) => n + 1), [])

  // web3.js Addresses stringify stably; key the effect on the pool identities.
  const passKey = market?.passAmm.toString()
  const failKey = market?.failAmm.toString()
  const mock = isMockMode()
  const live = market != null && !mock && !market.settled

  const marketRef = useRef(market)
  marketRef.current = market

  useEffect(() => {
    const current = marketRef.current
    if (!current) {
      setPass(null)
      setFail(null)
      setLoading(false)
      return
    }
    let active = true
    setLoading(true)
    const run = () => {
      const task = mock
        ? Promise.resolve(mockMarketAmms())
        : fetchMarketAmms(connection, current)
      task.then(
        (res) => {
          if (!active) return
          setPass(res.pass)
          setFail(res.fail)
          setLoading(false)
        },
        () => {
          if (!active) return
          setLoading(false)
        },
      )
    }
    run()
    const timer = live ? setInterval(run, POLL_MS) : undefined
    return () => {
      active = false
      if (timer) clearInterval(timer)
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [connection, passKey, failKey, mock, live, nonce])

  return { pass, fail, loading, refetch }
}
