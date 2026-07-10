/**
 * High-level `compose → activate` keeper flow.
 *
 * Before a `kassandra-market` market can be activated, a client must stand up the
 * MetaDAO scaffolding it will bind to: a `Question` (resolver == the Market PDA),
 * a KASS `ConditionalVault` (minting cYES/cNO at outcome idx 0/1), and the
 * cYES/cNO `Amm` pool. {@link composeMarketInstructions} returns exactly that
 * ordered instruction list plus every derived address (`refs`) that the
 * subsequent `activate` needs, and {@link activateInstruction} wires those refs
 * (together with the market-PDA-owned cYES/cNO/LP holders it derives) into the
 * Task-2 `activate` builder.
 *
 * This is the TypeScript mirror of the Rust test harness `compose_metadao_market`
 * (`programs/kassandra-market/tests/common/mod.rs`) + `activate`. The twap /
 * observation constants below are copied from that harness verbatim so the pool
 * is seeded as a valid, balanced, EMPTY 50/50 book (the on-chain `activate`
 * rejects a non-empty pool).
 */
import { Address, type TransactionInstruction } from "@solana/web3.js";

import { EXTERNAL_PROGRAM_IDS } from "../constants.js";
import type { Market } from "../accounts/market.js";
import { activate as buildActivate } from "../instructions/market.js";
import * as metadao from "../metadao/index.js";
import * as pda from "../pda.js";
import type { AddressInput } from "../pda.js";
import { toAddr } from "./util.js";

// ── AMM seeding constants (mirror the Rust `compose_metadao_market`) ───────────

/**
 * `twap_initial_observation` passed to `create_amm` — 1e12, a balanced (price
 * 1.0) starting TWAP observation for a 50/50 binary book. Matches the Rust
 * harness `compose_metadao_market` (`1_000_000_000_000`).
 */
export const TWAP_INITIAL_OBSERVATION = 1_000_000_000_000n;

/**
 * `twap_max_observation_change_per_update` — `(u64::MAX) * 1e12`, effectively
 * unbounded per-update movement. Matches the Rust harness
 * (`(u64::MAX as u128) * 1_000_000_000_000`).
 */
export const TWAP_MAX_OBSERVATION_CHANGE_PER_UPDATE = (2n ** 64n - 1n) * 1_000_000_000_000n;

/** `twap_start_delay_slots` — 0 (no delay), matching the Rust harness. */
export const TWAP_START_DELAY_SLOTS = 0n;

// ── refs ───────────────────────────────────────────────────────────────────────

/**
 * Every address a composed MetaDAO market exposes — the precondition set for
 * `activate` (and later `trade` / `redeem`). Mirrors the Rust `MetaDaoRefs`,
 * extended with the fields `activate` needs (the event authorities + program ids)
 * and the market-PDA-owned holders.
 */
export interface MarketRefs {
  /** The kassandra-market `Market` PDA. */
  market: Address;
  /** The Kassandra oracle the market resolves against (also the question id). */
  oracle: Address;
  /** The KASS underlying mint. */
  kassMint: Address;
  /** MetaDAO `Question` (resolver == the Market PDA). */
  question: Address;
  /** KASS `ConditionalVault`. */
  vault: Address;
  /** The vault's KASS underlying ATA (split/merge/redeem destination). */
  vaultUnderlyingAta: Address;
  /** cYES conditional mint (outcome idx 0). */
  yesMint: Address;
  /** cNO conditional mint (outcome idx 1). */
  noMint: Address;
  /** cYES/cNO `Amm` pool. */
  amm: Address;
  /** The pool's LP mint. */
  lpMint: Address;
  /** The AMM's cYES (base) reserve ATA. */
  ammVaultBase: Address;
  /** The AMM's cNO (quote) reserve ATA. */
  ammVaultQuote: Address;
  /** conditional_vault `#[event_cpi]` event authority. */
  cvEventAuthority: Address;
  /** amm `#[event_cpi]` event authority. */
  ammEventAuthority: Address;
  /** Market-PDA-owned cYES holder (created at `activate`). */
  marketCyes: Address;
  /** Market-PDA-owned cNO holder (created at `activate`). */
  marketCno: Address;
  /** Market-PDA-owned LP holder (created at `activate`). */
  lpVault: Address;
  /** Market-PDA-owned KASS escrow (drained at `activate`). */
  escrow: Address;
}

