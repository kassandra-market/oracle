/**
 * Market side of the `make dev` seeder — deploys the kassandra-market program +
 * the MetaDAO v0.4 fixtures onto the SAME surfpool node the oracle seeder booted,
 * inits the market `Config`, and pre-creates a spread of demo markets on the
 * already-seeded oracles so the app's `/markets` section has live data.
 *
 * Reuses the oracle {@link SeedCtx} (one surfpool node, one KASS mint): the funded
 * browser wallet already holds KASS on `ctx.kassMint`, so it can contribute to /
 * create markets in the UI immediately. Everything here is signed by `ctx.payer`
 * (via {@link sendIx}); the market program is deployed non-upgradeable (BPFLoader2)
 * with a fabricated `ProgramData` so `init_config`'s upgrade-authority gate passes
 * — mirroring `sdk-market/test/surfpool/harness.ts`.
 */
import { readFileSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

import { Address, ComputeBudgetProgram } from '@solana/web3.js'

import { TOKEN_PROGRAM_ID, associatedTokenAccount } from '@kassandra/sdk'
import {
  BPF_UPGRADEABLE_LOADER_ID,
  MARKET_PROGRAM_ID,
  MarketStatus,
  createMarket,
  flows,
  initConfig,
  metadao,
  pda as marketPda,
} from '@kassandra-market/sdk'

import { toHex, tokenAccountBytes } from '../../sdk/test/surfpool/harness.ts'
import { sendIx, sendIxs, type SeedCtx } from './seed.ts'

type MarketRefs = flows.MarketRefs

const here = dirname(fileURLToPath(import.meta.url))
const MARKET_SO = resolve(here, '../../target/deploy/kassandra_market_program.so')
const FIXTURES_DIR = resolve(here, '../../programs/kassandra-market/tests/fixtures')

/** The deprecated (non-upgradeable) BPF loader: a program account IS its ELF. */
const BPF_LOADER_2 = 'BPFLoader2111111111111111111111111111111111'

/** MetaDAO v0.4 programs the market CPIs, at their canonical mainnet ids. */
const METADAO_FIXTURES = [
  { id: 'VLTX1ishMBbcX3rdBWGssxawAo1Q2X2qxYFYqiGodVg', file: 'metadao_conditional_vault.so' },
  { id: 'AMMyu265tkBpRW21iGQxKGLaves3gKm2JcMUqfXNSpqD', file: 'metadao_amm.so' },
]

export const MIN_LIQUIDITY = 1_000_000_000n // 1 KASS (9 decimals) funding floor
const BELOW_FLOOR = 100_000_000n // a partially-funded (Funding) seed
const FEE_BPS = 100 // 1%

/** Write a local ELF at `id` as a BPFLoader2 executable (surfpool JIT-loads it). */
async function deployElf(ctx: SeedCtx, id: string, soPath: string): Promise<void> {
  await ctx.harness.setAccount(id, {
    lamports: 5_000_000_000,
    owner: BPF_LOADER_2,
    executable: true,
    data: readFileSync(soPath).toString('hex'),
  })
}

/**
 * Deploy the market program + the MetaDAO v0.4 fixtures it CPIs, fabricate the
 * upgrade-authority `ProgramData` so `init_config` passes, fund the payer's KASS
 * ATA (the market-creation / fee-destination account), and init the governed
 * `Config` singleton. Returns the payer's KASS ATA (base58). Idempotent enough to
 * run once per surfpool node; shared by {@link seedMarkets} and the active-market
 * seed used by the candle e2e.
 */
export async function deployAndInitMarket(ctx: SeedCtx): Promise<string> {
  const h = ctx.harness
  const payer = ctx.payer.publicKey.toString()
  const kassMint = ctx.kassMint.publicKey.toString()

  // 1) Deploy the market program + the MetaDAO v0.4 fixtures it CPIs.
  await deployElf(ctx, MARKET_PROGRAM_ID.toString(), MARKET_SO)
  for (const { id, file } of METADAO_FIXTURES) {
    await deployElf(ctx, id, join(FIXTURES_DIR, file))
  }

  // 2) Fabricate the program's BPF-Upgradeable-Loader `ProgramData` so `payer` is
  //    its upgrade authority — the precondition `init_config` checks. 45-byte
  //    `UpgradeableLoaderState::ProgramData`: u32 LE variant=3 @0, Some tag=1 @12,
  //    authority @13..45 (mirrors the market harness).
  const programData = (await marketPda.programData()).address.toString()
  const meta = new Uint8Array(45)
  new DataView(meta.buffer).setUint32(0, 3, true)
  meta[12] = 1
  meta.set(ctx.payer.publicKey.toBytes(), 13)
  await h.setAccount(programData, {
    lamports: 2_000_000,
    owner: BPF_UPGRADEABLE_LOADER_ID.toString(),
    executable: false,
    data: toHex(meta),
  })

  // 3) Fund the payer's KASS ATA (creators seed markets from it) + use it as the
  //    protocol fee destination. Same mint as the browser wallet already holds.
  const payerKass = (await associatedTokenAccount(payer, kassMint)).address.toString()
  await h.setAccount(payerKass, {
    lamports: 5_000_000,
    owner: TOKEN_PROGRAM_ID.toString(),
    executable: false,
    data: toHex(tokenAccountBytes(ctx.kassMint.publicKey.toBytes(), ctx.payer.publicKey.toBytes(), 10n ** 15n)),
  })

  // 4) Init the governed Config singleton.
  await sendIx(
    ctx,
    await initConfig({
      payer,
      kassMint,
      authority: payer,
      minLiquidity: MIN_LIQUIDITY,
      feeBps: FEE_BPS,
      feeDestination: payerKass,
    }),
  )
  return payerKass
}

/** Compute-unit limits for the MetaDAO composition / activation / trade CPIs. */
const COMPOSE_CU = 400_000
const ACTIVATE_CU = 1_400_000
const TRADE_CU = 400_000

/** Prepend a compute-unit-limit ix so the compose/activate/trade CPIs fit. */
function withCu(units: number, ...ixs: Parameters<typeof sendIxs>[1]): Parameters<typeof sendIxs>[1] {
  return [ComputeBudgetProgram.setComputeUnitLimit({ units }), ...ixs]
}

/** A stood-up active market: its address, compose refs, and the payer's cYES/cNO ATAs. */
export interface ActiveMarketSeed {
  market: string
  refs: MarketRefs
  /** The payer's cYES / cNO ATAs (a swap inventory when created with `split`). */
  cyesAta: Address
  cnoAta: Address
}

/** Options for {@link createAndActivateMarket}. */
export interface CreateActiveMarketOpts {
  /** The oracle outcome this sub-market binds to (default 0). */
  outcomeIndex?: number
  /** KASS to seed the market with (default {@link MIN_LIQUIDITY} = the floor). */
  seedAmount?: bigint
  /**
   * When true, fabricate the payer's cYES/cNO ATAs and split 5 KASS of each leg to
   * them — a trading inventory for the candle e2e's swaps. `make dev` doesn't need
   * it (users split/swap through the UI against the seeded pool).
   */
  split?: boolean
}

/**
 * Create a market on `oracle` funded to `seedAmount`, compose its MetaDAO
 * question/vault/AMM, and activate it — leaving it **Active** (its cYES/cNO pool is
 * live, so the app shows the trade panel). Assumes the market program + MetaDAO
 * fixtures are deployed and `Config` is initialized (see {@link deployAndInitMarket}),
 * and that `oracle` is non-terminal (activation requires a live oracle). Returns the
 * {@link ActiveMarketSeed}; throws if activation didn't take.
 */
export async function createAndActivateMarket(
  ctx: SeedCtx,
  oracle: string,
  payerKass: string,
  opts: CreateActiveMarketOpts = {},
): Promise<ActiveMarketSeed> {
  const outcomeIndex = opts.outcomeIndex ?? 0
  const seedAmount = opts.seedAmount ?? MIN_LIQUIDITY
  const h = ctx.harness
  const payer = ctx.payer.publicKey.toString()
  const kassMint = ctx.kassMint.publicKey.toString()

  // createMarket with seed == floor funds the market fully in one shot (Funding →
  // activatable). Outcome i = YES pays if the oracle resolves to option i.
  const market = (await marketPda.market(oracle, outcomeIndex)).address.toString()
  await sendIx(
    ctx,
    await createMarket({ creator: payer, oracle, kassMint, creatorKassAta: payerKass, seedAmount, outcomeIndex }),
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

  const cyesAta = (await associatedTokenAccount(payer, refs.yesMint.toString())).address
  const cnoAta = (await associatedTokenAccount(payer, refs.noMint.toString())).address
  if (opts.split) {
    // Fabricate the payer's cYES / cNO ATAs (empty), then split KASS into a cYES+cNO
    // inventory so a swap can push the price either way.
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
  }

  // Verify Active: the `Market.status` byte sits at offset 154 (account_type[1] +
  // _pad_hdr[7] + 4×Pubkey[128] + min_liquidity[8] + total_contributed[8] +
  // open_contributions[2]). MarketStatus.Active == 1.
  const info = await h.connection.getAccountInfo(new Address(market))
  if (!info || info.data[154] !== MarketStatus.Active) {
    throw new Error(`market ${market} did not reach Active after activate (status=${info?.data[154]})`)
  }
  return { market, refs, cyesAta, cnoAta }
}

/**
 * Deploy + init the market and pre-create demo markets on the seeded oracles.
 * `oracles` is the map the oracle seeder built (phase → { address, ... }). Returns
 * the created market addresses for the wallet fixture. Best-effort: throws on a
 * hard failure so `make dev` surfaces it (the caller tears down).
 */
export async function seedMarkets(
  ctx: SeedCtx,
  oracles: Record<string, Record<string, string>>,
): Promise<{ seeded: Record<string, unknown>; active: ActiveMarketSeed | null }> {
  const payer = ctx.payer.publicKey.toString()
  const kassMint = ctx.kassMint.publicKey.toString()

  // 1-4) Deploy the program + fixtures, fabricate ProgramData, fund the payer KASS
  //       ATA, and init the Config singleton.
  const payerKass = await deployAndInitMarket(ctx)

  // 5) Pre-create demo markets on the already-seeded oracles.
  const createOne = async (oracle: string, outcomeIndex: number, seedAmount: bigint) => {
    await sendIx(
      ctx,
      await createMarket({ creator: payer, oracle, kassMint, creatorKassAta: payerKass, seedAmount, outcomeIndex }),
    )
    return (await marketPda.market(oracle, outcomeIndex)).address.toString()
  }

  const seeded: Record<string, unknown> = { kassMint, config: (await marketPda.config()).address.toString() }

  // The 3-option "proposal" oracle → a categorical spread of 3 Funding sub-markets.
  if (oracles.proposal?.address) {
    const categorical: string[] = []
    for (let i = 0; i < 3; i++) categorical.push(await createOne(oracles.proposal.address, i, BELOW_FLOOR))
    seeded.categoricalOracle = oracles.proposal.address
    seeded.categoricalMarkets = categorical
  }
  // A 2-option oracle → a spread across the market lifecycle:
  //   • outcome 0: funded to the floor, then composed + ACTIVATED → a live cYES/cNO
  //     pool. `make dev` must always surface at least one tradable + funded market
  //     (the app renders the trade panel for an Active market). Created WITH a split
  //     inventory so the caller can drive swaps that populate the price chart.
  //   • outcome 1: funded to the floor but left in Funding — the "ready to activate"
  //     state, so the Activate flow is still demoable.
  // Activation is best-effort: if the MetaDAO compose/activate CPI fails, fall back
  // to leaving outcome 0 funded-but-inactive so the rest of the seed still stands.
  let active: ActiveMarketSeed | null = null
  if (oracles.factProposal?.address) {
    try {
      active = await createAndActivateMarket(ctx, oracles.factProposal.address, payerKass, {
        outcomeIndex: 0,
        split: true,
      })
      seeded.activeMarket = active.market
    } catch (e) {
      // Loud, not silent: a missing active market is exactly the "no tradable market"
      // symptom, so surface why it fell back instead of quietly degrading.
      // eslint-disable-next-line no-console
      console.warn(`[dev] ⚠ market activation failed, seeding a funded market instead: ${(e as Error).message}`)
      seeded.fundedMarket = await createOne(oracles.factProposal.address, 0, MIN_LIQUIDITY)
    }
    seeded.activatableMarket = await createOne(oracles.factProposal.address, 1, MIN_LIQUIDITY)
  }

  return { seeded, active }
}
