/**
 * D3b — instruction builders for the challenge round: `open_challenge` and
 * `settle_challenge`.
 *
 * These two bind to EXTERNALLY-COMPOSED MetaDAO accounts. As in the test
 * harness, the SDK does NOT create the MetaDAO market (question / conditional
 * vaults / AMMs / conditional mints / event authority) — the caller composes
 * those in their own transactions and passes their pubkeys here. The SDK
 * derives only the Kassandra-owned PDAs:
 *   - oracle        `[b"oracle", nonce_le]`
 *   - ai_claim      `[b"claim", oracle, proposer]`   (open_challenge only)
 *   - market        `[b"market", ai_claim]`
 *   - stake_vault   `[b"vault", oracle]`
 *   - escrow_vault  `[b"challenge_usdc", market]`
 *   - protocol      `[b"protocol"]`                  (open_challenge only)
 * and pins the fixed program ids (conditional-vault, token, system).
 *
 * Account orders mirror the harness `open_challenge_ix` / `settle_ix`
 * (`programs/kassandra/tests/challenge_e2e.rs`) slot-by-slot. Payload for both
 * is `oracle_nonce: u64 LE`.
 */
import { Address, TransactionInstruction } from "@solana/web3.js";

import {
  EXTERNAL_PROGRAM_IDS,
  Ix,
  KASSANDRA_PROGRAM_ID,
  SYSTEM_PROGRAM_ID,
  TOKEN_PROGRAM_ID,
} from "../constants.js";
import * as pda from "../pda.js";
import type { AddressInput } from "../pda.js";
import { addr, ro, u64LE, w, withDisc } from "./payload.js";

const CONDITIONAL_VAULT_ID = EXTERNAL_PROGRAM_IDS.conditionalVault;

// ---------------------------------------------------------------------------
// OpenChallenge (Ix=4) — processor/open_challenge.rs
// 25 accounts (see slot map below). Payload: oracle_nonce u64.
//
// SDK-DERIVED (from nonce/proposer): 0 oracle, 1 ai_claim, 3 market,
//   10 stake_vault, 20 protocol, 24 escrow_vault. Fixed program ids: 16 cv
//   program, 17 token program, 18 system program.
// CALLER-SUPPLIED (MetaDAO + actor): 2 proposer, 4 challenger(signer),
//   5 question, 6 kass_vault, 7 usdc_vault, 8 pass_amm, 9 fail_amm,
//   11 kass_vault_underlying, 12 pass_kass_mint, 13 fail_kass_mint,
//   14 oracle_pass_kass, 15 oracle_fail_kass, 19 cv_event_authority,
//   21 kass_dao, 22 usdc_mint, 23 challenger_usdc_src.
// ---------------------------------------------------------------------------
export interface OpenChallengeArgs {
  /** Oracle nonce — payload + derives the oracle/stake_vault PDAs. */
  nonce: bigint | number;
  /** The challenged claim's Proposer PDA (derives ai_claim/market). */
  proposer: AddressInput;
  /** Challenger (signer): pays the Market + escrow rent, funds the USDC escrow. */
  challenger: AddressInput;
  // --- MetaDAO market accounts (caller-composed) ---
  /** Binary MetaDAO `Question` (resolver == oracle PDA). Read-only. */
  question: AddressInput;
  /** KASS conditional vault (underlying == oracle.kass_mint). Writable. */
  kassVault: AddressInput;
  /** USDC conditional vault (underlying == oracle.usdc_mint). Read-only. */
  usdcVault: AddressInput;
  /** Pass-side AMM (owned by the AMM program). Read-only. */
  passAmm: AddressInput;
  /** Fail-side AMM. Read-only. */
  failAmm: AddressInput;
  /** `kass_vault.underlying_token_account`. Writable. */
  kassVaultUnderlying: AddressInput;
  /** Conditional-KASS mint idx 0 of kass_vault (pass). Writable. */
  passKassMint: AddressInput;
  /** Conditional-KASS mint idx 1 of kass_vault (fail). Writable. */
  failKassMint: AddressInput;
  /** Oracle-PDA-owned pass-KASS holder token account. Writable. */
  oraclePassKass: AddressInput;
  /** Oracle-PDA-owned fail-KASS holder token account. Writable. */
  oracleFailKass: AddressInput;
  /** Conditional-vault `#[event_cpi]` event authority PDA. Read-only. */
  cvEventAuthority: AddressInput;
  /** The futarchy `Dao` (`== protocol.kass_dao`), kass_price source. Read-only. */
  kassDao: AddressInput;
  /** Canonical USDC mint (`== oracle.usdc_mint`). Read-only. */
  usdcMint: AddressInput;
  /** Challenger's USDC source token account. Writable. */
  challengerUsdcSrc: AddressInput;
  programId?: Address;
}

