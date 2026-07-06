/**
 * The app's SOLE data + transaction gateway: a tiny `fetch`-based client over the
 * same-origin `/api/*` indexer HTTP surface (Phase 1). The app never talks to a
 * Solana RPC — the indexer decoded every account already, holds the RPC secret,
 * and relays transactions — so there is NO web3.js `Connection` anywhere in the
 * bundle. See `indexer/src/{json.rs,api.rs}` for the exact DTO shapes this mirrors
 * (pubkeys base58, every `u64` a string to dodge JS `Number` precision loss).
 */
import { createContext, useContext } from "react";
import { Address } from "@solana/web3.js";

/** `GET /api/config` — the program `Config` singleton. */
export interface ConfigDto {
  address: string;
  authority: string;
  kassMint: string;
  minLiquidity: string;
  bump: number;
  feeBps: number;
  feeDestination: string;
  slot: string;
}

/** One `Market` (from `GET /api/markets` or the `market` of a detail). */
export interface MarketDto {
  address: string;
  status: number;
  statusLabel: string;
  oracle: string;
  creator: string;
  kassMint: string;
  escrowVault: string;
  minLiquidity: string;
  totalContributed: string;
  openContributions: number;
  bump: number;
  escrowBump: number;
  outcomeIndex: number;
  feeBps: number;
  feeCollected: number;
  settled: number;
  question: string;
  vault: string;
  yesMint: string;
  noMint: string;
  amm: string;
  lpMint: string;
  lpVault: string;
  lpTotal: string;
  slot: string;
}

/** One `Contribution` in a market detail. */
export interface ContributionDto {
  market: string;
  contributor: string;
  amount: string;
  claimed: boolean;
  bump: number;
}

/** The linked Kassandra oracle's three read fields (enrichment). */
export interface OracleDto {
  optionsCount: number;
  phase: number;
  resolvedOption: number;
}

/** The market's cYES/cNO pool reserves (raw base units, string-encoded). */
export interface ReservesDto {
  base: string;
  quote: string;
}

/** `GET /api/markets/{pubkey}` — a market plus children + oracle + reserves. */
export interface MarketDetailDto {
  market: MarketDto;
  contributions: ContributionDto[];
  oracle: OracleDto | null;
  reserves: ReservesDto | null;
}

/** A raw on-demand account read (`getAccount`): decoded from `AccountDto`. */
export interface AccountRead {
  data: Uint8Array;
  owner: Address;
  lamports: bigint;
}

/** The `{ status, err? }` shape of `GET /api/transaction/{sig}`. */
export interface SignatureStatus {
  status: "processed" | "confirmed" | "finalized" | "failed" | "pending";
  err?: string | null;
}

/**
 * A relayed-transaction rejection (`POST /api/transaction` 4xx). Carries the
 * indexer's `error` text + any program `logs` so the write layer can humanize the
 * program error exactly as it did off a web3.js send error (`extractLogs` reads
 * `.logs`, `humanizeProgramError` reads the message).
 */
export class IndexerTxError extends Error {
  readonly logs?: string[];
  constructor(message: string, logs?: string[]) {
    super(message);
    this.name = "IndexerTxError";
    this.logs = logs;
  }
}

