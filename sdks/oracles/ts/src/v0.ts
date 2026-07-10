/**
 * I2 — v0 (versioned) transaction + Address Lookup Table path.
 *
 * The near-cap finalizes (`finalizeProposals` / `finalizeOracle`) thread the
 * FULL proposer set (up to `MAX_PROPOSERS = 60`) as account metas. At ~28
 * proposers a LEGACY transaction's compiled message already exceeds the
 * 1232-byte packet (`PACKET_DATA_SIZE`) — each key is 32 bytes and the legacy
 * message inlines every one. This module removes that limit by packing the
 * proposer PDAs into an Address Lookup Table (ALT) and sending the finalize as a
 * v0 transaction: the read-only proposer keys are then referenced by a 1-byte
 * table index instead of a 32-byte inline key.
 *
 * All symbols are the CLASSIC `@solana/web3.js@3.0.0-rc.2` API, confirmed
 * against the installed `.d.ts` (see `NOTES-api.md`):
 *   - `AddressLookupTableProgram.createLookupTable(params)`
 *       → `Promise<[TransactionInstruction, Address]>`  (ASYNC)
 *   - `AddressLookupTableProgram.extendLookupTable(params)`
 *       → `TransactionInstruction`  (SYNC; each extend must itself fit a tx, so
 *          the address list is CHUNKED)
 *   - `new TransactionMessage({ payerKey, instructions, recentBlockhash })
 *        .compileToV0Message([alt])`  → `MessageV0`
 *   - `new VersionedTransaction(message)` → `.sign(signers)` (ASYNC) →
 *     `.serialize()` (SYNC)
 *   - `connection.getAddressLookupTable(key)`
 *       → `Promise<RpcResponseAndContext<AddressLookupTableAccount | null>>`
 *
 * IMPORTANT: an ALT is only usable in a v0 tx once it is on-chain AND at least
 * one slot has passed since its last extend (the added addresses become active
 * the FOLLOWING slot). That is why ALT setup is inherently 2+ transactions + a
 * slot wait, and why this path is LIVE-CLUSTER / surfpool only — NOT litesvm
 * (which has no ALT resolution / slot progression semantics). See `NOTES-api.md`.
 */
import {
  AddressLookupTableProgram,
  Connection,
  Keypair,
  Transaction,
  TransactionMessage,
  VersionedTransaction,
  type Address,
  type AddressLookupTableAccount,
  type Blockhash,
  type MessageV0,
  type TransactionInstruction,
} from "@solana/web3.js";

/**
 * Addresses added per `extendLookupTable` instruction. The extend ix inlines
 * each 32-byte key in its OWN transaction, so ~30 keeps a single extend well
 * under the 1232-byte packet (30 × 32 = 960 B + overhead).
 */
export const DEFAULT_EXTEND_CHUNK = 30;

/** Poll-to-confirm callback for a sent signature (throws on tx error/timeout). */
export type ConfirmFn = (signature: string) => Promise<void>;

const sleep = (ms: number): Promise<void> => new Promise((r) => setTimeout(r, ms));

/**
 * Default confirmation: poll `getSignatureStatuses` until the tx is
 * confirmed/finalized (throws on tx error or timeout). Callers with their own
 * confirm loop (e.g. the surfpool harness) can pass a {@link ConfirmFn} instead.
 */
export async function confirmSignature(
  connection: Connection,
  signature: string,
  timeoutMs = 30_000,
): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const { value } = await connection.getSignatureStatuses([signature], {
      searchTransactionHistory: true,
    });
    const st = value[0];
    if (st) {
      if (st.err) throw new Error(`tx ${signature} failed: ${JSON.stringify(st.err)}`);
      const cs = st.confirmationStatus;
      if (cs === "confirmed" || cs === "finalized") return;
    }
    await sleep(150);
  }
  throw new Error(`tx ${signature} not confirmed within ${timeoutMs}ms`);
}

