/**
 * The tx SEND-AND-CONFIRM seam (pure, NO React) — routed through the indexer.
 *
 * The app has NO Solana `Connection`: a transaction is built from instructions,
 * stamped with a blockhash from the indexer, SIGNED locally (wallet or keypair),
 * and RELAYED to the indexer (`POST /api/transaction`), which submits it to the
 * RPC it fronts. {@link sendAndConfirm} then confirms the signature by polling the
 * indexer (`GET /api/transaction/{sig}`).
 *
 * A {@link TxSender} abstracts "given these instructions, sign + relay, return the
 * signature". Both a wallet and a keypair satisfy it via {@link signAndRelay}:
 *   - the UI backs it with wallet-adapter's `signTransaction` + the indexer relay;
 *   - tests/CLIs back it with {@link keypairSender} (a funded {@link Keypair}).
 *
 * A failed relay / failed-to-confirm / expired tx surfaces as a typed
 * {@link SendError} (carrying the signature + any program logs) the caller renders.
 */
import { Address, Keypair, Transaction, type TransactionInstruction } from "@solana/web3.js";
import { MARKET_PROGRAM_ID, MARKET_ERROR_MESSAGES, decodeError } from "@kassandra-market/markets";
import { IndexerTxError, type IndexerClient } from "../lib/indexer";

/** Signs a prepared (feePayer + blockhash set) legacy {@link Transaction} in place. */
export type SignTransaction = (tx: Transaction) => Promise<Transaction>;

/**
 * The signer-abstraction seam: given the instruction list, sign + relay a legacy
 * transaction through the indexer and resolve to its base58 signature. The UI
 * supplies a wallet-backed sender; tests supply {@link keypairSender}.
 */
export type TxSender = (ixs: TransactionInstruction[]) => Promise<string>;

/** The successful outcome of {@link sendAndConfirm}. */
export interface SendResult {
  /** The confirmed transaction signature (base58). */
  signature: string;
}

/** A typed failure of the send/confirm path — carries the signature + program logs when known. */
export class SendError extends Error {
  /** The tx signature, if the relay succeeded but confirmation failed. */
  readonly signature?: string;
  /** Program logs pulled off the underlying relay error, if any. */
  readonly logs?: string[];
  /** The underlying error thrown by the sender / confirm. */
  readonly cause?: unknown;
  constructor(message: string, opts?: { signature?: string; logs?: string[]; cause?: unknown }) {
    super(message);
    this.name = "SendError";
    this.signature = opts?.signature;
    this.logs = opts?.logs;
    this.cause = opts?.cause;
  }
}

