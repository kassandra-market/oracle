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

import { TOKEN_PROGRAM_ID, associatedTokenAccount } from '@kassandra/sdk'
import {
  BPF_UPGRADEABLE_LOADER_ID,
  MARKET_PROGRAM_ID,
  createMarket,
  initConfig,
  pda as marketPda,
} from '@kassandra-market/sdk'

import { toHex, tokenAccountBytes } from '../../sdk/test/surfpool/harness.ts'
import { sendIx, type SeedCtx } from './seed.ts'

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

const MIN_LIQUIDITY = 1_000_000_000n // 1 KASS (9 decimals) funding floor
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
 * Deploy + init the market and pre-create demo markets on the seeded oracles.
 * `oracles` is the map the oracle seeder built (phase → { address, ... }). Returns
 * the created market addresses for the wallet fixture. Best-effort: throws on a
 * hard failure so `make dev` surfaces it (the caller tears down).
 */
export async function seedMarkets(
  ctx: SeedCtx,
  oracles: Record<string, Record<string, string>>,
): Promise<Record<string, unknown>> {
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
  // A 2-option oracle → one market seeded AT the floor (funded / activatable).
  if (oracles.factProposal?.address) {
    seeded.fundedMarket = await createOne(oracles.factProposal.address, 0, MIN_LIQUIDITY)
  }

  return seeded
}