/** Build, sign (payer + extra signers), send + confirm a LEGACY tx (ALT setup). */
async function sendLegacy(
  connection: Connection,
  payer: Keypair,
  instructions: TransactionInstruction[],
  signers: Keypair[],
  confirm: ConfirmFn,
): Promise<string> {
  const tx = new Transaction();
  tx.feePayer = payer.publicKey;
  tx.recentBlockhash = (await connection.getLatestBlockhash()).blockhash;
  for (const ix of instructions) tx.add(ix);
  await tx.sign(payer, ...signers);
  const sig = await connection.sendRawTransaction(await tx.serialize(), { skipPreflight: false });
  await confirm(sig);
  return sig;
}

export interface CompileV0Args {
  /** Fee payer (also the sole required signer of a finalize). */
  payer: Address;
  /** The instruction(s) to compile (e.g. the finalize ix, maybe a compute-budget ix). */
  instructions: TransactionInstruction[];
  /** The lookup tables whose keys should be referenced by index. */
  lookupTableAccounts: AddressLookupTableAccount[];
  /** A recent blockhash for the message. */
  recentBlockhash: Blockhash | string;
}

/**
 * Compile a v0 (`MessageV0`) message that references `lookupTableAccounts`.
 * PURE (no network) — the read-only, non-signer, non-program keys present in a
 * supplied ALT are replaced by table lookups. Unit-testable offline.
 */
export function compileV0Message(args: CompileV0Args): MessageV0 {
  return new TransactionMessage({
    payerKey: args.payer,
    recentBlockhash: args.recentBlockhash as Blockhash,
    instructions: args.instructions,
  }).compileToV0Message(args.lookupTableAccounts);
}

export interface CreateProposerAltArgs {
  connection: Connection;
  /** Payer + authority of the new ALT (also signs the setup txs). */
  payer: Keypair;
  /** The addresses to publish into the table (e.g. the oracle's proposer PDAs). */
  addresses: ReadonlyArray<Address>;
  /** Addresses per extend (default {@link DEFAULT_EXTEND_CHUNK}). */
  extendChunkSize?: number;
  /** Confirm callback (default {@link confirmSignature} over `connection`). */
  confirm?: ConfirmFn;
  /** Overall wait for the ALT to become active, ms (default 30000). */
  activateTimeoutMs?: number;
}

/**
 * Create + chunk-extend an Address Lookup Table over `addresses`, then WAIT for
 * it to be on-chain and active (fetchable, all addresses present, and a slot
 * past its last extend). Returns the resolved {@link AddressLookupTableAccount},
 * ready to hand to {@link compileV0Message} / {@link sendV0}.
 *
 * This is 1 create tx + ceil(addresses / chunk) extend txs + a slot wait.
 */
export async function createProposerAlt(
  args: CreateProposerAltArgs,
): Promise<AddressLookupTableAccount> {
  const { connection, payer } = args;
  const chunk = args.extendChunkSize ?? DEFAULT_EXTEND_CHUNK;
  const confirm = args.confirm ?? ((sig) => confirmSignature(connection, sig));

  // `createLookupTable` derives the ALT address from a recent slot; the slot
  // must be < the current slot when the create ix executes (it is, since slots
  // advance between this read and the confirmed create tx).
  const recentSlot = await connection.getSlot();
  const [createIx, lookupTableAddress] = await AddressLookupTableProgram.createLookupTable({
    authority: payer.publicKey,
    payer: payer.publicKey,
    recentSlot,
  });
  await sendLegacy(connection, payer, [createIx], [], confirm);

  for (let i = 0; i < args.addresses.length; i += chunk) {
    const extendIx = AddressLookupTableProgram.extendLookupTable({
      lookupTable: lookupTableAddress,
      authority: payer.publicKey,
      payer: payer.publicKey,
      addresses: [...args.addresses.slice(i, i + chunk)],
    });
    await sendLegacy(connection, payer, [extendIx], [], confirm);
  }

  return waitForActiveLookupTable(
    connection,
    lookupTableAddress,
    args.addresses.length,
    args.activateTimeoutMs ?? 30_000,
  );
}

/**
 * Poll until the ALT is fetchable, holds all `expectedCount` addresses, AND the
 * current slot is strictly past its `lastExtendedSlot` (so the newest addresses
 * are active for lookups). Throws on timeout.
 */
