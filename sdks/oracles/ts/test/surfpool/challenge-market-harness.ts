/**
 * T4 surfpool CHALLENGE-MARKET E2E — shared primitives (consts, interfaces, and
 * low-level drivers). Extracted from `challenge-market-e2e.test.ts` so no single
 * file exceeds ~400 lines. Pure move: bodies are verbatim.
 */
import {
  Address,
  ComputeBudgetProgram,
  Keypair,
  Transaction,
  TransactionInstruction,
} from "@solana/web3.js";

import { decodeOracle } from "../../src/accounts/index.js";
import { EXTERNAL_PROGRAM_IDS, TOKEN_PROGRAM_ID, VOTE_APPROVE } from "../../src/constants.js";
import { createOracle, propose } from "../../src/instructions/index.js";
import * as pda from "../../src/pda.js";

import {
  SurfpoolHarness,
  surfpoolReady,
  tokenAccountAmount,
  tokenAccountBytes,
  toHex,
} from "./harness.js";

export const ENABLED = process.env.KASSANDRA_E2E === "1" && surfpoolReady();

export const SYSTEM_PROGRAM_ID = new Address("11111111111111111111111111111111");
export const ATA_PROGRAM_ID = new Address("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");
export const VLTX = EXTERNAL_PROGRAM_IDS.conditionalVault;
export const AMM_ID = EXTERNAL_PROGRAM_IDS.ammV04;
export const FUTARCHY_ID = EXTERNAL_PROGRAM_IDS.futarchyV06;
export const BPF_UPGRADEABLE = "BPFLoaderUpgradeab1e11111111111111111111111";

// MetaDAO Anchor discriminators (mirror `src/cpi/metadao.rs`).
export const INITIALIZE_QUESTION = Uint8Array.from([0xf5, 0x97, 0x6a, 0xbc, 0x58, 0x2c, 0x41, 0xd4]);
export const INITIALIZE_CONDITIONAL_VAULT = Uint8Array.from([0x25, 0x58, 0xfa, 0xd4, 0x36, 0xda, 0xe3, 0xaf]);

// `kass_price` consts (mirror `config.rs` + the Rust test harness).
export const KASS_PRICE_TWAP = 500_000_000n;
export const KASS_PRICE_SCALE = 1_000_000_000_000n;

export const enc = new TextEncoder();

/** 1 KASS (9 dp) bond — large enough that required_usdc = bond×twap/scale > 0. */
export const BOND = 1_000_000_000n;

// --- v0.4 AMM pool seeding (mirror challenge_e2e.rs build_pool) ---------------
/** Largest per-update observation change — a single crank folds the current
 * price straight into the TWAP (no clamp), so the cranked TWAP is deterministic.
 * (== `u64::MAX × 1e12`, the same value the Rust e2e uses.) */
export const MAX_PRICE = ((1n << 64n) - 1n) * 1_000_000_000_000n;
/** Base reserve: 100 conditional-KASS (9 dp). */
export const BASE_RESERVE = 100_000_000_000n;
/** Quote reserve: 100 conditional-USDC (6 dp) → seeded price 1e9 (scaled). */
export const QUOTE_NEUTRAL = 100_000_000n;

export interface Fixture {
  harness: SurfpoolHarness;
  payer: Keypair;
  kassMint: Keypair;
  usdcMint: Keypair;
  kassDao: Address;
}

// ---------------------------------------------------------------------------
// MetaDAO market composition over RPC (mirrors challenge_e2e.rs setup_market)
// ---------------------------------------------------------------------------
export interface VaultAccounts {
  vault: Address;
  underlying: Address;
  passMint: Address;
  failMint: Address;
}

/** ATA derivation `[owner, token_program, mint]` under the ATA program. */
export async function ata(owner: Address, mint: Address): Promise<Address> {
  return (
    await Address.findProgramAddress(
      [owner.toBytes(), TOKEN_PROGRAM_ID.toBytes(), mint.toBytes()],
      ATA_PROGRAM_ID,
    )
  )[0];
}

