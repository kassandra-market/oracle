/**
 * Candle-e2e market helpers on top of the shared market seeding in
 * `seed-market.ts`: stand up an ACTIVE market with a split trading inventory, and
 * drive swaps against its pool to move the price so the indexer's ws price
 * subscriber records real candle points.
 */
import { ComputeBudgetProgram } from '@solana/web3.js'

import { metadao } from '@kassandra-market/sdk'

import { sendIxs, type SeedCtx } from './seed.ts'
import {
  createAndActivateMarket,
  deployAndInitMarket,
  type ActiveMarketSeed,
} from './seed-market.ts'

export type { ActiveMarketSeed } from './seed-market.ts'

/** Compute-unit limit for the split/swap trade CPIs. */
const TRADE_CU = 400_000

/**
 * Deploy + init the market program, then create + activate one market on `oracle`
 * (outcome 0) with a split cYES+cNO trading inventory — the candle e2e's entry
 * point. Throws if activation didn't take.
 */
export async function seedActiveMarket(ctx: SeedCtx, oracle: string): Promise<ActiveMarketSeed> {
  const payerKass = await deployAndInitMarket(ctx)
  return createAndActivateMarket(ctx, oracle, payerKass, { split: true })
}

/** Which leg to push: `"down"` sells cYES (P(YES)↓), `"up"` buys cYES (P(YES)↑). */
export type SwapDir = 'down' | 'up'

/**
 * Swap `amount` (raw units) on the seeded pool to move the price. Fired by the e2e
 * AFTER the indexer has subscribed, so each swap's AMM account update lands as a
 * fresh candle point — the proof the subscription path captures live trades.
 */
export async function swapOnPool(
  ctx: SeedCtx,
  seed: ActiveMarketSeed,
  dir: SwapDir,
  amount: bigint,
): Promise<void> {
  const payer = ctx.payer.publicKey.toString()
  await sendIxs(ctx, [
    ComputeBudgetProgram.setComputeUnitLimit({ units: TRADE_CU }),
    await metadao.swap({
      payer,
      baseMint: seed.refs.yesMint,
      quoteMint: seed.refs.noMint,
      // Sell = base→quote (cYES in) drops P(YES); Buy = quote→base (cNO in) lifts it.
      swapType: dir === 'down' ? metadao.SwapType.Sell : metadao.SwapType.Buy,
      inputAmount: amount,
      outputAmountMin: 0n,
      userBase: seed.cyesAta,
      userQuote: seed.cnoAta,
    }),
  ])
}
