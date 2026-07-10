/**
 * Batch "create all N outcomes" flow — a pure convenience wrapper over the
 * per-outcome `createMarket` builder.
 *
 * A categorical Kassandra oracle (`options_count > 2`) has ONE binary sub-market
 * per outcome, each keyed by `(oracle, outcomeIndex)`. Creating them one at a
 * time is tedious, so {@link createAllOutcomeMarkets} composes the full set:
 * for every `outcomeIndex` in `0..optionsCount-1` it builds the `createMarket`
 * instruction and derives that outcome's market PDA. It is PURE (no RPC) — each
 * `create_market` is still its own on-chain instruction; the caller sends them
 * as a resumable multi-tx sequence (each reverts if its market PDA already
 * exists, so a re-run skips already-created outcomes).
 *
 * One shared `seedAmount` applies to every outcome; the creator funds
 * `optionsCount × seed` KASS in total.
 */
import { Address, type TransactionInstruction } from "@solana/web3.js";

import { createMarket } from "../instructions/market.js";
import * as pda from "../pda.js";
import type { AddressInput } from "../pda.js";

export interface CreateAllParams {
  /** The categorical Kassandra oracle every sub-market resolves against. */
  oracle: AddressInput;
  /** The oracle's `options_count` — one sub-market is created per outcome. */
  optionsCount: number;
  /** Creator authority (the signer): pays rent + seeds each contribution. */
  creator: AddressInput;
  /** Canonical KASS mint (== `config.kass_mint`). */
  kassMint: AddressInput;
  /** Creator's KASS token account each seed transfers from. */
  creatorKassAta: AddressInput;
  /** KASS seeded into each outcome's escrow (raw base units); charged per outcome. */
  seedAmount: bigint | number;
  /** Override the program id (defaults to the SDK's `MARKET_PROGRAM_ID`). */
  programId?: Address;
}

/** One outcome's create step: its index, its market PDA, and the ix to send. */
export interface CreateAllStep {
  /** The oracle outcome this sub-market binds to (`0 <= i < optionsCount`). */
  outcomeIndex: number;
  /** The `Market` PDA this step creates (`pda.market(oracle, outcomeIndex)`). */
  market: Address;
  /** The `create_market` instruction for this outcome. */
  instruction: TransactionInstruction;
}

/**
 * Build the full per-outcome `createMarket` set for a categorical oracle — one
 * {@link CreateAllStep} for each `outcomeIndex` in `0..optionsCount-1`, in order.
 * Pure: no RPC, no signing. The caller sends the instructions as a sequence.
 */
export async function createAllOutcomeMarkets(
  params: CreateAllParams,
): Promise<{ steps: CreateAllStep[] }> {
  const {
    oracle,
    optionsCount,
    creator,
    kassMint,
    creatorKassAta,
    seedAmount,
    programId,
  } = params;

  if (!Number.isInteger(optionsCount) || optionsCount < 1) {
    throw new Error(`optionsCount must be a positive integer, got ${optionsCount}`);
  }

  const steps = await Promise.all(
    Array.from({ length: optionsCount }, async (_unused, outcomeIndex) => {
      const market = (await pda.market(oracle, outcomeIndex, programId)).address;
      const instruction = await createMarket({
        creator,
        oracle,
        kassMint,
        creatorKassAta,
        seedAmount,
        outcomeIndex,
        programId,
      });
      return { outcomeIndex, market, instruction };
    }),
  );

  return { steps };
}