/** Params for {@link marketRefsFromAccount}. */
export interface MarketRefsFromAccountParams {
  /** The kassandra-market `Market` PDA. */
  market: AddressInput;
  /** The decoded `Market` account (carries the composed bindings). */
  decoded: Market;
}

/**
 * Rebuild the composed {@link MarketRefs} for an already-Active market from its
 * PDA + decoded {@link Market} account — WITHOUT re-running {@link
 * composeMarketInstructions}.
 *
 * Once a market is Active the compose-time refs are no longer in memory, but the
 * `Market` account records the composed bindings (`question` / `vault` /
 * `yesMint` / `noMint` / `amm` / `lpMint` / `lpVault` / `escrowVault`). This
 * reads those and DERIVES the few addresses the account does not store (the
 * vault/AMM ATAs, the two `#[event_cpi]` event authorities, the market-PDA-owned
 * cYES/cNO holders) via `pda` / `metadao.pda`, reproducing EXACTLY what compose
 * built. A second consumer (a keeper, the surfpool harness) can reuse it.
 */
export async function marketRefsFromAccount(
  params: MarketRefsFromAccountParams,
): Promise<MarketRefs> {
  const market = toAddr(params.market);
  const { oracle, kassMint, question, vault, yesMint, noMint, amm, lpMint, lpVault, escrowVault } =
    params.decoded;

  const [
    vaultUnderlyingAta,
    ammVaultBase,
    ammVaultQuote,
    cvEventAuthority,
    ammEventAuthority,
    marketCyes,
    marketCno,
  ] = await Promise.all([
    metadao.pda.ata(vault, kassMint),
    metadao.pda.ata(amm, yesMint),
    metadao.pda.ata(amm, noMint),
    metadao.pda.vaultEventAuthority(),
    metadao.pda.ammEventAuthority(),
    pda.cyes(market),
    pda.cno(market),
  ]);

  return {
    market,
    oracle,
    kassMint,
    question,
    vault,
    vaultUnderlyingAta,
    yesMint,
    noMint,
    amm,
    lpMint,
    ammVaultBase,
    ammVaultQuote,
    cvEventAuthority: cvEventAuthority.address,
    ammEventAuthority: ammEventAuthority.address,
    marketCyes: marketCyes.address,
    marketCno: marketCno.address,
    lpVault,
    escrow: escrowVault,
  };
}

// ── compose ─────────────────────────────────────────────────────────────────────

export interface ComposeParams {
  /** The kassandra-market `Market` PDA (derive via `pda.market(oracle, outcomeIndex)`). */
  market: AddressInput;
  /** The Kassandra oracle (its 32 bytes become the MetaDAO question id). */
  oracle: AddressInput;
  /** The KASS underlying mint. */
  kassMint: AddressInput;
  /** Rent payer + signer for the three composition instructions. */
  payer: AddressInput;
}

/**
 * Build the ordered `initializeQuestion → initializeConditionalVault → createAmm`
 * instruction list a client sends BEFORE `activate`, plus every derived address.
 *
 * Each instruction may be sent in its own transaction (the Rust harness does) or
 * batched — but they must land in this order (the vault reads the question, the
 * AMM reads the mints). The MetaDAO composition + activate exceed the 200k default
 * compute budget, so callers should prepend a `SetComputeUnitLimit`.
 */