/** Decode standard base64 (indexer `AccountDto.data`) into raw bytes. */
function base64ToBytes(b64: string): Uint8Array {
  const bin = atob(b64);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

async function jsonOrThrow<T>(res: Response, what: string): Promise<T> {
  if (!res.ok) {
    let detail = res.statusText;
    try {
      const body = (await res.json()) as { error?: string };
      if (body?.error) detail = body.error;
    } catch {
      // non-JSON body — keep the status text.
    }
    throw new Error(`${what} failed (${res.status}): ${detail}`);
  }
  return (await res.json()) as T;
}

/**
 * The indexer HTTP client. All calls are same-origin `fetch` to `VITE_API_BASE`
 * (default `/api`) — the browser never learns the RPC or indexer URL (a proxy
 * fronts it). Reads map 404 → `null`; the tx relay throws {@link IndexerTxError}.
 */
export class IndexerClient {
  private readonly base: string;

  constructor(base: string = (import.meta.env.VITE_API_BASE as string | undefined) ?? "/api") {
    // Normalize a trailing slash so `${base}/config` is always well-formed.
    this.base = base.replace(/\/$/, "");
  }

  /** `GET /api/config` → the `Config` singleton, or `null` (404 = uninitialised). */
  async getConfig(): Promise<ConfigDto | null> {
    const res = await fetch(`${this.base}/config`);
    if (res.status === 404) return null;
    return jsonOrThrow<ConfigDto>(res, "getConfig");
  }

  /** `GET /api/markets` → every indexed market. */
  async getMarkets(): Promise<MarketDto[]> {
    const res = await fetch(`${this.base}/markets`);
    return jsonOrThrow<MarketDto[]>(res, "getMarkets");
  }

  /** `GET /api/markets/{pubkey}` → the detail payload, or `null` on 404. */
  async getMarket(pubkey: string): Promise<MarketDetailDto | null> {
    const res = await fetch(`${this.base}/markets/${pubkey}`);
    if (res.status === 404) return null;
    return jsonOrThrow<MarketDetailDto>(res, "getMarket");
  }

  /**
   * `GET /api/account/{pubkey}` → a raw account read (base64 → bytes), or `null`
   * when the account does not exist (404). Powers ATA-existence, step-landed, the
   * oracle read, and the KASS-balance decode.
   */
  async getAccount(pubkey: string): Promise<AccountRead | null> {
    const res = await fetch(`${this.base}/account/${pubkey}`);
    if (res.status === 404) return null;
    const dto = await jsonOrThrow<{ data: string; owner: string; lamports: string }>(
      res,
      "getAccount",
    );
    return {
      data: base64ToBytes(dto.data),
      owner: new Address(dto.owner),
      lamports: BigInt(dto.lamports),
    };
  }

  /** `GET /api/blockhash` → a recent blockhash (base58) to stamp a tx. */
  async getBlockhash(): Promise<string> {
    const res = await fetch(`${this.base}/blockhash`);
    const dto = await jsonOrThrow<{ blockhash: string }>(res, "getBlockhash");
    return dto.blockhash;
  }

  /**
   * `POST /api/transaction` { tx } → the relayed signature. Throws
   * {@link IndexerTxError} (message + logs) on a 4xx rejection so the write layer
   * surfaces the program error.
   */
  async sendTransaction(txBase64: string): Promise<string> {
    const res = await fetch(`${this.base}/transaction`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ tx: txBase64 }),
    });
    if (!res.ok) {
      let error = res.statusText;
      let logs: string[] | undefined;
      try {
        const body = (await res.json()) as { error?: string; logs?: string[] };
        if (body?.error) error = body.error;
        if (Array.isArray(body?.logs)) logs = body.logs;
      } catch {
        // keep the status text
      }
      throw new IndexerTxError(error, logs);
    }
    const dto = (await res.json()) as { signature: string };
    return dto.signature;
  }

  /** `GET /api/transaction/{sig}` → the confirmation status. */
  async getSignatureStatus(signature: string): Promise<SignatureStatus> {
    const res = await fetch(`${this.base}/transaction/${signature}`);
    return jsonOrThrow<SignatureStatus>(res, "getSignatureStatus");
  }
}

/** A default process-wide client (same-origin `/api`) for non-React call sites. */
export const indexer = new IndexerClient();

/**
 * React context carrying the app's single {@link IndexerClient}. The
 * {@link IndexerProvider} component (in `IndexerProvider.tsx`) supplies it; the
 * context + hook live here (a non-component module) so fast-refresh stays happy.
 */
export const IndexerContext = createContext<IndexerClient | null>(null);

/** The app's {@link IndexerClient}. Throws outside an `IndexerProvider`. */
export function useIndexer(): IndexerClient {
  const ctx = useContext(IndexerContext);
  if (!ctx) throw new Error("useIndexer must be used within an IndexerProvider");
  return ctx;
}
