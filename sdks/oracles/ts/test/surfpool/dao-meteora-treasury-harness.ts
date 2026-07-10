/**
 * D1 surfpool DAO-OWNED METEORA TREASURY-FEE CLAIM E2E — shared primitives
 * (consts, interfaces, and low-level drivers). Extracted from
 * `dao-meteora-treasury-e2e.test.ts` so no single file exceeds ~400 lines.
 * Pure move: bodies are verbatim.
 */
import type { AccountMeta } from "@solana/web3.js";
import {
  Address,
  ComputeBudgetProgram,
  Keypair,
  Transaction,
  TransactionInstruction,
} from "@solana/web3.js";
import { expect } from "vitest";

import { TOKEN_PROGRAM_ID } from "../../src/constants.js";
import * as futarchy from "../../src/futarchy/index.js";

import {
  SurfpoolHarness,
  surfpoolReady,
  tokenAccountAmount,
  tokenAccountBytes,
  toHex,
} from "./harness.js";

export const ENABLED = process.env.KASSANDRA_E2E === "1" && surfpoolReady();

export const MAINNET_RPC = "https://api.mainnet-beta.solana.com";

export const ATA_PROGRAM_ID = new Address("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");

/** A REAL public + static mainnet cp-amm Config (index 0): `pool_creator_authority
 * == Pubkey::default` (permissionless). Cloned onto the fork so `initialize_pool`
 * accepts our arbitrary payer as pool creator (same as M2). */
export const REAL_CONFIG = new Address("8CNy9goNQNLM4wtgRw528tUQGMKD3vSuFRZY2gLGLLvF");

/** Full-range price bounds baked into the public configs (Q64.64). */
export const SQRT_MIN = 4295048016n;
export const SQRT_PRICE_INIT = 1n << 64n; // price 1.0
export const INIT_LIQUIDITY = 1_000_000_000n * (1n << 64n); // vault position (the main LP)
export const PROBE_LIQUIDITY = INIT_LIQUIDITY / 2n; // payer probe position
export const U64_MAX = (1n << 64n) - 1n;

/**
 * MetaDAO's PUBLIC "permissionless" member keypair (futarchy
 * `sdk/permissionless-account.json` → EP3SoC2…), the multisig's only Initiate
 * member. Its secret is published by design; it (not the Dao) creates + executes
 * the Squads VaultTransaction (mirrors G3).
 */
export const PERMISSIONLESS_SECRET = Uint8Array.from([
  249, 158, 188, 171, 243, 143, 1, 48, 87, 243, 209, 153, 144, 106, 23, 88, 161, 209, 65, 217,
  199, 121, 0, 250, 3, 203, 133, 138, 141, 112, 243, 38, 198, 205, 120, 222, 160, 224, 151, 190,
  84, 254, 127, 178, 224, 195, 130, 243, 145, 73, 20, 91, 9, 69, 222, 184, 23, 1, 2, 196, 202,
  206, 153, 192,
]);

export interface Fixture {
  harness: SurfpoolHarness;
  payer: Keypair;
  kassMint: Keypair;
  usdcMint: Keypair;
  dao: Address;
  multisig: Address;
  vault: Address;
}

// ---------------------------------------------------------------------------
// shared Meteora state (set by the ownership flow, read by the claim flow)
// ---------------------------------------------------------------------------
export interface MeteoraState {
  poolAddr: Address;
  tokenAVault: Address;
  tokenBVault: Address;
  vaultPos: Address;
  vaultPosNftMint: Address;
  vaultPosNftAccount: Address;
}

// ---------------------------------------------------------------------------
// no-MetaDAO-admin/vault guard
// ---------------------------------------------------------------------------
export function assertNoMetaDao(keys: Address[]): void {
  const banned = new Set([
    futarchy.METADAO_ADMIN.toString(), // tSTp6B6k…
    futarchy.METADAO_MULTISIG_VAULT.toString(), // 6awyHMsh…
  ]);
  for (const k of keys) {
    expect(banned.has(k.toString()), `MetaDAO admin/vault ${k} must not appear in the DAO-owned claim flow`).toBe(
      false,
    );
  }
}

