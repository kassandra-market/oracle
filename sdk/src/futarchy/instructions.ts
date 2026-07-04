/**
 * Instruction builders for futarchy v0.6 + Squads v4 + conditional_vault.
 *
 * Each builder returns a web3.js (classic) `TransactionInstruction` whose
 * `data == [disc, ...borsh_args]` and whose `keys` are the EXACT account-meta
 * order documented in `./NOTES.md` (sourced from the binary-validated Rust CPI
 * modules + the `metaDAOproject/futarchy@v0.6.0` / `Squads-Protocol/v4` source).
 *
 * All futarchy + conditional_vault instructions are `#[event_cpi]`: the two
 * trailing accounts (event_authority PDA, program id) are appended by the
 * builders. The Meteora DAMM v2 builders live in the sibling `../meteora` module.
 */
import { Address, TransactionInstruction } from "@solana/web3.js";
import type { AccountMeta } from "@solana/web3.js";

import { SYSTEM_PROGRAM_ID, TOKEN_PROGRAM_ID } from "../constants.js";
import * as mpda from "../meteora/pda.js";
import type { AddressInput } from "../pda.js";
import {
  CONDITIONAL_VAULT_ID,
  DAMM_V2_POOL_AUTHORITY,
  DISC,
  FUTARCHY_ID,
  Market,
  METADAO_ADMIN,
  METADAO_MULTISIG_VAULT,
  METEORA_DAMM_V2_ID,
  SQUADS_PERMISSIONLESS_MEMBER,
  SQUADS_V4_ID,
  SwapType,
} from "./constants.js";
import * as fpda from "./pda.js";

/** The SPL Associated Token Account program. */
export const ATA_PROGRAM_ID = new Address("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");

// ── meta + borsh helpers ─────────────────────────────────────────────────────

function addr(a: AddressInput): Address {
  return a instanceof Address ? a : new Address(a);
}
function w(pubkey: AddressInput, isSigner = false): AccountMeta {
  return { pubkey: addr(pubkey), isSigner, isWritable: true };
}
function ro(pubkey: AddressInput, isSigner = false): AccountMeta {
  return { pubkey: addr(pubkey), isSigner, isWritable: false };
}

function u8b(v: number): Uint8Array {
  return Uint8Array.from([v & 0xff]);
}
function u16le(v: number): Uint8Array {
  const o = new Uint8Array(2);
  new DataView(o.buffer).setUint16(0, v, true);
  return o;
}
function i16le(v: number): Uint8Array {
  const o = new Uint8Array(2);
  new DataView(o.buffer).setInt16(0, v, true);
  return o;
}
function u32le(v: number): Uint8Array {
  const o = new Uint8Array(4);
  new DataView(o.buffer).setUint32(0, v, true);
  return o;
}
function u64le(v: bigint | number): Uint8Array {
  const o = new Uint8Array(8);
  new DataView(o.buffer).setBigUint64(0, BigInt(v), true);
  return o;
}
function u128le(v: bigint | number): Uint8Array {
  const o = new Uint8Array(16);
  const dv = new DataView(o.buffer);
  const x = BigInt(v);
  dv.setBigUint64(0, x & 0xffffffffffffffffn, true);
  dv.setBigUint64(8, x >> 64n, true);
  return o;
}
function boolb(v: boolean): Uint8Array {
  return Uint8Array.from([v ? 1 : 0]);
}
/** Borsh `Vec<u8>` — u32 LE length prefix then the bytes. */
function vecU8(bytes: Uint8Array): Uint8Array {
  return concat([u32le(bytes.length), bytes]);
}
/** Borsh `Option<String>` (None or UTF-8 with a u32 length prefix). */
function optString(s: string | null | undefined): Uint8Array {
  if (s === null || s === undefined) return Uint8Array.from([0]);
  const b = new TextEncoder().encode(s);
  return concat([Uint8Array.from([1]), u32le(b.length), b]);
}
function concat(parts: Array<Uint8Array>): Uint8Array {
  const total = parts.reduce((n, p) => n + p.length, 0);
  const out = new Uint8Array(total);
  let off = 0;
  for (const p of parts) {
    out.set(p, off);
    off += p.length;
  }
  return out;
}

/** Associated token account `[owner, TOKEN_PROGRAM, mint]` under the ATA program. */
export async function ata(owner: AddressInput, mint: AddressInput): Promise<Address> {
  const [a] = await Address.findProgramAddress(
    [addr(owner).toBytes(), TOKEN_PROGRAM_ID.toBytes(), addr(mint).toBytes()],
    ATA_PROGRAM_ID,
  );
  return a;
}

