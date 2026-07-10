/**
 * Playwright globalSetup for the FORKED challenge-market project.
 *
 * Boots surfpool FORKING MAINNET (so MetaDAO's deployed conditional_vault / amm /
 * futarchy programs are executable) in `clock` block-production mode with a fast
 * slot-time (the v0.4 AMM TWAP crank is SLOT-based), then drives one oracle to the
 * Challenge phase with a surviving proposer's AI claim finalized — the exact state
 * the browser's client-side compose→open→swap→crank→settle→close flow acts on.
 *
 * The browser wallet is the CHALLENGER (funded with SOL + KASS + USDC at its
 * canonical ATAs); the challenged proposer is a separate seeded keypair. Writes
 * the funded keypair + the seeded market inputs to `e2e/fork/.wallet.json`.
 */
import { writeFileSync } from 'node:fs'
import { buildDaoBlob } from '../../../sdks/oracles/ts/test/surfpool/futarchy-dao.ts'
import { join } from 'node:path'

import { Keypair } from '@solana/web3.js'
import {
  EXTERNAL_PROGRAM_IDS,
  TOKEN_PROGRAM_ID,
  associatedTokenAccount,
  futarchy,
  setGovernance,
} from '@kassandra-market/oracles'

import { toHex, tokenAccountBytes } from '../../../sdks/oracles/ts/test/surfpool/harness.ts'
import { bootAndInit, createOracleReal, driveToChallengeSurviving, sendIx } from '../seed.ts'

const PORT = 8940
const WALLET_FILE = join(process.cwd(), 'e2e', 'fork', '.wallet.json')

// KASS/USDC spot TWAP (raw USDC per raw KASS × 1e12). open_challenge sizes the
// escrow as `required_usdc = proposer.bond × twap / 1e12`; the seed's proposers
// bond only 1_000 raw KASS, so the TWAP is set high enough that the escrow is a
// non-zero 500_000 raw USDC (else the tiny bond rounds it to 0 → ZeroStake).
const KASS_PRICE_TWAP = 500_000_000_000_000n
async function globalSetup(): Promise<() => Promise<void>> {
  const ctx = await bootAndInit(PORT, {
    fork: 'mainnet',
    blockProductionMode: 'clock',
    slotTimeMs: 10,
    readyTimeoutMs: 60_000,
  })
  const rpcUrl = `http://127.0.0.1:${PORT}`

  // ── Governance: a futarchy-owned kass_dao + the REAL one-shot set_governance
  //    handoff (validates the Squads-vault linkage) — open_challenge's USDC escrow
  //    sizing reads kass_price(kass_dao).
  const kassDao = await Keypair.generate()
  await ctx.harness.setAccount(kassDao.publicKey.toString(), {
    lamports: 5_000_000,
    owner: EXTERNAL_PROGRAM_IDS.futarchyV06.toString(),
    executable: false,
    data: toHex(buildDaoBlob(KASS_PRICE_TWAP * 1_000_000n, 1_000_000n, 0n)),
  })
  const multisig = (await futarchy.pda.squadsMultisig(kassDao.publicKey.toString())).address
  const daoAuthority = (await futarchy.pda.squadsVault(multisig.toString(), 0)).address
  await sendIx(
    ctx,
    await setGovernance({
      authority: ctx.payer.publicKey.toString(),
      daoAuthority: daoAuthority.toString(),
      kassDao: kassDao.publicKey.toString(),
    }),
  )

  // ── Drive one oracle to Challenge with a surviving proposer's AI claim. The
  //    proposer authority is a dedicated keypair (NOT the browser challenger).
  const nonce = 200n
  const proposerAuthority = await Keypair.generate()
  const oracle = await createOracleReal(ctx, nonce, 2, 'E2E forked challenge market')
  const { proposers } = await driveToChallengeSurviving(ctx, oracle, nonce, proposerAuthority)
  const challengedProposer = proposers[0].toString()

  // ── The funded browser wallet = the CHALLENGER. It composes + funds the whole
  //    market, so it needs SOL + KASS (split base) + USDC (split quote + escrow).
  const wallet = await Keypair.generate()
  await ctx.harness.airdrop(wallet.publicKey.toString(), 50_000_000_000)
  const walletKass = (
    await associatedTokenAccount(wallet.publicKey.toString(), ctx.kassMint.publicKey.toString())
  ).address
  const walletUsdc = (
    await associatedTokenAccount(wallet.publicKey.toString(), ctx.usdcMint.publicKey.toString())
  ).address
  await ctx.harness.setAccount(walletKass.toString(), {
    lamports: 5_000_000,
    owner: TOKEN_PROGRAM_ID.toString(),
    executable: false,
    data: toHex(
      tokenAccountBytes(ctx.kassMint.publicKey.toBytes(), wallet.publicKey.toBytes(), 10n ** 15n),
    ),
  })
  await ctx.harness.setAccount(walletUsdc.toString(), {
    lamports: 5_000_000,
    owner: TOKEN_PROGRAM_ID.toString(),
    executable: false,
    data: toHex(
      tokenAccountBytes(ctx.usdcMint.publicKey.toBytes(), wallet.publicKey.toBytes(), 10n ** 12n),
    ),
  })

  writeFileSync(
    WALLET_FILE,
    JSON.stringify(
      {
        secretKey: Array.from(wallet.secretKey as Uint8Array),
        publicKey: wallet.publicKey.toString(),
        rpcUrl,
        kassMint: ctx.kassMint.publicKey.toString(),
        usdcMint: ctx.usdcMint.publicKey.toString(),
        kassDao: kassDao.publicKey.toString(),
        oracle: oracle.toString(),
        nonce: nonce.toString(),
        proposer: challengedProposer,
      },
      null,
      2,
    ),
  )

  // eslint-disable-next-line no-console
  console.log(
    `[e2e:fork] surfpool ${rpcUrl}; challenger ${wallet.publicKey.toString()}; oracle ${oracle.toString()} @ Challenge; proposer ${challengedProposer}`,
  )

  return async () => {
    await ctx.harness.teardown()
  }
}

export default globalSetup
