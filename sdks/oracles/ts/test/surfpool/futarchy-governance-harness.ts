/**
 * Shared surfpool harness for the T-G3 FULL FUTARCHY GOVERNANCE E2E suite:
 * module-level consts, the fixture interfaces, and the low-level on-chain
 * primitives (tx send, account fetch/fabricate, conditional-vault init, the
 * Squads compact-message encoder, and the kass_price reader) reused verbatim by
 * `futarchy-governance-e2e.test.ts` and `futarchy-governance2-e2e.test.ts`.
 */
import type { AccountMeta } from "@solana/web3.js";
import {
  Address,
  ComputeBudgetProgram,
  Keypair,
  Transaction,
  TransactionInstruction,
} from "@solana/web3.js";

import type { Protocol } from "../../src/accounts/index.js";
import { KASSANDRA_PROGRAM_ID, TOKEN_PROGRAM_ID } from "../../src/constants.js";
import { kassPrice, type SetConfigParams } from "../../src/instructions/index.js";
import * as futarchy from "../../src/futarchy/index.js";

import type { SurfpoolHarness } from "./harness.js";
import { toHex, tokenAccountBytes } from "./harness.js";

export const ATA_PROGRAM_ID = new Address("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");
export const SYSTEM_PROGRAM_ID = new Address("11111111111111111111111111111111");

/**
 * MetaDAO's PUBLIC "permissionless" proposer keypair (futarchy
 * `sdk/permissionless-account.json` → EP3SoC2…), a fixed multisig member with
 * Initiate|Execute. Its secret is published by design so anyone can stage +
 * execute futarchy-DAO Squads transactions. Used here to create + execute the
 * VaultTransaction (the Dao member can only Vote/Execute, not Initiate).
 */
export const PERMISSIONLESS_SECRET = Uint8Array.from([
  249, 158, 188, 171, 243, 143, 1, 48, 87, 243, 209, 153, 144, 106, 23, 88, 161, 209, 65, 217,
  199, 121, 0, 250, 3, 203, 133, 138, 141, 112, 243, 38, 198, 205, 120, 222, 160, 224, 151, 190,
  84, 254, 127, 178, 224, 195, 130, 243, 145, 73, 20, 91, 9, 69, 222, 184, 23, 1, 2, 196, 202,
  206, 153, 192,
]);

/** A sentinel `total_supply_cap` the futarchy verdict must drive on-chain via Squads. */
export const SENTINEL_SUPPLY_CAP = 424_242_424_242n;

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
// helpers
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

export function w(pubkey: Address, isSigner = false): AccountMeta {
  return { pubkey, isSigner, isWritable: true };
}
export function ro(pubkey: Address, isSigner = false): AccountMeta {
  return { pubkey, isSigner, isWritable: false };
}
export function readI64(data: Uint8Array, off: number): bigint {
  return new DataView(data.buffer, data.byteOffset, data.length).getBigInt64(off, true);
}

export async function ata(owner: Address, mint: Address): Promise<Address> {
  return (
    await Address.findProgramAddress([owner.toBytes(), TOKEN_PROGRAM_ID.toBytes(), mint.toBytes()], ATA_PROGRAM_ID)
  )[0];
}