// ════════════════════════════════════════════════════════════════════════════
// conditional_vault
// ════════════════════════════════════════════════════════════════════════════

export interface InitializeQuestionArgs {
  /** 32-byte question id. */
  questionId: Uint8Array;
  /** Oracle/resolver (for a futarchy proposal this is the Proposal PDA). */
  oracle: AddressInput;
  /** Outcome count (binary futarchy uses 2). */
  numOutcomes: number;
  /** Rent payer + signer. */
  payer: AddressInput;
}

export async function initializeQuestion(a: InitializeQuestionArgs): Promise<TransactionInstruction> {
  const question = (await fpda.question(a.questionId, a.oracle, a.numOutcomes)).address;
  const eventAuthority = (await fpda.vaultEventAuthority()).address;
  return new TransactionInstruction({
    programId: addr(CONDITIONAL_VAULT_ID),
    keys: [
      w(question),
      w(a.payer, true),
      ro(SYSTEM_PROGRAM_ID),
      ro(eventAuthority),
      ro(CONDITIONAL_VAULT_ID),
    ],
    data: concat([
      DISC.initializeQuestion,
      a.questionId,
      addr(a.oracle).toBytes(),
      u8b(a.numOutcomes),
    ]),
  });
}

export interface InitializeConditionalVaultArgs {
  question: AddressInput;
  underlyingMint: AddressInput;
  payer: AddressInput;
  /** Number of outcomes → that many conditional-token mints created (default 2). */
  numOutcomes?: number;
}

export async function initializeConditionalVault(
  a: InitializeConditionalVaultArgs,
): Promise<TransactionInstruction> {
  const n = a.numOutcomes ?? 2;
  const vault = (await fpda.conditionalVault(a.question, a.underlyingMint)).address;
  const vaultUnderlying = await ata(vault, a.underlyingMint);
  const eventAuthority = (await fpda.vaultEventAuthority()).address;
  const condMints: AccountMeta[] = [];
  for (let i = 0; i < n; i++) {
    condMints.push(w((await fpda.conditionalTokenMint(vault, i)).address));
  }
  return new TransactionInstruction({
    programId: addr(CONDITIONAL_VAULT_ID),
    keys: [
      w(vault),
      ro(a.question),
      ro(a.underlyingMint),
      w(vaultUnderlying),
      w(a.payer, true),
      ro(TOKEN_PROGRAM_ID),
      ro(ATA_PROGRAM_ID),
      ro(SYSTEM_PROGRAM_ID),
      ro(eventAuthority),
      ro(CONDITIONAL_VAULT_ID),
      ...condMints,
    ],
    data: DISC.initializeConditionalVault,
  });
}

export interface InteractWithVaultArgs {
  question: AddressInput;
  vault: AddressInput;
  vaultUnderlying: AddressInput;
  /** Signer that owns the user token accounts. */
  authority: AddressInput;
  userUnderlying: AddressInput;
  /** Conditional-token mints, outcome order (index 0..n). */
  conditionalMints: AddressInput[];
  /** User's conditional-token accounts, outcome order (index 0..n). */
  userConditionalAccounts: AddressInput[];
}

async function interactWithVault(disc: Uint8Array, a: InteractWithVaultArgs): Promise<TransactionInstruction> {
  const eventAuthority = (await fpda.vaultEventAuthority()).address;
  return new TransactionInstruction({
    programId: addr(CONDITIONAL_VAULT_ID),
    keys: [
      ro(a.question),
      w(a.vault),
      w(a.vaultUnderlying),
      ro(a.authority, true),
      w(a.userUnderlying),
      ro(TOKEN_PROGRAM_ID),
      ro(eventAuthority),
      ro(CONDITIONAL_VAULT_ID),
      ...a.conditionalMints.map((m) => w(m)),
      ...a.userConditionalAccounts.map((u) => w(u)),
    ],
    data: disc,
  });
}

/** `split_tokens` — mints `amount` of each conditional token, pulls underlying in. */
export function splitTokens(a: InteractWithVaultArgs & { amount: bigint | number }): Promise<TransactionInstruction> {
  return interactWithVault(concat([DISC.splitTokens, u64le(a.amount)]), a);
}

/** `merge_tokens` — burns `amount` of each conditional token, returns underlying. */
export function mergeTokens(a: InteractWithVaultArgs & { amount: bigint | number }): Promise<TransactionInstruction> {
  return interactWithVault(concat([DISC.mergeTokens, u64le(a.amount)]), a);
}