/** Real `initialize_conditional_vault` CPI for `underlyingMint` against `question`. */
export async function composeVault(f: Fixture, question: Address, underlyingMint: Address): Promise<VaultAccounts> {
  const [vault] = await Address.findProgramAddress(
    [enc.encode("conditional_vault"), question.toBytes(), underlyingMint.toBytes()],
    VLTX,
  );
  const [passMint] = await Address.findProgramAddress(
    [enc.encode("conditional_token"), vault.toBytes(), Uint8Array.from([0])],
    VLTX,
  );
  const [failMint] = await Address.findProgramAddress(
    [enc.encode("conditional_token"), vault.toBytes(), Uint8Array.from([1])],
    VLTX,
  );
  const [eventAuthority] = await Address.findProgramAddress([enc.encode("__event_authority")], VLTX);
  const underlying = await ata(vault, underlyingMint);

  await sendIx(
    f,
    new TransactionInstruction({
      programId: VLTX,
      keys: [
        { pubkey: vault, isSigner: false, isWritable: true },
        { pubkey: question, isSigner: false, isWritable: false },
        { pubkey: underlyingMint, isSigner: false, isWritable: false },
        { pubkey: underlying, isSigner: false, isWritable: true },
        { pubkey: f.payer.publicKey, isSigner: true, isWritable: true },
        { pubkey: TOKEN_PROGRAM_ID, isSigner: false, isWritable: false },
        { pubkey: ATA_PROGRAM_ID, isSigner: false, isWritable: false },
        { pubkey: SYSTEM_PROGRAM_ID, isSigner: false, isWritable: false },
        { pubkey: eventAuthority, isSigner: false, isWritable: false },
        { pubkey: VLTX, isSigner: false, isWritable: false },
        { pubkey: passMint, isSigner: false, isWritable: true },
        { pubkey: failMint, isSigner: false, isWritable: true },
      ],
      data: INITIALIZE_CONDITIONAL_VAULT,
    }),
    [],
    400_000,
  );
  return { vault, underlying, passMint, failMint };
}

/** Fabricate an SPL token account (owner = `owner`) on `mint` with `amount`. */
export async function fabricateTokenAccountMint(
  f: Fixture,
  mint: Address,
  owner: Address,
  amount: bigint,
): Promise<Address> {
  const acct = await Keypair.generate();
  await f.harness.setAccount(acct.publicKey.toString(), {
    lamports: 5_000_000,
    owner: TOKEN_PROGRAM_ID.toString(),
    executable: false,
    data: toHex(tokenAccountBytes(mint.toBytes(), owner.toBytes(), amount)),
  });
  return acct.publicKey;
}

/** Fabricate a placeholder account owned by the AMM program (open only checks owner). */
export async function fabricateAmmOwned(f: Fixture): Promise<Address> {
  const acct = await Keypair.generate();
  await f.harness.setAccount(acct.publicKey.toString(), {
    lamports: 5_000_000,
    owner: AMM_ID.toString(),
    executable: false,
    data: toHex(new Uint8Array(8)),
  });
  return acct.publicKey;
}

// ---------------------------------------------------------------------------
// Front door → Challenge + market composition + open/settle (shared by arms)
// ---------------------------------------------------------------------------

export interface Challenged {
  oracle: Address;
  proposer: Address;
  proposerAuthority: Address;
  aiClaim: Address;
  proposerPdas: Address[];
  authorities: Keypair[];
}

export interface MarketComposition {
  question: Address;
  kass: VaultAccounts;
  usdc: VaultAccounts;
  oraclePassKass: Address;
  oracleFailKass: Address;
}

export interface Payouts {
  escrowVault: Address;
  proposerUsdc: Address;
  challengerUsdcDest: Address;
  challengerKass: Address;
}

// ---------------------------------------------------------------------------
// Real v0.4 AMM pool driving over RPC (port challenge_e2e.rs build/swap/crank)
// ---------------------------------------------------------------------------

