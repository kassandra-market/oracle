/**
 * SD1 — DERIVE-FROM-MARKET settle (pure ix-builder, NO React).
 *
 * `settle_challenge` binds 15 caller-supplied accounts (plus the SDK-derived
 * oracle / market / stake_vault / escrow_vault). RF4's {@link buildSettleChallengeIxs}
 * only THREADS those 15 pubkeys into the SDK builder — the runner used to emit
 * them as pasted JSON. SD1 removes the paste: given the DECODED on-chain
 * {@link Market} + {@link Oracle} (the same objects the challenge detail already
 * fetches), it DERIVES every one of the 15 client-side, so settle becomes a
 * one-click wallet action:
 *
 *   - **Directly on the Market** (fields decoded from the on-chain account):
 *     `aiClaim`, `proposer`, `question`, `passAmm`, `failAmm`, `kassVault`,
 *     `oraclePassKass`, `oracleFailKass`.
 *   - **Derived (CU2/CU3 derivations, reused import-only):**
 *     - `passKassMint` / `failKassMint` = `conditionalTokenMint(market.kassVault, 0/1)`;
 *     - `kassVaultUnderlying` = `associatedTokenAccount(market.kassVault, oracle.kassMint)`
 *       — the KASS conditional vault's underlying ATA (the same one CU3 composes +
 *       the settle handler binds against `kass_vault.underlying_token_account`);
 *     - `cvEventAuthority` = `futarchy.pda.vaultEventAuthority()` (fixed PDA);
 *     - `proposerUsdc` = `ATA(proposerAuthority, oracle.usdcMint)` — NOTE the
 *       owner is the proposer's WALLET (`Proposer.authority`), NOT the Proposer
 *       PDA (`market.proposer`): the settle handler binds it with
 *       `assert_token_account(.., oracle.usdc_mint, proposer.authority)`. The
 *       decoded `Market` records only the Proposer PDA, so the caller supplies
 *       `proposerAuthority` (read off the decoded `Proposer` whose pubkey ==
 *       `market.proposer`, already fetched by `fetchOracleDetail`);
 *     - `challengerUsdcDest` = `ATA(market.challenger, oracle.usdcMint)`;
 *     - `challengerKass` = `ATA(market.challenger, oracle.kassMint)`.
 *
 * --- challengerUsdcDest vs the escrow (settle account 19 vs 17) ---
 * The settle handler (`processor/settle_challenge.rs`) reads TWO challenger-USDC
 * accounts. Account 17 (`escrow_vault` / `market.challenger_usdc_vault`) is the
 * Market-owned USDC escrow — SDK-derived from the market PDA, NOT supplied here.
 * Account 19 (`challenger_usdc_dest`) is the challenger's OWN USDC payout account:
 * the handler binds it with `assert_token_account(.., oracle.usdc_mint,
 * market.challenger)` — an SPL token account, mint == usdc_mint, owner ==
 * market.challenger — i.e. `ATA(market.challenger, oracle.usdcMint)`. So the DEST
 * is the ATA, not the escrow. (Same shape for proposerUsdc / challengerKass.)
 *
 * --- do the payout ATAs need creating? ---
 * settle ASSERTS the three payout destinations (proposerUsdc / challengerUsdcDest
 * / challengerKass) as existing SPL token accounts — it does NOT create them (it
 * only `assert_token_account`s owner+mint, then transfers into them). On a real
 * cluster a payout ATA could be absent (e.g. the challenger never held KASS), and
 * settle would then fail the assert. So — when a `connection` is supplied — this
 * prepends an idempotent `createAssociatedTokenAccountIdempotent` for each of the
 * three payout ATAs (the settle payer covers the tiny rent; already-present ATAs
 * are a no-op). Without a `connection` it emits just the settle ix (the offline
 * byte-match path).
 *
 * NO core / SDK change: every derivation reuses the SDK
 * (`associatedTokenAccount` / `futarchy.pda.vaultEventAuthority`) + CU2's
 * `conditionalTokenMint`; the settle ix itself is RF4's
 * {@link buildSettleChallengeIxs} (import-only).
 */
import { Address, TransactionInstruction, type Connection } from "@solana/web3.js";
import {
  ATA_PROGRAM_ID,
  SYSTEM_PROGRAM_ID,
  TOKEN_PROGRAM_ID,
  associatedTokenAccount,
  futarchy,
  type Market,
  type Oracle,
} from "@kassandra-market/oracles";

import { ValidationError } from "../actions";
import { conditionalTokenMint } from "./challengeTrade";
import { buildSettleChallengeIxs } from "./challenge";

export interface BuildSettleFromMarketArgs {
  /**
   * RPC connection — when supplied, an idempotent create for each of the three
   * payout ATAs (proposerUsdc / challengerUsdcDest / challengerKass) is prepended
   * so settle never fails on an absent destination. Omit for the offline
   * byte-match (settle ix only).
   */
  connection?: Connection;
  /** Oracle nonce (payload + re-derives the oracle/stake-vault signer PDAs). */
  oracleNonce: bigint | number;
  /** The DECODED on-chain challenge {@link Market} (source of 8 of the 15 accounts). */
  market: Market;
  /** The DECODED {@link Oracle} (its kass/usdc mints derive the payout ATAs). */
  oracle: Oracle;
  /**
   * The challenged proposer's WALLET authority (`Proposer.authority`) — the OWNER
   * of `proposerUsdc` the settle handler asserts. Read off the decoded `Proposer`
   * whose pubkey == `market.proposer` (`fetchOracleDetail` already fetches it).
   * Required: the Market records only the Proposer PDA, not its authority.
   */
  proposerAuthority: Address | string;
  /**
   * The settle payer (fee-payer + signer). Only used to pay the rent of the
   * idempotent payout-ATA creates when a `connection` is supplied.
   */
  payer?: Address | string;
  programId?: Address;
}