// ---------------------------------------------------------------------------
// Squads compact TransactionMessage compiler (generic, from a web3 ix)
// ---------------------------------------------------------------------------
export interface SquadsMessage {
  message: Uint8Array;
  /** account_keys in message order, as vault_transaction_execute remaining accounts
   * (writability mirrors the message; NONE marked signer — Squads signs the vault). */
  remainingAccounts: AccountMeta[];
}

/**
 * Compile a single web3 `TransactionInstruction` into Squads v4's compact
 * `TransactionMessage`. `vaultSigner` is the DAO vault PDA that Squads
 * `invoke_signed`s (it must be the message's sole signer, readonly). Keys are
 * deduped and ordered [w-signers, ro-signers, w-non-signers, ro-non-signers] per
 * the Squads message format (see NOTES.md "G3 ADDENDUM").
 */
export function compileSquadsMessage(ix: TransactionInstruction, vaultSigner: Address): SquadsMessage {
  interface Role {
    pubkey: Address;
    isSigner: boolean;
    isWritable: boolean;
  }
  const roles = new Map<string, Role>();
  const note = (pubkey: Address, isSigner: boolean, isWritable: boolean) => {
    const k = pubkey.toString();
    const prev = roles.get(k);
    if (prev) {
      prev.isSigner ||= isSigner;
      prev.isWritable ||= isWritable;
    } else {
      roles.set(k, { pubkey, isSigner, isWritable });
    }
  };
  for (const meta of ix.keys) note(meta.pubkey, meta.isSigner, meta.isWritable);
  note(ix.programId, false, false); // the inner program must be an account_key
  // the vault is the message signer (Squads signs for it); it is readonly here.
  note(vaultSigner, true, false);

  const all = [...roles.values()];
  const rank = (r: Role) =>
    r.isSigner && r.isWritable ? 0 : r.isSigner ? 1 : r.isWritable ? 2 : 3;
  all.sort((a, b) => rank(a) - rank(b));

  const numSigners = all.filter((r) => r.isSigner).length;
  const numWritableSigners = all.filter((r) => r.isSigner && r.isWritable).length;
  const numWritableNonSigners = all.filter((r) => !r.isSigner && r.isWritable).length;

  const indexOf = (pk: Address) => all.findIndex((r) => r.pubkey.toString() === pk.toString());
  const compiled = {
    programIdIndex: indexOf(ix.programId),
    accountIndexes: ix.keys.map((meta) => indexOf(meta.pubkey)),
    data: new Uint8Array(ix.data),
  };

  const out: number[] = [numSigners, numWritableSigners, numWritableNonSigners];
  out.push(all.length & 0xff);
  for (const r of all) out.push(...r.pubkey.toBytes());
  out.push(1); // one instruction
  out.push(compiled.programIdIndex & 0xff);
  out.push(compiled.accountIndexes.length & 0xff);
  out.push(...compiled.accountIndexes.map((i) => i & 0xff));
  out.push(compiled.data.length & 0xff, (compiled.data.length >> 8) & 0xff); // u16 LE
  out.push(...compiled.data);
  out.push(0); // address_table_lookups: empty

  const remainingAccounts: AccountMeta[] = all.map((r) => ({
    pubkey: r.pubkey,
    isSigner: false,
    isWritable: r.isWritable,
  }));
  return { message: Uint8Array.from(out), remainingAccounts };
}

// ---------------------------------------------------------------------------
// helpers (mirrors G3 + M2)
// ---------------------------------------------------------------------------