export async function composeMarketInstructions(
  params: ComposeParams,
): Promise<{ instructions: TransactionInstruction[]; refs: MarketRefs }> {
  const market = toAddr(params.market);
  const oracle = toAddr(params.oracle);
  const kassMint = toAddr(params.kassMint);
  const payer = toAddr(params.payer);

  // The MetaDAO question id is the raw 32 bytes of the Kassandra oracle address;
  // the question's resolver ("oracle") is the MARKET PDA.
  const questionId = oracle.toBytes();

  const question = (await metadao.pda.question(questionId, market, 2)).address;
  const vault = (await metadao.pda.conditionalVault(question, kassMint)).address;
  const vaultUnderlyingAta = await metadao.pda.ata(vault, kassMint);
  const yesMint = (await metadao.pda.conditionalTokenMint(vault, 0)).address;
  const noMint = (await metadao.pda.conditionalTokenMint(vault, 1)).address;
  const amm = (await metadao.pda.amm(yesMint, noMint)).address;
  const lpMint = (await metadao.pda.ammLpMint(amm)).address;
  const ammVaultBase = await metadao.pda.ata(amm, yesMint);
  const ammVaultQuote = await metadao.pda.ata(amm, noMint);
  const cvEventAuthority = (await metadao.pda.vaultEventAuthority()).address;
  const ammEventAuthority = (await metadao.pda.ammEventAuthority()).address;

  const marketCyes = (await pda.cyes(market)).address;
  const marketCno = (await pda.cno(market)).address;
  const lpVault = (await pda.lpVault(market)).address;
  const escrow = (await pda.escrow(market)).address;

  const ixQuestion = await metadao.initializeQuestion({
    payer,
    questionId,
    oracle: market, // resolver authority == the Market PDA
    numOutcomes: 2,
  });
  const ixVault = await metadao.initializeConditionalVault({
    payer,
    question,
    underlyingMint: kassMint,
    numOutcomes: 2,
  });
  const ixAmm = await metadao.createAmm({
    payer,
    baseMint: yesMint,
    quoteMint: noMint,
    twapInitialObservation: TWAP_INITIAL_OBSERVATION,
    twapMaxObservationChangePerUpdate: TWAP_MAX_OBSERVATION_CHANGE_PER_UPDATE,
    twapStartDelaySlots: TWAP_START_DELAY_SLOTS,
  });

  return {
    instructions: [ixQuestion, ixVault, ixAmm],
    refs: {
      market,
      oracle,
      kassMint,
      question,
      vault,
      vaultUnderlyingAta,
      yesMint,
      noMint,
      amm,
      lpMint,
      ammVaultBase,
      ammVaultQuote,
      cvEventAuthority,
      ammEventAuthority,
      marketCyes,
      marketCno,
      lpVault,
      escrow,
    },
  };
}

// ── activate ─────────────────────────────────────────────────────────────────────

export interface ActivateFlowParams {
  /** The composed refs from {@link composeMarketInstructions}. */
  refs: MarketRefs;
  /** Payer (signer): rent for the three new market-owned token accounts. */
  payer: AddressInput;
}

/**
 * Convenience over the Task-2 `activate` builder: wires a composed {@link MarketRefs}
 * (plus the market-PDA cYES/cNO/LP holders + escrow it carries) into a single
 * `activate` instruction. Send AFTER the {@link composeMarketInstructions} list has
 * landed. Needs a raised compute budget (the split + add_liquidity CPIs).
 */
export function activateInstruction(params: ActivateFlowParams): Promise<TransactionInstruction> {
  const { refs, payer } = params;
  return buildActivate({
    market: refs.market,
    oracle: refs.oracle,
    payer,
    question: refs.question,
    vault: refs.vault,
    vaultUnderlyingAta: refs.vaultUnderlyingAta,
    yesMint: refs.yesMint,
    noMint: refs.noMint,
    marketCyes: refs.marketCyes,
    marketCno: refs.marketCno,
    amm: refs.amm,
    lpMint: refs.lpMint,
    lpVault: refs.lpVault,
    ammVaultBase: refs.ammVaultBase,
    ammVaultQuote: refs.ammVaultQuote,
    cvEventAuthority: refs.cvEventAuthority,
    ammEventAuthority: refs.ammEventAuthority,
    cvProgram: EXTERNAL_PROGRAM_IDS.conditionalVault,
    ammProgram: EXTERNAL_PROGRAM_IDS.ammV04,
  });
}