/** Write canonical SPL token-account bytes AT a specific (ATA) address. */
export async function setTokenAccountAt(
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

/** Wait for the on-chain EXECUTION slot to advance by ≥ `n` (the v0.4 AMM crank
 * rate-limit is slot-based: `ONE_MINUTE_IN_SLOTS == 150`). In `clock`
 * block-production mode the slot advances on a wall-clock timer, so we poll
 * `getSlot` until it has moved past `start + n`. */
export async function advanceSlots(f: Fixture, n: number): Promise<void> {
  const start = await f.harness.currentSlot();
  const deadline = Date.now() + 30_000;
  while (Date.now() < deadline) {
    if ((await f.harness.currentSlot()) >= start + n) return;
    await new Promise((r) => setTimeout(r, 100));
  }
  throw new Error(`slot did not advance by ${n} within 30s (clock mode not producing blocks?)`);
}

/** Decode the v0.4 `Amm` TWAP fields + compute `get_twap()` (offsets from
 * `cpi/metadao.rs`: created_at @9, last_updated @131, aggregator(u128) @171,
 * start_delay @219). */
export function decodeAmmTwap(data: Uint8Array): {
  createdAt: bigint;
  lastUpdated: bigint;
  aggregator: bigint;
  startDelay: bigint;
  twap: bigint;
} {
  const dv = new DataView(data.buffer, data.byteOffset, data.length);
  const u128 = (off: number): bigint => dv.getBigUint64(off, true) | (dv.getBigUint64(off + 8, true) << 64n);
  const createdAt = dv.getBigUint64(9, true);
  const lastUpdated = dv.getBigUint64(131, true);
  const aggregator = u128(171);
  const startDelay = dv.getBigUint64(219, true);
  const slots = lastUpdated - (createdAt + startDelay);
  const twap = slots > 0n && aggregator > 0n ? aggregator / slots : 0n;
  return { createdAt, lastUpdated, aggregator, startDelay, twap };
}

/** Read `Question.payout_numerators[0..2]` (after the u32 Vec length @72; the two
 * u32 numerators at @76, @80) to confirm the resolution `[pass, fail]`. */
export function questionResolution(data: Uint8Array): [number, number] {
  const dv = new DataView(data.buffer, data.byteOffset, data.length);
  return [dv.getUint32(76, true), dv.getUint32(80, true)];
}

// ---------------------------------------------------------------------------
// Dispute-core drivers over RPC (self-contained; mirror lifecycle-e2e.test.ts)
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

export async function fundKass(f: Fixture, owner: Address, amount: bigint): Promise<Address> {
  return fabricateTokenAccountMint(f, f.kassMint.publicKey, owner, amount);
}

export async function createOracleReal(f: Fixture, nonce: bigint, optionsCount: number): Promise<void> {
  const creatorKass = await fundKass(f, f.payer.publicKey, 10n ** 15n);
  const nowUnix = await f.harness.clockUnixTimestamp();
  await sendIx(
    f,
    await createOracle({
      nonce,
      optionsCount,
      deadline: nowUnix + 1_000n,
      twapWindow: 600n,
      creator: f.payer.publicKey,
      creatorKassToken: creatorKass,
      kassMint: f.kassMint.publicKey,
      usdcMint: f.usdcMint.publicKey,
    }),
  );
}

export async function openProposals(f: Fixture, oracle: Address): Promise<void> {
  const o = decodeOracle(await fetchAccount(f, oracle));
  await f.harness.advanceToUnix(o.deadline + 60n);
}

export async function advancePastPhaseEnd(f: Fixture, oracle: Address): Promise<void> {
  const o = decodeOracle(await fetchAccount(f, oracle));
  await f.harness.advanceToUnix(o.phaseEndsAt + 120n);
}

export async function proposeRealWithAuthority(
  f: Fixture,
  oracle: Address,
  option: number,
  bond: bigint,
): Promise<{ authority: Keypair; proposer: Address }> {
  const authority = await Keypair.generate();
  await f.harness.airdrop(authority.publicKey.toString(), 2_000_000_000);
  const authorityKass = await fundKass(f, authority.publicKey, bond * 10n);
  await sendIx(f, await propose({ oracle, authority: authority.publicKey, authorityKass, option, bond }), [authority]);
  const proposer = (await pda.proposer(oracle, authority.publicKey)).address;
  return { authority, proposer };
}