/** `redeem_tokens` — burns the holder's full balances, pays out per resolution. */
export function redeemTokens(a: InteractWithVaultArgs): Promise<TransactionInstruction> {
  return interactWithVault(DISC.redeemTokens, a);
}

export interface ResolveQuestionArgs {
  question: AddressInput;
  /** The question's oracle (signer). */
  oracle: AddressInput;
  /** Binary payout numerators — `[1,0]` pass-side, `[0,1]` fail-side. */
  payoutNumerators: [number, number];
}

export async function resolveQuestion(a: ResolveQuestionArgs): Promise<TransactionInstruction> {
  const eventAuthority = (await fpda.vaultEventAuthority()).address;
  return new TransactionInstruction({
    programId: addr(CONDITIONAL_VAULT_ID),
    keys: [w(a.question), ro(a.oracle, true), ro(eventAuthority), ro(CONDITIONAL_VAULT_ID)],
    data: concat([
      DISC.resolveQuestion,
      u32le(2),
      u32le(a.payoutNumerators[0]),
      u32le(a.payoutNumerators[1]),
    ]),
  });
}

// ════════════════════════════════════════════════════════════════════════════
// futarchy v0.6
// ════════════════════════════════════════════════════════════════════════════

export interface InitializeDaoArgs {
  /** Signer that seeds the Dao PDA (`[b"dao", dao_creator, nonce]`). */
  daoCreator: AddressInput;
  /** Rent payer + signer. */
  payer: AddressInput;
  /** DAO base token mint (e.g. KASS). */
  baseMint: AddressInput;
  /** DAO quote token mint (must be 6-decimal, e.g. USDC). */
  quoteMint: AddressInput;
  /**
   * Squads `ProgramConfig.treasury`. Read from the on-chain Squads ProgramConfig
   * account (not a PDA). G3 fetches it live; offline callers pass it explicitly.
   */
  squadsProgramConfigTreasury: AddressInput;
  // InitializeDaoParams (Borsh; initial_spending_limit forced to None)
  twapInitialObservation: bigint | number;
  twapMaxObservationChangePerUpdate: bigint | number;
  twapStartDelaySeconds: number;
  minQuoteFutarchicLiquidity: bigint | number;
  minBaseFutarchicLiquidity: bigint | number;
  baseToStake: bigint | number;
  passThresholdBps: number;
  secondsPerProposal: number;
  nonce: bigint | number;
  /**
   * v0.6.1 trailing param `team_sponsored_pass_threshold_bps: i16` (default 0).
   * The DEPLOYED program (0.6.1) added this after `initial_spending_limit`; the
   * v0.6.0 source lacked it. Confirmed against the on-chain Anchor IDL.
   */
  teamSponsoredPassThresholdBps?: number;
  /** v0.6.1 trailing param `team_address: Pubkey` (default the zero/system key). */
  teamAddress?: AddressInput;
}

/**
 * `initialize_dao` (initial_spending_limit == None). Creates the Dao AND, via an
 * internal CPI, the Squads multisig with `create_key == Dao` + vault index 0
 * (see NOTES.md). All Squads/ATA PDAs are derived internally.
 *
 * Arg layout = v0.6.1 DEPLOYED (`InitializeDaoParams` + the two trailing
 * `team_*` fields). 117 bytes for the None spending-limit case.
 */
