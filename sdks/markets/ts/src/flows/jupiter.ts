/**
 * Thin Jupiter any-token-entry helper.
 *
 * kassandra-market settles in KASS, but a trader may hold any SPL token (USDC,
 * SOL, …). The "any-token entry" boundary is: swap the trader's input token into
 * KASS via Jupiter, then feed that KASS straight into a market `buy`. This module
 * is deliberately OFFLINE and network-free — it only:
 *
 *   1. {@link buildJupiterEntryRequest} — shapes the typed request params you POST
 *      to the Jupiter v6 quote + swap API (`quote-api.jup.ag`). The SDK does NOT
 *      fetch; the APP performs the HTTP calls (so the SDK stays dependency-free
 *      and unit-testable), then deserializes the returned swap transaction into a
 *      `TransactionInstruction`.
 *   2. {@link composeWithEntry} — stitches that app-fetched Jupiter swap
 *      instruction in FRONT of the market instructions, yielding one ordered list
 *      (`[jupiterSwapIx, ...marketInstructions]`) — the Jupiter swap produces KASS,
 *      the market instructions immediately consume it.
 *
 * ── The boundary (important) ──────────────────────────────────────────────────
 *   SDK  : shapes the request (this file) + combines instructions.
 *   APP  : does the actual `fetch` to Jupiter, and turns the returned base64
 *          swap transaction into a `TransactionInstruction`.
 * No test in this SDK hits the network.
 */
import type { TransactionInstruction } from "@solana/web3.js";

import type { AddressInput } from "../pda.js";

/** Stringify any address-like input to the base58 form the Jupiter API expects. */
function s(a: AddressInput): string {
  return typeof a === "string" ? a : a.toString();
}

export interface JupiterEntryParams {
  /** The trader's input mint (what they're paying with, e.g. USDC/SOL). */
  inputMint: AddressInput;
  /** The output mint — for a market entry this is the KASS mint. */
  outputMint: AddressInput;
  /** Input amount in the input mint's base units. */
  amount: bigint | number;
  /** Max slippage in basis points (e.g. 50 = 0.5%). */
  slippageBps: number;
  /** The trader's wallet (fee payer + token authority for the swap). */
  userPublicKey: AddressInput;
  /** Restrict to direct routes only (default false). */
  onlyDirectRoutes?: boolean;
  /** Wrap/unwrap SOL automatically when SOL is an endpoint (default true). */
  wrapAndUnwrapSol?: boolean;
}

/** Typed params for the Jupiter v6 `GET /quote` endpoint. */
export interface JupiterQuoteRequest {
  inputMint: string;
  outputMint: string;
  amount: string;
  slippageBps: number;
  onlyDirectRoutes: boolean;
  swapMode: "ExactIn";
}

/** Typed params for the Jupiter v6 `POST /swap` endpoint body. */
export interface JupiterSwapRequest {
  userPublicKey: string;
  wrapAndUnwrapSol: boolean;
  /**
   * Placeholder for the `quoteResponse` the app obtains from `GET /quote`. The SDK
   * cannot produce it (it requires the live quote), so the app fills it before
   * POSTing. Typed as `unknown` to avoid pinning Jupiter's response schema here.
   */
  quoteResponse?: unknown;
}

/**
 * The full, offline-shaped Jupiter v6 entry request: the `quote` query params and
 * the `swap` body the APP will POST (after filling `swap.quoteResponse` with the
 * quote result). NO network call happens here.
 */
export interface JupiterEntryRequest {
  /** Base URL of the Jupiter v6 API (override for self-hosted). */
  baseUrl: string;
  /** `GET {baseUrl}/quote?<quote params>`. */
  quote: JupiterQuoteRequest;
  /** `POST {baseUrl}/swap` body (set `.quoteResponse` from the quote first). */
  swap: JupiterSwapRequest;
}

/** Default public Jupiter v6 quote API host. */
export const JUPITER_V6_BASE_URL = "https://quote-api.jup.ag/v6";

/**
 * Shape (do NOT send) the Jupiter v6 quote + swap request for a "swap `inputMint`
 * → KASS (`outputMint`)" market entry. The app fetches `GET /quote` with
 * `request.quote`, assigns the result to `request.swap.quoteResponse`, POSTs
 * `request.swap` to `/swap`, then deserializes the returned swap transaction into
 * an instruction to pass to {@link composeWithEntry}.
 */
export function buildJupiterEntryRequest(
  params: JupiterEntryParams,
  baseUrl: string = JUPITER_V6_BASE_URL,
): JupiterEntryRequest {
  return {
    baseUrl,
    quote: {
      inputMint: s(params.inputMint),
      outputMint: s(params.outputMint),
      amount: BigInt(params.amount).toString(),
      slippageBps: params.slippageBps,
      onlyDirectRoutes: params.onlyDirectRoutes ?? false,
      swapMode: "ExactIn",
    },
    swap: {
      userPublicKey: s(params.userPublicKey),
      wrapAndUnwrapSol: params.wrapAndUnwrapSol ?? true,
    },
  };
}

/**
 * Combine the app-fetched Jupiter swap instruction with the market instructions:
 * `[jupiterSwapIx, ...marketInstructions]`. The Jupiter swap runs first (producing
 * KASS), then the market flow (e.g. `buyInstructions`) consumes it in the same
 * transaction.
 */
export function composeWithEntry(
  jupiterSwapIx: TransactionInstruction,
  marketInstructions: TransactionInstruction[],
): TransactionInstruction[] {
  return [jupiterSwapIx, ...marketInstructions];
}