/** Materialise an SPL token account (owner=`owner`) on `mint` with `amount`. */
export async function fabricateToken(f: Fixture, mint: Address, owner: Address, amount: bigint): Promise<Address> {
  const acct = await Keypair.generate();
  await f.harness.setAccount(acct.publicKey.toString(), {
    lamports: 5_000_000,
    owner: TOKEN_PROGRAM_ID.toString(),
    executable: false,
    data: toHex(tokenAccountBytes(mint.toBytes(), owner.toBytes(), amount)),
  });
  return acct.publicKey;
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

/** Materialise a 392-byte Kassandra-owned Oracle in Phase::InvalidDeadend(8). */
export async function fabricateDeadendOracle(f: Fixture, optionsCount: number): Promise<Address> {
  const data = new Uint8Array(392);
  data[0] = 1; // AccountType::Oracle
  data[160] = optionsCount; // options_count
  data[161] = 8; // phase = InvalidDeadend
  data[197] = 0xff; // resolved_option sentinel
  const acct = await Keypair.generate();
  await f.harness.setAccount(acct.publicKey.toString(), {
    lamports: 5_000_000,
    owner: KASSANDRA_PROGRAM_ID.toString(),
    executable: false,
    data: toHex(data),
  });
  return acct.publicKey;
}

/** Read the futarchy spot TWAP (u128 LE return data) via a simulated kass_price tx. */
export async function readKassPrice(f: Fixture, dao: Address): Promise<bigint> {
  const conn = f.harness.connection;
  const tx = new Transaction();
  tx.feePayer = f.payer.publicKey;
  tx.recentBlockhash = (await conn.getLatestBlockhash()).blockhash;
  tx.add(await kassPrice({ kassDao: dao }));
  await tx.sign(f.payer);
  const b64 = Buffer.from(await tx.serialize()).toString("base64");
  const res = await f.harness.rpc<{
    value: { err: unknown; returnData: { data: [string, string] } | null };
  }>("simulateTransaction", [b64, { encoding: "base64", commitment: "confirmed" }]);
  if (res.value.err) throw new Error(`kass_price sim failed: ${JSON.stringify(res.value.err)}`);
  const rd = res.value.returnData?.data?.[0];
  if (!rd) throw new Error("kass_price returned no data");
  const bytes = Buffer.from(rd, "base64");
  let v = 0n;
  for (let i = bytes.length - 1; i >= 0; i--) v = (v << 8n) | BigInt(bytes[i]);
  return v;
}

export function paramsFromProtocol(p: Protocol): SetConfigParams {
  return {
    emissionNum: p.emissionNum,
    emissionDen: p.emissionDen,
    totalSupplyCap: p.totalSupplyCap,
    feeEmaHalflife: p.feeEmaHalflife,
    feePerEmaUnit: p.feePerEmaUnit,
    feeEmaIncrement: p.feeEmaIncrement,
    thresholdNum: p.thresholdNum,
    thresholdDen: p.thresholdDen,
    marketThresholdNum: p.marketThresholdNum,
    marketThresholdDen: p.marketThresholdDen,
    flipSlashNum: p.flipSlashNum,
    flipSlashDen: p.flipSlashDen,
    phaseWindow: p.phaseWindow,
    proposalWindow: p.proposalWindow,
    factVoteSlashNum: p.factVoteSlashNum,
    factVoteSlashDen: p.factVoteSlashDen,
    rewardProposerWeight: p.rewardProposerWeight,
    rewardFactWeight: p.rewardFactWeight,
    challengeFailUsdcFeeNum: p.challengeFailUsdcFeeNum,
    challengeFailUsdcFeeDen: p.challengeFailUsdcFeeDen,
    challengeSuccessKassFeeNum: p.challengeSuccessKassFeeNum,
    challengeSuccessKassFeeDen: p.challengeSuccessKassFeeDen,
    stakeFloorEmaThreshold: p.stakeFloorEmaThreshold,
    stakeFloorEmaCap: p.stakeFloorEmaCap,
    stakeFloorMax: p.stakeFloorMax,
  };
}

export interface SquadsCompiledIx {
  programIdIndex: number;
  accountIndexes: number[];
  data: Uint8Array;
}

/**
 * Encode a Squads v4 compact `TransactionMessage` (see the recon spec):
 *   num_signers u8, num_writable_signers u8, num_writable_non_signers u8,
 *   account_keys SmallVec<u8, Pubkey>, instructions SmallVec<u8, CompiledIx>,
 *   address_table_lookups SmallVec<u8, _> (empty here).
 * CompiledIx: program_id_index u8, account_indexes SmallVec<u8,u8>,
 *   data SmallVec<u16, u8> (u16 length prefix — the one wide field).
 */
export function buildSquadsMessage(m: {
  accountKeys: Address[];
  numSigners: number;
  numWritableSigners: number;
  numWritableNonSigners: number;
  instructions: SquadsCompiledIx[];
}): Uint8Array {
  const parts: number[] = [m.numSigners, m.numWritableSigners, m.numWritableNonSigners];
  parts.push(m.accountKeys.length & 0xff);
  const out: number[] = [...parts];
  for (const k of m.accountKeys) out.push(...k.toBytes());
  out.push(m.instructions.length & 0xff);
  for (const ix of m.instructions) {
    out.push(ix.programIdIndex & 0xff);
    out.push(ix.accountIndexes.length & 0xff);
    out.push(...ix.accountIndexes.map((i) => i & 0xff));
    out.push(ix.data.length & 0xff, (ix.data.length >> 8) & 0xff); // u16 LE
    out.push(...ix.data);
  }
  out.push(0); // address_table_lookups: empty
  return Uint8Array.from(out);
}