export async function initializeDao(a: InitializeDaoArgs): Promise<TransactionInstruction> {
  const dao = (await fpda.dao(a.daoCreator, a.nonce)).address;
  const multisig = (await fpda.squadsMultisig(dao)).address;
  const vault = (await fpda.squadsVault(multisig, 0)).address;
  const programConfig = (await fpda.squadsProgramConfig()).address;
  const spendingLimit = (await fpda.squadsSpendingLimit(multisig, dao)).address;
  const baseVault = await ata(dao, a.baseMint);
  const quoteVault = await ata(dao, a.quoteMint);
  const eventAuthority = (await fpda.futarchyEventAuthority()).address;

  const data = concat([
    DISC.initializeDao,
    u128le(a.twapInitialObservation),
    u128le(a.twapMaxObservationChangePerUpdate),
    u32le(a.twapStartDelaySeconds),
    u64le(a.minQuoteFutarchicLiquidity),
    u64le(a.minBaseFutarchicLiquidity),
    u64le(a.baseToStake),
    u16le(a.passThresholdBps),
    u32le(a.secondsPerProposal),
    u64le(a.nonce),
    Uint8Array.from([0]), // Option<InitialSpendingLimit>::None
    i16le(a.teamSponsoredPassThresholdBps ?? 0), // v0.6.1
    addr(a.teamAddress ?? SYSTEM_PROGRAM_ID).toBytes(), // v0.6.1 team_address (default zero key)
  ]);

  return new TransactionInstruction({
    programId: addr(FUTARCHY_ID),
    keys: [
      w(dao),
      ro(a.daoCreator, true),
      w(a.payer, true),
      ro(SYSTEM_PROGRAM_ID),
      ro(a.baseMint),
      ro(a.quoteMint),
      w(multisig),
      ro(vault),
      ro(SQUADS_V4_ID),
      ro(programConfig),
      w(a.squadsProgramConfigTreasury),
      w(spendingLimit),
      w(baseVault),
      w(quoteVault),
      ro(TOKEN_PROGRAM_ID),
      ro(ATA_PROGRAM_ID),
      ro(eventAuthority),
      ro(FUTARCHY_ID),
    ],
    data,
  });
}

export interface InitializeProposalArgs {
  /** The Squads proposal the futarchy proposal references (seeds the Proposal PDA). */
  squadsProposal: AddressInput;
  /** The Squads multisig (v0.6.1 added it as an explicit account). */
  squadsMultisig: AddressInput;
  dao: AddressInput;
  question: AddressInput;
  quoteVault: AddressInput;
  baseVault: AddressInput;
  proposer: AddressInput;
  payer: AddressInput;
}

export async function initializeProposal(a: InitializeProposalArgs): Promise<TransactionInstruction> {
  const proposal = (await fpda.proposal(a.squadsProposal)).address;
  const eventAuthority = (await fpda.futarchyEventAuthority()).address;
  return new TransactionInstruction({
    programId: addr(FUTARCHY_ID),
    keys: [
      w(proposal),
      ro(a.squadsProposal),
      ro(a.squadsMultisig),
      w(a.dao),
      ro(a.question),
      ro(a.quoteVault),
      ro(a.baseVault),
      ro(a.proposer, true),
      w(a.payer, true),
      ro(SYSTEM_PROGRAM_ID),
      ro(eventAuthority),
      ro(FUTARCHY_ID),
    ],
    data: DISC.initializeProposal,
  });
}

export interface LaunchProposalArgs {
  proposal: AddressInput;
  baseVault: AddressInput;
  quoteVault: AddressInput;
  passBaseMint: AddressInput;
  passQuoteMint: AddressInput;
  failBaseMint: AddressInput;
  failQuoteMint: AddressInput;
  dao: AddressInput;
  payer: AddressInput;
  ammPassBaseVault: AddressInput;
  ammPassQuoteVault: AddressInput;
  ammFailBaseVault: AddressInput;
  ammFailQuoteVault: AddressInput;
  /** v0.6.1 added the Squads multisig + proposal as explicit accounts. */
  squadsMultisig: AddressInput;
  squadsProposal: AddressInput;
}

export async function launchProposal(a: LaunchProposalArgs): Promise<TransactionInstruction> {
  const eventAuthority = (await fpda.futarchyEventAuthority()).address;
  return new TransactionInstruction({
    programId: addr(FUTARCHY_ID),
    keys: [
      w(a.proposal),
      ro(a.baseVault),
      ro(a.quoteVault),
      ro(a.passBaseMint),
      ro(a.passQuoteMint),
      ro(a.failBaseMint),
      ro(a.failQuoteMint),
      w(a.dao),
      w(a.payer, true),
      w(a.ammPassBaseVault),
      w(a.ammPassQuoteVault),
      w(a.ammFailBaseVault),
      w(a.ammFailQuoteVault),
      ro(a.squadsMultisig),
      ro(a.squadsProposal),
      ro(SYSTEM_PROGRAM_ID),
      ro(TOKEN_PROGRAM_ID),
      ro(ATA_PROGRAM_ID),
      ro(eventAuthority),
      ro(FUTARCHY_ID),
    ],
    data: DISC.launchProposal,
  });
}