export async function openChallenge(args: OpenChallengeArgs): Promise<TransactionInstruction> {
  const programId = args.programId ?? KASSANDRA_PROGRAM_ID;
  const oracle = await pda.oracle(BigInt(args.nonce), programId);
  const proposer = addr(args.proposer);
  const aiClaim = await pda.aiClaim(oracle.address, proposer, programId);
  const market = await pda.market(aiClaim.address, programId);
  const stakeVault = await pda.stakeVault(oracle.address, programId);
  const protocol = await pda.protocol(programId);
  const escrowVault = await pda.challengeUsdcVault(market.address, programId);

  return new TransactionInstruction({
    programId,
    keys: [
      w(oracle.address), // 0
      w(aiClaim.address), // 1
      w(proposer), // 2
      w(market.address), // 3
      w(addr(args.challenger), true), // 4
      ro(addr(args.question)), // 5
      w(addr(args.kassVault)), // 6
      ro(addr(args.usdcVault)), // 7
      ro(addr(args.passAmm)), // 8
      ro(addr(args.failAmm)), // 9
      w(stakeVault.address), // 10
      w(addr(args.kassVaultUnderlying)), // 11
      w(addr(args.passKassMint)), // 12
      w(addr(args.failKassMint)), // 13
      w(addr(args.oraclePassKass)), // 14
      w(addr(args.oracleFailKass)), // 15
      ro(CONDITIONAL_VAULT_ID), // 16
      ro(TOKEN_PROGRAM_ID), // 17
      ro(SYSTEM_PROGRAM_ID), // 18
      ro(addr(args.cvEventAuthority)), // 19
      ro(protocol.address), // 20
      ro(addr(args.kassDao)), // 21
      ro(addr(args.usdcMint)), // 22
      w(addr(args.challengerUsdcSrc)), // 23
      w(escrowVault.address), // 24
    ],
    data: withDisc(Ix.OpenChallenge, u64LE(args.nonce)),
  });
}

// ---------------------------------------------------------------------------
// SettleChallenge (Ix=5) — processor/settle_challenge.rs
// 21 accounts (see slot map below). Payload: oracle_nonce u64.
//
// SDK-DERIVED: 0 oracle (from nonce), 1 market (from ai_claim),
//   10 stake_vault (from oracle), 17 escrow_vault (from market). Fixed ids:
//   7 cv program, 9 token program.
// CALLER-SUPPLIED: 2 ai_claim, 3 proposer, 4 question, 5 pass_amm, 6 fail_amm,
//   8 cv_event_authority, 11 kass_vault, 12 kass_vault_underlying,
//   13 pass_kass_mint, 14 fail_kass_mint, 15 oracle_pass_kass,
//   16 oracle_fail_kass, 18 proposer_usdc, 19 challenger_usdc_dest,
//   20 challenger_kass.
// ---------------------------------------------------------------------------
export interface SettleChallengeArgs {
  /** Oracle nonce — payload + derives the oracle/stake_vault PDAs. */
  nonce: bigint | number;
  /** The challenged claim's AiClaim (`== market.ai_claim`); derives market. */
  aiClaim: AddressInput;
  /** The claim's Proposer PDA (`== market.proposer`). Writable. */
  proposer: AddressInput;
  // --- MetaDAO market accounts (caller-composed) ---
  /** The MetaDAO `Question` (`== market.question`); resolved here. Writable. */
  question: AddressInput;
  /** Pass-side AMM (`== market.pass_amm`). Read-only. */
  passAmm: AddressInput;
  /** Fail-side AMM (`== market.fail_amm`). Read-only. */
  failAmm: AddressInput;
  /** Conditional-vault `#[event_cpi]` event authority PDA. Read-only. */
  cvEventAuthority: AddressInput;
  /** KASS conditional vault (`== market.kass_vault`). Writable. */
  kassVault: AddressInput;
  /** `kass_vault.underlying_token_account`. Writable. */
  kassVaultUnderlying: AddressInput;
  /** Conditional-KASS mint idx 0 of kass_vault (pass). Writable. */
  passKassMint: AddressInput;
  /** Conditional-KASS mint idx 1 of kass_vault (fail). Writable. */
  failKassMint: AddressInput;
  /** Oracle-PDA-owned pass-KASS holder (`== market.oracle_pass_kass`). Writable. */
  oraclePassKass: AddressInput;
  /** Oracle-PDA-owned fail-KASS holder (`== market.oracle_fail_kass`). Writable. */
  oracleFailKass: AddressInput;
  /** Proposer's USDC payout account (mint==usdc, owner==proposer.authority). Writable. */
  proposerUsdc: AddressInput;
  /** Challenger's USDC payout account (mint==usdc, owner==market.challenger). Writable. */
  challengerUsdcDest: AddressInput;
  /** Challenger's KASS payout account (mint==kass, owner==market.challenger). Writable. */
  challengerKass: AddressInput;
  programId?: Address;
}

export async function settleChallenge(args: SettleChallengeArgs): Promise<TransactionInstruction> {
  const programId = args.programId ?? KASSANDRA_PROGRAM_ID;
  const oracle = await pda.oracle(BigInt(args.nonce), programId);
  const aiClaim = addr(args.aiClaim);
  const market = await pda.market(aiClaim, programId);
  const stakeVault = await pda.stakeVault(oracle.address, programId);
  const escrowVault = await pda.challengeUsdcVault(market.address, programId);

  return new TransactionInstruction({
    programId,
    keys: [
      w(oracle.address), // 0
      w(market.address), // 1
      ro(aiClaim), // 2
      w(addr(args.proposer)), // 3
      w(addr(args.question)), // 4
      ro(addr(args.passAmm)), // 5
      ro(addr(args.failAmm)), // 6
      ro(CONDITIONAL_VAULT_ID), // 7
      ro(addr(args.cvEventAuthority)), // 8
      ro(TOKEN_PROGRAM_ID), // 9
      w(stakeVault.address), // 10
      w(addr(args.kassVault)), // 11
      w(addr(args.kassVaultUnderlying)), // 12
      w(addr(args.passKassMint)), // 13
      w(addr(args.failKassMint)), // 14
      w(addr(args.oraclePassKass)), // 15
      w(addr(args.oracleFailKass)), // 16
      w(escrowVault.address), // 17
      w(addr(args.proposerUsdc)), // 18
      w(addr(args.challengerUsdcDest)), // 19
      w(addr(args.challengerKass)), // 20
    ],
    data: withDisc(Ix.SettleChallenge, u64LE(args.nonce)),
  });
}