/**
 * The idempotent `createAssociatedTokenAccountIdempotent` ix (ATA program
 * discriminant `1`), same hand-built layout CU3 uses (no `@solana/spl-token`
 * dep). Accounts: payer(w,signer), ata(w), owner(ro), mint(ro), system(ro),
 * token(ro).
 */
function createAtaIdempotentIx(
  payer: Address,
  ataAddr: Address,
  owner: Address,
  mint: Address,
): TransactionInstruction {
  return new TransactionInstruction({
    programId: ATA_PROGRAM_ID,
    keys: [
      { pubkey: payer, isSigner: true, isWritable: true },
      { pubkey: ataAddr, isSigner: false, isWritable: true },
      { pubkey: owner, isSigner: false, isWritable: false },
      { pubkey: mint, isSigner: false, isWritable: false },
      { pubkey: SYSTEM_PROGRAM_ID, isSigner: false, isWritable: false },
      { pubkey: TOKEN_PROGRAM_ID, isSigner: false, isWritable: false },
    ],
    data: Uint8Array.of(1),
  });
}

/**
 * Derive the FULL `settle_challenge` account set from the decoded Market +
 * Oracle and build the settle instruction(s). Returns the settle ix (RF4's
 * {@link buildSettleChallengeIxs}), optionally preceded by idempotent creates of
 * the three payout ATAs when a `connection` (and `payer`) is supplied.
 *
 * Validates the Market is present and NOT already settled (settle would revert
 * with `AlreadySettled`).
 */
export async function buildSettleFromMarketIxs(
  args: BuildSettleFromMarketArgs,
): Promise<TransactionInstruction[]> {
  const { market, oracle } = args;
  if (!market || market.accountType === undefined) {
    throw new ValidationError("market", "The challenge Market is required to settle.");
  }
  if (market.settled) {
    throw new ValidationError("market", "This challenge market is already settled.");
  }
  if (!oracle || oracle.accountType === undefined) {
    throw new ValidationError("oracle", "The decoded Oracle is required to derive the payout accounts.");
  }
  if (!args.proposerAuthority) {
    throw new ValidationError(
      "proposerAuthority",
      "The challenged proposer's authority is required (owner of the proposer USDC payout).",
    );
  }
  const proposerAuthority =
    args.proposerAuthority instanceof Address
      ? args.proposerAuthority
      : new Address(args.proposerAuthority);

  // --- derived: conditional-KASS mints (pass=0/fail=1) + the vault underlying ATA
  const [passKassMint, failKassMint, kassVaultUnderlying, cvEventAuthority] = await Promise.all([
    conditionalTokenMint(market.kassVault, 0),
    conditionalTokenMint(market.kassVault, 1),
    associatedTokenAccount(market.kassVault, oracle.kassMint).then((p) => p.address),
    futarchy.pda.vaultEventAuthority().then((p) => p.address),
  ]);

  // --- derived: the three payout ATAs (settle account 18 / 19 / 20).
  const [proposerUsdc, challengerUsdcDest, challengerKass] = await Promise.all([
    associatedTokenAccount(proposerAuthority, oracle.usdcMint).then((p) => p.address),
    associatedTokenAccount(market.challenger, oracle.usdcMint).then((p) => p.address),
    associatedTokenAccount(market.challenger, oracle.kassMint).then((p) => p.address),
  ]);

  const settleIxs = await buildSettleChallengeIxs({
    oracleNonce: args.oracleNonce,
    // --- directly on the Market ---
    aiClaim: market.aiClaim,
    proposer: market.proposer,
    question: market.question,
    passAmm: market.passAmm,
    failAmm: market.failAmm,
    kassVault: market.kassVault,
    oraclePassKass: market.oraclePassKass,
    oracleFailKass: market.oracleFailKass,
    // --- derived ---
    passKassMint,
    failKassMint,
    kassVaultUnderlying,
    cvEventAuthority,
    proposerUsdc,
    challengerUsdcDest,
    challengerKass,
    programId: args.programId,
  });

  // Prepend idempotent payout-ATA creates only when we can pay for them. settle
  // assumes the three destinations already exist (it asserts, never creates); an
  // absent one on a real cluster would fail the assert, so create them first.
  if (args.connection && args.payer) {
    const payer = args.payer instanceof Address ? args.payer : new Address(args.payer);
    const creates = [
      createAtaIdempotentIx(payer, proposerUsdc, proposerAuthority, oracle.usdcMint),
      createAtaIdempotentIx(payer, challengerUsdcDest, market.challenger, oracle.usdcMint),
      createAtaIdempotentIx(payer, challengerKass, market.challenger, oracle.kassMint),
    ];
    return [...creates, ...settleIxs];
  }

  return settleIxs;
}