export interface FinalizeProposalArgs {
  proposal: AddressInput;
  dao: AddressInput;
  question: AddressInput;
  squadsProposal: AddressInput;
  squadsMultisig: AddressInput;
  ammPassBaseVault: AddressInput;
  ammPassQuoteVault: AddressInput;
  ammFailBaseVault: AddressInput;
  ammFailQuoteVault: AddressInput;
  ammBaseVault: AddressInput;
  ammQuoteVault: AddressInput;
  quoteVault: AddressInput;
  quoteVaultUnderlying: AddressInput;
  passQuoteMint: AddressInput;
  failQuoteMint: AddressInput;
  passBaseMint: AddressInput;
  failBaseMint: AddressInput;
  baseVault: AddressInput;
  baseVaultUnderlying: AddressInput;
}

export async function finalizeProposal(a: FinalizeProposalArgs): Promise<TransactionInstruction> {
  const eventAuthority = (await fpda.futarchyEventAuthority()).address;
  const vaultEventAuthority = (await fpda.vaultEventAuthority()).address;
  return new TransactionInstruction({
    programId: addr(FUTARCHY_ID),
    keys: [
      w(a.proposal),
      w(a.dao),
      w(a.question),
      w(a.squadsProposal),
      ro(a.squadsMultisig),
      ro(SQUADS_V4_ID),
      w(a.ammPassBaseVault),
      w(a.ammPassQuoteVault),
      w(a.ammFailBaseVault),
      w(a.ammFailQuoteVault),
      w(a.ammBaseVault),
      w(a.ammQuoteVault),
      ro(CONDITIONAL_VAULT_ID),
      ro(vaultEventAuthority),
      ro(TOKEN_PROGRAM_ID),
      w(a.quoteVault),
      w(a.quoteVaultUnderlying),
      w(a.passQuoteMint),
      w(a.failQuoteMint),
      w(a.passBaseMint),
      w(a.failBaseMint),
      w(a.baseVault),
      w(a.baseVaultUnderlying),
      ro(eventAuthority),
      ro(FUTARCHY_ID),
    ],
    data: DISC.finalizeProposal,
  });
}

export interface ConditionalSwapArgs {
  dao: AddressInput;
  ammBaseVault: AddressInput;
  ammQuoteVault: AddressInput;
  proposal: AddressInput;
  ammPassBaseVault: AddressInput;
  ammPassQuoteVault: AddressInput;
  ammFailBaseVault: AddressInput;
  ammFailQuoteVault: AddressInput;
  trader: AddressInput;
  userInputAccount: AddressInput;
  userOutputAccount: AddressInput;
  baseVault: AddressInput;
  baseVaultUnderlying: AddressInput;
  quoteVault: AddressInput;
  quoteVaultUnderlying: AddressInput;
  passBaseMint: AddressInput;
  failBaseMint: AddressInput;
  passQuoteMint: AddressInput;
  failQuoteMint: AddressInput;
  question: AddressInput;
  /** `Market.Pass` or `Market.Fail` (Spot is rejected on-chain). */
  market: Market;
  swapType: SwapType;
  inputAmount: bigint | number;
  minOutputAmount: bigint | number;
}

export async function conditionalSwap(a: ConditionalSwapArgs): Promise<TransactionInstruction> {
  const eventAuthority = (await fpda.futarchyEventAuthority()).address;
  const vaultEventAuthority = (await fpda.vaultEventAuthority()).address;
  return new TransactionInstruction({
    programId: addr(FUTARCHY_ID),
    keys: [
      w(a.dao),
      w(a.ammBaseVault),
      w(a.ammQuoteVault),
      ro(a.proposal),
      w(a.ammPassBaseVault),
      w(a.ammPassQuoteVault),
      w(a.ammFailBaseVault),
      w(a.ammFailQuoteVault),
      ro(a.trader, true),
      w(a.userInputAccount),
      w(a.userOutputAccount),
      w(a.baseVault),
      w(a.baseVaultUnderlying),
      w(a.quoteVault),
      w(a.quoteVaultUnderlying),
      w(a.passBaseMint),
      w(a.failBaseMint),
      w(a.passQuoteMint),
      w(a.failQuoteMint),
      ro(CONDITIONAL_VAULT_ID),
      ro(vaultEventAuthority),
      ro(a.question),
      ro(TOKEN_PROGRAM_ID),
      ro(eventAuthority),
      ro(FUTARCHY_ID),
    ],
    data: concat([
      DISC.conditionalSwap,
      u8b(a.market),
      u8b(a.swapType),
      u64le(a.inputAmount),
      u64le(a.minOutputAmount),
    ]),
  });
}