async function waitForActiveLookupTable(
  connection: Connection,
  key: Address,
  expectedCount: number,
  timeoutMs: number,
): Promise<AddressLookupTableAccount> {
  const deadline = Date.now() + timeoutMs;
  let last = "not fetched";
  while (Date.now() < deadline) {
    const { value } = await connection.getAddressLookupTable(key);
    if (value && value.state.addresses.length >= expectedCount) {
      const slot = await connection.getSlot();
      if (value.isActive() && slot > Number(value.state.lastExtendedSlot)) return value;
      last = `slot ${slot} not yet past lastExtendedSlot ${value.state.lastExtendedSlot}`;
    } else {
      last = `have ${value?.state.addresses.length ?? 0}/${expectedCount} addresses`;
    }
    await sleep(200);
  }
  throw new Error(`ALT ${key} did not become active within ${timeoutMs}ms (${last})`);
}

export interface SendV0Args {
  connection: Connection;
  /** Fee payer / signer. */
  payer: Keypair;
  /** Instruction(s) to send — the finalize ix (+ optional compute-budget ix). */
  instructions: TransactionInstruction[];
  /** The ALT(s) over which the keys are looked up. */
  lookupTableAccounts: AddressLookupTableAccount[];
  /** Extra signers beyond the payer. */
  signers?: Keypair[];
  /** Confirm callback (default {@link confirmSignature} over `connection`). */
  confirm?: ConfirmFn;
}

/** Compile a v0 message over the ALT(s), sign, send + confirm. Returns the signature. */
export async function sendV0(args: SendV0Args): Promise<string> {
  const { connection, payer } = args;
  const signers = args.signers ?? [];
  const confirm = args.confirm ?? ((sig) => confirmSignature(connection, sig));

  const message = compileV0Message({
    payer: payer.publicKey,
    instructions: args.instructions,
    lookupTableAccounts: args.lookupTableAccounts,
    recentBlockhash: (await connection.getLatestBlockhash()).blockhash,
  });
  const tx = new VersionedTransaction(message);
  await tx.sign([payer, ...signers]);
  const sig = await connection.sendRawTransaction(tx.serialize(), { skipPreflight: false });
  await confirm(sig);
  return sig;
}

export interface SendFinalizeViaAltArgs {
  connection: Connection;
  /** Fee payer + ALT authority (signs the ALT setup + the finalize). */
  payer: Keypair;
  /** The built finalize instruction (`finalizeProposals` / `finalizeOracle`). */
  instruction: TransactionInstruction;
  /**
   * The addresses to pack into the ALT — the oracle's proposer PDAs (the
   * read-only tail that overflows a legacy tx). Extra keys (mint, vault, ...)
   * are harmless but the read-only proposer set is what matters.
   */
  lookupAddresses: ReadonlyArray<Address>;
  /** Instructions to prepend (e.g. `ComputeBudgetProgram.setComputeUnitLimit`). */
  prependInstructions?: TransactionInstruction[];
  /** Extra signers beyond the payer. */
  signers?: Keypair[];
  /** Confirm callback (default {@link confirmSignature} over `connection`). */
  confirm?: ConfirmFn;
  /** Addresses per extend (default {@link DEFAULT_EXTEND_CHUNK}). */
  extendChunkSize?: number;
}

/**
 * One-shot: publish an ALT over `lookupAddresses`, then send `instruction` as a
 * v0 tx that references it. Returns the finalize signature + the created ALT
 * (reusable for a second finalize over the same proposer set).
 */
export async function sendFinalizeViaAlt(
  args: SendFinalizeViaAltArgs,
): Promise<{ signature: string; lookupTableAccount: AddressLookupTableAccount }> {
  const confirm = args.confirm ?? ((sig: string) => confirmSignature(args.connection, sig));
  const lookupTableAccount = await createProposerAlt({
    connection: args.connection,
    payer: args.payer,
    addresses: args.lookupAddresses,
    extendChunkSize: args.extendChunkSize,
    confirm,
  });
  const signature = await sendV0({
    connection: args.connection,
    payer: args.payer,
    instructions: [...(args.prependInstructions ?? []), args.instruction],
    lookupTableAccounts: [lookupTableAccount],
    signers: args.signers,
    confirm,
  });
  return { signature, lookupTableAccount };
}
