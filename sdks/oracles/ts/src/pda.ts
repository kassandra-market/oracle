/**
 * Program-derived-address (PDA) derivation for the Kassandra program.
 *
 * Every seed list here is the program's PUBLIC CONTRACT, verified against the
 * processors and the test harness `*_pda` helpers in
 * `programs/oracles/tests/common/mod.rs`. Seed-byte encodings:
 *   - literal seeds (`"oracle"`, `"protocol"`, ...) are ASCII byte strings;
 *   - pubkey seeds are the 32 RAW bytes of the address;
 *   - the oracle `nonce` is a **u64 little-endian, 8 bytes**;
 *   - `content_hash` is its 32 raw bytes.
 *
 * web3.js@3.0.0-rc.2 exposes only the ASYNC `Address.findProgramAddress`
 * (there is NO `findProgramAddressSync` in this version), so every derivation
 * here is async and returns `{ address, bump }`.
 */
import { Address } from "@solana/web3.js";

import { u64LE } from "./bytes.js";
import { ATA_PROGRAM_ID, KASSANDRA_PROGRAM_ID, TOKEN_PROGRAM_ID } from "./constants.js";

/** Anything that can name an account: a web3.js `Address`/`PublicKey` or a base58 string. */
export type AddressInput = Address | string;

/** A derived PDA: its address and the canonical bump seed. */
export interface Pda {
  address: Address;
  bump: number;
}

const enc = new TextEncoder();

/** 32 raw bytes of an address (the seed form of a pubkey). */
function pubkeyBytes(a: AddressInput): Uint8Array {
  return (a instanceof Address ? a : new Address(a)).toBytes();
}

async function derive(seeds: Array<Uint8Array>, programId: Address = KASSANDRA_PROGRAM_ID): Promise<Pda> {
  const [address, bump] = await Address.findProgramAddress(seeds, programId);
  return { address, bump };
}

/** Protocol singleton PDA — seeds `[b"protocol"]`. */
export function protocol(programId?: Address): Promise<Pda> {
  return derive([enc.encode("protocol")], programId);
}

/** KASS mint-authority PDA — seeds `[b"mint_authority"]`. */
export function mintAuthority(programId?: Address): Promise<Pda> {
  return derive([enc.encode("mint_authority")], programId);
}

/** Oracle PDA — seeds `[b"oracle", nonce_u64_le]`. */
export function oracle(nonce: bigint | number, programId?: Address): Promise<Pda> {
  return derive([enc.encode("oracle"), u64LE(nonce)], programId);
}

/** Oracle stake-vault PDA (KASS token account) — seeds `[b"vault", oracle]`. */
export function stakeVault(oracleAddr: AddressInput, programId?: Address): Promise<Pda> {
  return derive([enc.encode("vault"), pubkeyBytes(oracleAddr)], programId);
}

/** Oracle-metadata PDA — seeds `[b"oracle_meta", oracle]`. Holds the plaintext
 *  subject + option labels + uri/uri_hash written by `write_oracle_meta`. */
export function oracleMeta(oracleAddr: AddressInput, programId?: Address): Promise<Pda> {
  return derive([enc.encode("oracle_meta"), pubkeyBytes(oracleAddr)], programId);
}

/** Proposer PDA — seeds `[b"proposer", oracle, authority]`. */
export function proposer(
  oracleAddr: AddressInput,
  authority: AddressInput,
  programId?: Address,
): Promise<Pda> {
  return derive([enc.encode("proposer"), pubkeyBytes(oracleAddr), pubkeyBytes(authority)], programId);
}

/** Fact PDA — seeds `[b"fact", oracle, content_hash]`. `contentHash` is 32 bytes. */
export function fact(
  oracleAddr: AddressInput,
  contentHash: Uint8Array,
  programId?: Address,
): Promise<Pda> {
  return derive([enc.encode("fact"), pubkeyBytes(oracleAddr), contentHash], programId);
}

/** FactVote PDA — seeds `[b"vote", fact, voter]`. */
export function factVote(
  factAddr: AddressInput,
  voter: AddressInput,
  programId?: Address,
): Promise<Pda> {
  return derive([enc.encode("vote"), pubkeyBytes(factAddr), pubkeyBytes(voter)], programId);
}

/** AiClaim PDA — seeds `[b"claim", oracle, proposer]`. */
export function aiClaim(
  oracleAddr: AddressInput,
  proposerAddr: AddressInput,
  programId?: Address,
): Promise<Pda> {
  return derive([enc.encode("claim"), pubkeyBytes(oracleAddr), pubkeyBytes(proposerAddr)], programId);
}

/** Market PDA — seeds `[b"market", ai_claim]`. */
export function market(aiClaimAddr: AddressInput, programId?: Address): Promise<Pda> {
  return derive([enc.encode("market"), pubkeyBytes(aiClaimAddr)], programId);
}

/** Challenger USDC escrow PDA (SPL token account) — seeds `[b"challenge_usdc", market]`. */
export function challengeUsdcVault(marketAddr: AddressInput, programId?: Address): Promise<Pda> {
  return derive([enc.encode("challenge_usdc"), pubkeyBytes(marketAddr)], programId);
}

/**
 * SPL associated-token-account address — seeds `[owner, TOKEN_PROGRAM, mint]`
 * under the {@link ATA_PROGRAM_ID}. `sweep_oracle`'s DAO treasury is
 * `ATA(dao_authority, kass_mint)`, derived exactly as the program does in
 * `processor/sweep_oracle.rs`. NOTE: derived under the ATA program, NOT the
 * Kassandra program — there is no `programId` override.
 */
export function associatedTokenAccount(
  owner: AddressInput,
  mint: AddressInput,
): Promise<Pda> {
  return derive([pubkeyBytes(owner), TOKEN_PROGRAM_ID.toBytes(), pubkeyBytes(mint)], ATA_PROGRAM_ID);
}