export interface SpotSwapArgs {
  dao: AddressInput;
  userBaseAccount: AddressInput;
  userQuoteAccount: AddressInput;
  ammBaseVault: AddressInput;
  ammQuoteVault: AddressInput;
  user: AddressInput;
  inputAmount: bigint | number;
  swapType: SwapType;
  minOutputAmount: bigint | number;
}

/** `spot_swap` — trades against the embedded spot AMM, cranking its TWAP. */
export async function spotSwap(a: SpotSwapArgs): Promise<TransactionInstruction> {
  const eventAuthority = (await fpda.futarchyEventAuthority()).address;
  return new TransactionInstruction({
    programId: addr(FUTARCHY_ID),
    keys: [
      w(a.dao),
      w(a.userBaseAccount),
      w(a.userQuoteAccount),
      w(a.ammBaseVault),
      w(a.ammQuoteVault),
      ro(a.user, true),
      ro(TOKEN_PROGRAM_ID),
      ro(eventAuthority),
      ro(FUTARCHY_ID),
    ],
    data: concat([
      DISC.spotSwap,
      u64le(a.inputAmount),
      u8b(a.swapType),
      u64le(a.minOutputAmount),
    ]),
  });
}

export interface ProvideLiquidityArgs {
  dao: AddressInput;
  liquidityProvider: AddressInput;
  liquidityProviderBaseAccount: AddressInput;
  liquidityProviderQuoteAccount: AddressInput;
  payer: AddressInput;
  ammBaseVault: AddressInput;
  ammQuoteVault: AddressInput;
  /** Position authority (seeds the AmmPosition PDA; usually == liquidityProvider). */
  positionAuthority: AddressInput;
  quoteAmount: bigint | number;
  maxBaseAmount: bigint | number;
  minLiquidity: bigint | number;
}

/** `provide_liquidity` — seeds the embedded spot AMM (first deposit needs `quote >= MIN_QUOTE_LIQUIDITY`). */
export async function provideLiquidity(a: ProvideLiquidityArgs): Promise<TransactionInstruction> {
  const eventAuthority = (await fpda.futarchyEventAuthority()).address;
  const ammPosition = (await fpda.ammPosition(a.dao, a.positionAuthority)).address;
  return new TransactionInstruction({
    programId: addr(FUTARCHY_ID),
    keys: [
      w(a.dao),
      ro(a.liquidityProvider, true),
      w(a.liquidityProviderBaseAccount),
      w(a.liquidityProviderQuoteAccount),
      w(a.payer, true),
      ro(SYSTEM_PROGRAM_ID),
      w(a.ammBaseVault),
      w(a.ammQuoteVault),
      w(ammPosition),
      ro(TOKEN_PROGRAM_ID),
      ro(eventAuthority),
      ro(FUTARCHY_ID),
    ],
    data: concat([
      DISC.provideLiquidity,
      u64le(a.quoteAmount),
      u64le(a.maxBaseAmount),
      u128le(a.minLiquidity),
      addr(a.positionAuthority).toBytes(),
    ]),
  });
}

export interface CollectMeteoraDammFeesArgs {
  /** The futarchy `Dao` PDA (its Squads multisig + vault are derived from it). */
  dao: AddressInput;
  /**
   * `admin` — writable signer + rent payer for the staged Squads txn/proposal.
   * Under the `production` feature the program requires this == {@link METADAO_ADMIN}
   * (the default).
   */
  admin?: AddressInput;
  /**
   * The NEW Squads transaction index (== `multisig.transaction_index + 1` at
   * call time). Seeds the `squads_multisig_vault_transaction` + `squads_multisig_proposal`.
   */
  transactionIndex: bigint | number;
  // ── Meteora cp-amm (claim_position_fee) accounts ──
  /** The Meteora cp-amm `Pool` the DAO holds a position in. */
  pool: AddressInput;
  /** The DAO's Meteora `Position`. */
  position: AddressInput;
  /** cp-amm token-A vault (base). */
  tokenAVault: AddressInput;
  /** cp-amm token-B vault (quote). */
  tokenBVault: AddressInput;
  /** Base mint (must equal `dao.base_mint`). */
  tokenAMint: AddressInput;
  /** Quote mint (must equal `dao.quote_mint`). */
  tokenBMint: AddressInput;
  /** Token account holding the position NFT. */
  positionNftAccount: AddressInput;
  /** Position owner (per the handler, usually the DAO's Squads vault). */
  owner: AddressInput;
  /**
   * Base fee-recipient token account. Defaults to
   * `ata({@link METADAO_MULTISIG_VAULT}, tokenAMint)` (the on-chain
   * `associated_token::authority` constraint).
   */
  tokenAAccount?: AddressInput;
  /** Quote fee-recipient token account. Defaults to `ata(METADAO_MULTISIG_VAULT, tokenBMint)`. */
  tokenBAccount?: AddressInput;
  /** Token program owning base mint (SPL Token by default). */
  tokenAProgram?: AddressInput;
  /** Token program owning quote mint (SPL Token by default). */
  tokenBProgram?: AddressInput;
  /** The permissionless Squads member (signer). Defaults to {@link SQUADS_PERMISSIONLESS_MEMBER}. */
  permissionlessAccount?: AddressInput;
}

