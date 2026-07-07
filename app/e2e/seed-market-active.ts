/**
 * Seed an ACTIVE market with a live cYES/cNO pool + drive real swaps against it —
 * the on-chain fixture the candle e2e needs. Reuses the oracle {@link SeedCtx}
 * (one surfpool node, one KASS mint) and the market deploy/init from
 * {@link deployAndInitMarket}.
 *
 * The flow mirrors `sdk-market/test/lifecycle-active.e2e.test.ts`:
 *   createMarket(seed = floor) → compose(question + vault + amm) → activate → the
 *   pool is seeded and the market is Active. A {@link swapOnPool} helper then moves
 *   the price on demand (the e2e fires these AFTER the indexer has subscribed, so
 *   each swap's account update is captured as a fresh candle point).
 */
import { Address, ComputeBudgetProgram } from '@solana/web3.js'

import { TOKEN_PROGRAM_ID, associatedTokenAccount } from '@kassandra/sdk'
import {
  MarketStatus,
  createMarket,
  flows,
  metadao,
  pda as marketPda,
} from '@kassandra-market/sdk'

type MarketRefs = flows.MarketRefs

import { tokenAccountBytes, toHex } from '../../sdk/test/surfpool/harness.ts'
import { sendIx, sendIxs, type SeedCtx } from './seed.ts'
import { deployAndInitMarket, MIN_LIQUIDITY } from './seed-market.ts'

const COMPOSE_CU = 400_000
const ACTIVATE_CU = 1_400_000
const TRADE_CU = 400_000

/** A running active market: its address, the compose refs, and the payer's ATAs. */
export interface ActiveMarketSeed {
  market: string
  refs: MarketRefs
  /** The payer's cYES / cNO ATAs (the swapper's inventory for {@link swapOnPool}). */
  cyesAta: Address
  cnoAta: Address
}

/** Prepend a compute-unit-limit ix so the compose/activate/swap CPIs fit. */
function withCu(units: number, ...ixs: Parameters<typeof sendIxs>[1]): Parameters<typeof sendIxs>[1] {
  return [ComputeBudgetProgram.setComputeUnitLimit({ units }), ...ixs]
}

/**
 * Create a market on `oracle` (outcome 0) funded to the floor, compose its MetaDAO
 * question/vault/AMM, activate it, and hand the payer a split cYES+cNO inventory to
 * trade with. Returns the {@link ActiveMarketSeed}. Throws if activation didn't take.
 */
export async function seedActiveMarket(ctx: SeedCtx, oracle: string): Promise<ActiveMarketSeed> {
  const h = ctx.harness
  const payer = ctx.payer.publicKey.toString()
  const kassMint = ctx.kassMint.publicKey.toString()

  const payerKass = await deployAndInitMarket(ctx)

  // createMarket with seed == floor funds the market fully in one shot (Funding →
  // activatable). Outcome 0 = YES pays if the oracle resolves to option 0.
  const market = (await marketPda.market(oracle, 0)).address.toString()
  await sendIx(
    ctx,
    await createMarket({
      creator: payer,
      oracle,
      kassMint,
      creatorKassAta: payerKass,
      seedAmount: MIN_LIQUIDITY,
      outcomeIndex: 0,
    }),
  )

  // Compose the MetaDAO market (3 ixs), then activate (drains escrow → seeds pool).
  const { instructions: composeIxs, refs } = await flows.composeMarketInstructions({
    market,
    oracle,
    kassMint,
    payer,
  })
  await sendIxs(ctx, withCu(COMPOSE_CU, composeIxs[0]))
  await sendIxs(ctx, withCu(COMPOSE_CU, composeIxs[1]))
  await sendIxs(ctx, withCu(COMPOSE_CU, composeIxs[2]))
  await sendIxs(ctx, withCu(ACTIVATE_CU, await flows.activateInstruction({ refs, payer })))

  // Fabricate the payer's cYES / cNO ATAs (empty), then split KASS into a cYES+cNO
  // inventory so the swap helper can push the price either way.
  const cyesAta = (await associatedTokenAccount(payer, refs.yesMint.toString())).address
  const cnoAta = (await associatedTokenAccount(payer, refs.noMint.toString())).address
  for (const [ata, mint] of [
    [cyesAta, refs.yesMint],
    [cnoAta, refs.noMint],
  ] as const) {
    await h.setAccount(ata.toString(), {
      lamports: 5_000_000,
      owner: TOKEN_PROGRAM_ID.toString(),
      executable: false,
      data: toHex(tokenAccountBytes(mint.toBytes(), ctx.payer.publicKey.toBytes(), 0n)),
    })
  }
  await sendIxs(
    ctx,
    withCu(
      TRADE_CU,
      await metadao.splitTokens({
        question: refs.question,
        vault: refs.vault,
        vaultUnderlyingAta: refs.vaultUnderlyingAta,
        authority: payer,
        userUnderlyingAta: payerKass,
        conditionalMints: [refs.yesMint, refs.noMint],
        userConditionalAtas: [cyesAta, cnoAta],
        amount: 5_000_000_000n, // 5 KASS of each conditional leg
      }),
    ),
  )

  // Verify Active: the `Market.status` byte sits at offset 154 (account_type[1] +
  // _pad_hdr[7] + 4×Pubkey[128] + min_liquidity[8] + total_contributed[8] +
  // open_contributions[2]). MarketStatus.Active == 1.
  const info = await h.connection.getAccountInfo(new Address(market))
  if (!info || info.data[154] !== MarketStatus.Active) {
    throw new Error(`market ${market} did not reach Active after activate (status=${info?.data[154]})`)
  }
  return { market, refs, cyesAta, cnoAta }
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
  await sendIxs(
    ctx,
    withCu(
      TRADE_CU,
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
    ),
  )
}
