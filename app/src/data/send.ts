/**
 * WF1 — the tx SEND-AND-CONFIRM seam (pure, NO React).
 *
 * A {@link TxSender} abstracts "sign + submit a legacy tx built from these
 * instructions, return the signature". Both a wallet and a keypair satisfy it:
 *   - the UI (WF2) backs it with wallet-adapter's
 *     `sendTransaction(new Transaction().add(...ixs), connection)`;
 *   - tests back it with {@link keypairSender} (a funded {@link Keypair}).
 *
 * {@link sendAndConfirm} calls the sender then confirms the signature (reusing
 * the SDK's `confirmSignature` — a `getSignatureStatuses` poll), surfacing a
 * failed send / failed-to-confirm / expired tx as a typed {@link SendError}
 * (carrying the signature + any program logs) the caller can render.
 */
import { Connection, Keypair, Transaction, type TransactionInstruction } from "@solana/web3.js";
import { confirmSignature } from "@kassandra-market/oracles";

/**
 * The signer-abstraction seam: given the instruction list, sign + submit a
 * legacy transaction and resolve to its base58 signature. The UI supplies a
 * wallet-backed sender; tests supply {@link keypairSender}.
 */
export type TxSender = (ixs: TransactionInstruction[]) => Promise<string>;

/** The successful outcome of {@link sendAndConfirm}. */
export interface SendResult {
  /** The confirmed transaction signature (base58). */
  signature: string;
}

/** A typed failure of the send/confirm path — carries the signature + program logs when known. */
export class SendError extends Error {
  /** The tx signature, if the send succeeded but confirmation failed. */
  readonly signature?: string;
  /** Program logs pulled off the underlying send/simulate error, if any. */
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

/** Best-effort extraction of program logs off a web3.js send/simulate error. */
function extractLogs(e: unknown): string[] | undefined {
  const logs = (e as { logs?: unknown } | null)?.logs;
  return Array.isArray(logs) ? (logs as string[]) : undefined;
}

/**
 * Send `ixs` via `sender`, then confirm the resulting signature over
 * `connection`. Throws a {@link SendError} (with the signature + logs when
 * available) if the send throws or the tx fails / never confirms.
 */
export async function sendAndConfirm(
  connection: Connection,
  sender: TxSender,
  ixs: TransactionInstruction[],
): Promise<SendResult> {
  let signature: string;
  try {
    signature = await sender(ixs);
  } catch (e) {
    throw new SendError(`Transaction send failed: ${errMsg(e)}`, {
      logs: extractLogs(e),
      cause: e,
    });
  }
  try {
    await confirmSignature(connection, signature);
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
 * {@link Transaction} with a fresh blockhash, sets `keypair` as the fee payer,
 * signs with it, and `sendRawTransaction`s it (preflight on). Returns the
 * signature — {@link sendAndConfirm} then confirms it.
 *
 * NOTE: the UI does NOT use this — WF2 supplies its own wallet-adapter-backed
 * sender (`(ixs) => sendTransaction(new Transaction().add(...ixs), connection)`).
 */
export function keypairSender(connection: Connection, keypair: Keypair): TxSender {
  return async (ixs) => {
    const tx = new Transaction();
    tx.feePayer = keypair.publicKey;
    tx.recentBlockhash = (await connection.getLatestBlockhash()).blockhash;
    for (const ix of ixs) tx.add(ix);
    await tx.sign(keypair);
    return connection.sendRawTransaction(await tx.serialize(), { skipPreflight: false });
  };
}