/**
 * `collect_meteora_damm_fees` — **MetaDAO's protocol-rake op; NOT a Kassandra
 * dependency.** The futarchy program sweeps a DAO's Meteora cp-amm position fees
 * to **MetaDAO's OWN protocol vault** (`token_{a,b}_account` are
 * `associated_token::authority = metadao_multisig_vault::ID` = `6awyHMsh…`),
 * gated on **MetaDAO's keeper** (`require_keys_eq!(admin, metadao_admin::ID =
 * tSTp6B6k…)` under `production`). It builds a cp-amm `claim_position_fee` CPI,
 * stages it in the DAO's Squads multisig (`vault_transaction_create` →
 * `proposal_create` → `proposal_approve` → `vault_transaction_execute`, all CPI'd
 * internally), so the DAO's Squads vault signs the actual claim. NO positional
 * args (disc only).
 *
 * **Kassandra does NOT call this.** The DAO collects its OWN Meteora treasury fees
 * ADMIN-FREE — position owned by the DAO's Squads vault, claim authorized by the
 * DAO's own futarchy governance, fees → the DAO's OWN ATAs — via the M1
 * `meteora.claimPositionFee` builder wrapped in a Squads `vault_transaction`
 * (see `test/surfpool/dao-meteora-treasury-e2e.test.ts` / NOTES.md "D1"). This
 * builder is kept only as a faithful, wire-verified pin of the deployed
 * instruction (F2a byte test + F2b live admin-gate + D2 litesvm full-drive).
 *
 * Wire format PINNED from TWO authoritative sources that AGREE exactly (27
 * accounts incl. the `#[event_cpi]` tail, no args):
 *   (a) metaDAOproject/programs@c1000ed84ef6d084203ad2a9c13940fd14feb53c
 *       `programs/futarchy/src/instructions/collect_meteora_damm_fees.rs`
 *       (declare_id == FUTAREL…, Cargo.toml version 0.6.1) + `lib.rs:158`.
 *   (b) the on-chain Anchor IDL of `FUTARELBfJfQ8RDGhg1wdhddq1odMAJUePHFuBYfUxKq`
 *       (v0.6.1) — instruction `collectMeteoraDammFees`.
 * disc = sha256("global:collect_meteora_damm_fees")[..8] = 8bd469767e36d68f.
 */
export async function collectMeteoraDammFees(
  a: CollectMeteoraDammFeesArgs,
): Promise<TransactionInstruction> {
  const admin = a.admin ?? METADAO_ADMIN;
  const permissionless = a.permissionlessAccount ?? SQUADS_PERMISSIONLESS_MEMBER;
  const tokenAProgram = a.tokenAProgram ?? TOKEN_PROGRAM_ID;
  const tokenBProgram = a.tokenBProgram ?? TOKEN_PROGRAM_ID;

  const multisig = (await fpda.squadsMultisig(a.dao)).address;
  const squadsVault = (await fpda.squadsVault(multisig, 0)).address;
  const squadsTransaction = (await fpda.squadsTransaction(multisig, a.transactionIndex)).address;
  const squadsProposal = (await fpda.squadsProposal(multisig, a.transactionIndex)).address;
  const eventAuthority = (await fpda.futarchyEventAuthority()).address;

  const dammEventAuthority = (await mpda.eventAuthority()).address;
  // pool_authority::ID is hard-coded in the handler; == the cp-amm [b"pool_authority"] PDA.
  const poolAuthority = DAMM_V2_POOL_AUTHORITY;

  const tokenAAccount = a.tokenAAccount ?? (await ata(METADAO_MULTISIG_VAULT, a.tokenAMint));
  const tokenBAccount = a.tokenBAccount ?? (await ata(METADAO_MULTISIG_VAULT, a.tokenBMint));

  return new TransactionInstruction({
    programId: addr(FUTARCHY_ID),
    keys: [
      w(a.dao),
      w(admin, true),
      w(multisig),
      w(squadsVault),
      w(squadsTransaction),
      w(squadsProposal),
      ro(permissionless, true),
      // meteora_claim_position_fees_accounts (flattened, IDL order)
      ro(METEORA_DAMM_V2_ID),
      ro(dammEventAuthority),
      ro(poolAuthority),
      ro(a.pool),
      w(a.position),
      w(tokenAAccount),
      w(tokenBAccount),
      w(a.tokenAVault),
      w(a.tokenBVault),
      ro(a.tokenAMint),
      ro(a.tokenBMint),
      ro(a.positionNftAccount),
      ro(a.owner),
      ro(tokenAProgram),
      ro(tokenBProgram),
      // trailing programs + event_cpi tail
      ro(SYSTEM_PROGRAM_ID),
      ro(TOKEN_PROGRAM_ID),
      ro(SQUADS_V4_ID),
      ro(eventAuthority),
      ro(FUTARCHY_ID),
    ],
    data: DISC.collectMeteoraDammFees,
  });
}

