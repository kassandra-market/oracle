/**
 * Shared fixtures + byte helpers for the futarchy/Squads builder tests
 * (split out of futarchy.test.ts — see futarchy.test.ts / futarchy-squads.test.ts).
 */
import { Address } from "@solana/web3.js";

import { futarchy } from "../../src/index.js";

export const {
  DISC,
  ACCOUNT_DISC,
  Market,
  SwapType,
  FUTARCHY_ID,
  CONDITIONAL_VAULT_ID,
  SQUADS_V4_ID,
  SQUADS_PERMISSIONLESS_MEMBER,
  METADAO_ADMIN,
  METADAO_MULTISIG_VAULT,
  METEORA_DAMM_V2_ID,
  DAMM_V2_POOL_AUTHORITY,
  ATA_PROGRAM_ID,
  collectMeteoraDammFees,
  pda,
} = futarchy;

export const SYSTEM_ID = "11111111111111111111111111111111";
export const TOKEN_ID = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";

// Deterministic valid base58 stand-ins.
export const PAYER = "rqRMW2HFJsi1FE1jb8Rvaz4Qz3xHzNkZDb8am1pqEHE";
export const DAO_CREATOR = "84yVtdReAJ8GiR7Erqj7jyxoJurYWzQ6n9eaBGYBDNqM";
export const KASS_MINT = "So11111111111111111111111111111111111111112";
export const USDC_MINT = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
export const TREASURY = "7bQEwuq9ybNyjjFcbtHBfDPxdH3TuGAsZKVRZdihVN4d";
export const ADMIN = "7WCvk98KGRqi2o8D7EWTGrZQuFtikidP8A2D7CDVXwWJ";
export const SOME = "GuBhyNi5GFo9K5YXGKfPMDryWK8GwS5oXe9CJGrzo2sk";

export const enc = new TextEncoder();
export const hex = (b: Uint8Array) => Buffer.from(b).toString("hex");
export const u64 = (v: bigint) => {
  const o = new Uint8Array(8);
  new DataView(o.buffer).setBigUint64(0, v, true);
  return o;
};
export const u32 = (v: number) => {
  const o = new Uint8Array(4);
  new DataView(o.buffer).setUint32(0, v, true);
  return o;
};
export const u16 = (v: number) => {
  const o = new Uint8Array(2);
  new DataView(o.buffer).setUint16(0, v, true);
  return o;
};
export const u128 = (v: bigint) => {
  const o = new Uint8Array(16);
  const dv = new DataView(o.buffer);
  dv.setBigUint64(0, v & 0xffffffffffffffffn, true);
  dv.setBigUint64(8, v >> 64n, true);
  return o;
};
export const cat = (...ps: Uint8Array[]) => {
  const out = new Uint8Array(ps.reduce((n, p) => n + p.length, 0));
  let o = 0;
  for (const p of ps) {
    out.set(p, o);
    o += p.length;
  }
  return out;
};
export const ata = async (owner: string | Address, mint: string | Address) =>
  (
    await Address.findProgramAddress(
      [new Address(owner as string).toBytes(), new Address("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA").toBytes(), new Address(mint as string).toBytes()],
      ATA_PROGRAM_ID,
    )
  )[0];