function errMsg(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/** Best-effort extraction of program logs off a relay/send error. */
function extractLogs(e: unknown): string[] | undefined {
  const logs = (e as { logs?: unknown } | null)?.logs;
  return Array.isArray(logs) ? (logs as string[]) : undefined;
}

/** Base64-encode raw tx bytes for the `POST /api/transaction` body. */
function bytesToBase64(bytes: Uint8Array): string {
  let bin = "";
  for (let i = 0; i < bytes.length; i++) bin += String.fromCharCode(bytes[i]);
  return btoa(bin);
}

/**
 * Turn a kassandra-market `Custom(<code>)` program error into its human message
 * (from `MARKET_ERROR_MESSAGES`), or `undefined` when the failure isn't one of
 * OUR program's errors (a MetaDAO / SPL / other-program `Custom` code won't
 * decode — the caller falls back to the raw text).
 *
 * To avoid mis-attributing another program's `Custom(N)` (codes 0..=16 collide
 * with ours), it PREFERS a program-scoped log line — `Program <MARKET_PROGRAM_ID>
 * failed: custom program error: 0xN` — and only falls back to a bare `"Custom":N`
 * from the status-error JSON (the confirm path, where logs are absent).
 */
export function humanizeProgramError(text: string, logs?: string[]): string | undefined {
  const market = MARKET_PROGRAM_ID.toString();
  let code: number | undefined;

  const scoped = (logs ?? []).find(
    (l) => l.includes(market) && /custom program error:\s*0x[0-9a-fA-F]+/i.test(l),
  );
  if (scoped) {
    const h = /custom program error:\s*0x([0-9a-fA-F]+)/i.exec(scoped);
    if (h) code = parseInt(h[1], 16);
  }
  if (code === undefined) {
    const j = /"Custom"\s*:\s*(\d+)/.exec(text);
    if (j) code = Number(j[1]);
  }
  if (code === undefined) return undefined;

  const err = decodeError(code);
  return err === null ? undefined : MARKET_ERROR_MESSAGES[err];
}

const sleep = (ms: number): Promise<void> => new Promise((r) => setTimeout(r, ms));

/**
 * Build a legacy transaction from `ixs`, stamp it with `feePayer` + a fresh
 * indexer blockhash, sign it via `signTransaction`, then RELAY it to the indexer
 * and resolve to the signature. The shared core of every send path (wallet +
 * keypair).
 */
export async function signAndRelay(
  indexer: IndexerClient,
  feePayer: Address,
  signTransaction: SignTransaction,
  ixs: TransactionInstruction[],
): Promise<string> {
  const tx = new Transaction();
  for (const ix of ixs) tx.add(ix);
  tx.feePayer = feePayer;
  // The indexer returns a base58 blockhash string; brand it to the tx field type.
  tx.recentBlockhash = (await indexer.getBlockhash()) as Transaction["recentBlockhash"];
  const signed = await signTransaction(tx);
  return indexer.sendTransaction(bytesToBase64(await signed.serialize()));
}

/**
 * Poll the indexer (`GET /api/transaction/{sig}`) until the tx is
 * confirmed/finalized (throws on tx error or timeout). 150ms poll cadence, 30s
 * deadline — mirrors the old `getSignatureStatuses` loop against the RPC.
 */
export async function confirmSignature(
  indexer: IndexerClient,
  signature: string,
  timeoutMs = 30_000,
): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const { status, err } = await indexer.getSignatureStatus(signature);
    if (status === "failed" || err) {
      const raw = err ?? "unknown error";
      const human = humanizeProgramError(raw);
      throw new Error(`tx ${signature} failed: ${human ?? raw}`);
    }
    if (status === "confirmed" || status === "finalized") return;
    await sleep(150);
  }
  throw new Error(`tx ${signature} not confirmed within ${timeoutMs}ms`);
}

/**
 * Send `ixs` via `sender` (sign + relay), then confirm the resulting signature
 * against the indexer. Throws a {@link SendError} (with the signature + logs when
 * available) if the relay throws or the tx fails / never confirms.
 */
export async function sendAndConfirm(
  indexer: IndexerClient,
  sender: TxSender,
  ixs: TransactionInstruction[],
): Promise<SendResult> {
  let signature: string;
  try {
    signature = await sender(ixs);
  } catch (e) {
    const logs = e instanceof IndexerTxError ? e.logs : extractLogs(e);
    const human = humanizeProgramError(errMsg(e), logs);
    throw new SendError(human ? `Transaction failed: ${human}` : `Transaction send failed: ${errMsg(e)}`, {
      logs,
      cause: e,
    });
  }
  try {
    await confirmSignature(indexer, signature);
  } catch (e) {
    throw new SendError(`Transaction ${signature} failed to confirm: ${errMsg(e)}`, {
      signature,
      logs: extractLogs(e),
      cause: e,
    });
  }
  return { signature };
}

/**
 * A keypair-backed {@link TxSender} for tests/CLIs: builds a LEGACY
 * {@link Transaction}, stamps a fresh indexer blockhash, sets `keypair` as the fee
 * payer, signs with it, and RELAYS it through the indexer. Returns the signature —
 * {@link sendAndConfirm} then confirms it.
 *
 * NOTE: the UI does NOT use this — the wallet supplies its own `signTransaction`
 * to {@link signAndRelay}.
 */
export function keypairSender(indexer: IndexerClient, keypair: Keypair): TxSender {
  const sign: SignTransaction = async (tx) => {
    await tx.sign(keypair);
    return tx;
  };
  return (ixs) => signAndRelay(indexer, keypair.publicKey, sign, ixs);
}