// ════════════════════════════════════════════════════════════════════════════
// Squads v4
// ════════════════════════════════════════════════════════════════════════════

export interface VaultTransactionCreateArgs {
  multisig: AddressInput;
  /** Multisig member that initiates (the Dao PDA in the futarchy flow). */
  creator: AddressInput;
  rentPayer: AddressInput;
  /** Index of the new transaction = multisig.transaction_index + 1. */
  transactionIndex: bigint | number;
  vaultIndex?: number;
  ephemeralSigners?: number;
  /** Squads compact `TransactionMessage` bytes (the staged inner CPI). */
  transactionMessage: Uint8Array;
  memo?: string | null;
}

export async function vaultTransactionCreate(a: VaultTransactionCreateArgs): Promise<TransactionInstruction> {
  const transaction = (await fpda.squadsTransaction(a.multisig, a.transactionIndex)).address;
  return new TransactionInstruction({
    programId: addr(SQUADS_V4_ID),
    keys: [
      w(a.multisig),
      w(transaction),
      ro(a.creator, true),
      w(a.rentPayer, true),
      ro(SYSTEM_PROGRAM_ID),
    ],
    data: concat([
      DISC.vaultTransactionCreate,
      u8b(a.vaultIndex ?? 0),
      u8b(a.ephemeralSigners ?? 0),
      vecU8(a.transactionMessage),
      optString(a.memo),
    ]),
  });
}

export interface ProposalCreateArgs {
  multisig: AddressInput;
  creator: AddressInput;
  rentPayer: AddressInput;
  transactionIndex: bigint | number;
  draft?: boolean;
}

export async function proposalCreate(a: ProposalCreateArgs): Promise<TransactionInstruction> {
  const proposal = (await fpda.squadsProposal(a.multisig, a.transactionIndex)).address;
  return new TransactionInstruction({
    programId: addr(SQUADS_V4_ID),
    keys: [
      ro(a.multisig),
      w(proposal),
      ro(a.creator, true),
      w(a.rentPayer, true),
      ro(SYSTEM_PROGRAM_ID),
    ],
    data: concat([DISC.proposalCreate, u64le(a.transactionIndex), boolb(a.draft ?? false)]),
  });
}

export interface VaultTransactionExecuteArgs {
  multisig: AddressInput;
  transactionIndex: bigint | number;
  /** Multisig member that executes (the Dao PDA — has Execute permission). */
  member: AddressInput;
  /**
   * The inner transaction's accounts, in Squads message order (ALT accounts +
   * `message.account_keys`). Composing these is a G3 concern; pass them through.
   */
  remainingAccounts?: AccountMeta[];
}

/** `vault_transaction_execute` — no args; signs the inner CPIs as the vault PDA. */
export async function vaultTransactionExecute(a: VaultTransactionExecuteArgs): Promise<TransactionInstruction> {
  const proposal = (await fpda.squadsProposal(a.multisig, a.transactionIndex)).address;
  const transaction = (await fpda.squadsTransaction(a.multisig, a.transactionIndex)).address;
  return new TransactionInstruction({
    programId: addr(SQUADS_V4_ID),
    keys: [
      ro(a.multisig),
      w(proposal),
      ro(transaction),
      ro(a.member, true),
      ...(a.remainingAccounts ?? []),
    ],
    data: DISC.vaultTransactionExecute,
  });
}