export async function sendIx(
  f: Fixture,
  ix: TransactionInstruction,
  signers: Keypair[] = [],
  computeUnits?: number,
): Promise<void> {
  const conn = f.harness.connection;
  const tx = new Transaction();
  tx.feePayer = f.payer.publicKey;
  tx.recentBlockhash = (await conn.getLatestBlockhash()).blockhash;
  if (computeUnits) tx.add(ComputeBudgetProgram.setComputeUnitLimit({ units: computeUnits }));
  tx.add(ix);
  await tx.sign(f.payer, ...signers);
  const sig = await conn.sendRawTransaction(await tx.serialize(), { skipPreflight: false });
  await f.harness.confirmSignature(sig);
}

export async function fetchAccount(f: Fixture, address: Address, timeoutMs = 20_000): Promise<Uint8Array> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const info = await f.harness.connection.getAccountInfo(address);
    if (info && info.data.length > 0) return info.data;
    await new Promise((r) => setTimeout(r, 150));
  }
  throw new Error(`account ${address} did not appear within ${timeoutMs}ms`);
}

export async function tokenBalance(f: Fixture, address: Address): Promise<bigint> {
  return tokenAccountAmount(await fetchAccount(f, address));
}

export function readI64(data: Uint8Array, off: number): bigint {
  return new DataView(data.buffer, data.byteOffset, data.length).getBigInt64(off, true);
}

export async function ata(owner: Address, mint: Address): Promise<Address> {
  return (
    await Address.findProgramAddress([owner.toBytes(), TOKEN_PROGRAM_ID.toBytes(), mint.toBytes()], ATA_PROGRAM_ID)
  )[0];
}

/** Materialise an SPL token account (fresh keypair address) with `amount`. */
export async function fabricateToken(f: Fixture, mint: Address, owner: Address, amount: bigint): Promise<Address> {
  const acct = await Keypair.generate();
  await fabricateTokenAt(f, acct.publicKey, mint, owner, amount);
  return acct.publicKey;
}

/** Materialise an SPL token account at a SPECIFIC address (e.g. an ATA). */
export async function fabricateTokenAt(
  f: Fixture,
  address: Address,
  mint: Address,
  owner: Address,
  amount: bigint,
): Promise<void> {
  await f.harness.setAccount(address.toString(), {
    lamports: 5_000_000,
    owner: TOKEN_PROGRAM_ID.toString(),
    executable: false,
    data: toHex(tokenAccountBytes(mint.toBytes(), owner.toBytes(), amount)),
  });
}

export interface CondVault {
  vault: Address;
  underlying: Address;
  passMint: Address; // conditional_token_mints[1]
  failMint: Address; // conditional_token_mints[0]
}

/** Real conditional_vault init (vault + 2 conditional mints) for `underlyingMint`. */
export async function condVault(f: Fixture, question: Address, underlyingMint: Address): Promise<CondVault> {
  const vault = (await futarchy.pda.conditionalVault(question, underlyingMint)).address;
  const failMint = (await futarchy.pda.conditionalTokenMint(vault, 0)).address;
  const passMint = (await futarchy.pda.conditionalTokenMint(vault, 1)).address;
  const underlying = await ata(vault, underlyingMint);
  await sendIx(
    f,
    await futarchy.initializeConditionalVault({ question, underlyingMint, payer: f.payer.publicKey, numOutcomes: 2 }),
    [],
    400_000,
  );
  return { vault, underlying, passMint, failMint };
}

/** Fetch an account's raw data straight from mainnet (NOT the fork). */
export async function fetchMainnetAccount(address: Address): Promise<Uint8Array> {
  const res = await fetch(MAINNET_RPC, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      jsonrpc: "2.0",
      id: 1,
      method: "getAccountInfo",
      params: [address.toString(), { encoding: "base64" }],
    }),
  });
  const json = (await res.json()) as {
    result?: { value?: { data: [string, string] } | null };
    error?: { message: string };
  };
  if (json.error) throw new Error(`mainnet getAccountInfo failed: ${json.error.message}`);
  const b64 = json.result?.value?.data?.[0];
  if (!b64) throw new Error(`mainnet account ${address} not found`);
  return new Uint8Array(Buffer.from(b64, "base64"));
}
