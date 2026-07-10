/**
 * Shared harness, consts, deposit math and fixtures for the M2 surfpool Meteora
 * DAMM v2 spot-path E2E. Pure move-out of the module-level helpers from
 * meteora-spot-e2e.test.ts — no assertion or runtime-logic changes.
 */
import {
  Address,
  ComputeBudgetProgram,
  Keypair,
  Transaction,
  TransactionInstruction,
} from "@solana/web3.js";

import { meteora } from "../../src/index.js";
import { TOKEN_PROGRAM_ID } from "../../src/constants.js";

import {
  SurfpoolHarness,
  mintBytes,
  toHex,
  tokenAccountAmount,
  tokenAccountBytes,
} from "./harness.js";

export const MAINNET_RPC = "https://api.mainnet-beta.solana.com";

/** A REAL public + static mainnet cp-amm Config (index 0): `pool_creator_authority
 * == Pubkey::default` (permissionless), `config_type == Static`. Cloned onto the
 * fork so `initialize_pool` accepts our arbitrary payer as pool creator. */
export const REAL_CONFIG = new Address("8CNy9goNQNLM4wtgRw528tUQGMKD3vSuFRZY2gLGLLvF");

/** A REAL mainnet cp-amm Pool (token_b == USDC) — decoded from genuine deployed
 * bytes as an independent cross-check of the SDK decoder. */
export const REAL_POOL = new Address("11BWLuxs8ow5x42hXjVPi55j9KLVa4SCn1MspbBepVQ");

/** Full-range price bounds baked into the public configs (Q64.64). */
export const SQRT_MIN = 4295048016n;
export const SQRT_MAX = 79226673521066979257578248091n;
/** Initial price 1.0 → sqrt_price = 2^64. */
export const SQRT_PRICE_INIT = 1n << 64n;
/** Liquidity chosen so both deposit amounts land near 1e9 raw (~1000 tokens @6dp). */
export const INIT_LIQUIDITY = 1_000_000_000n * (1n << 64n);
export const ADD_LIQUIDITY = INIT_LIQUIDITY / 2n;
export const U64_MAX = (1n << 64n) - 1n;

// cp-amm deposit math (concentrated_liquidity.rs, Rounding::Up) — used to compute
// the EXACT amounts the program will pull, so we can assert the decoded reserves.
export function ceilDiv(n: bigint, d: bigint): bigint {
  return (n + d - 1n) / d;
}
/** token_a = ceil(L*(upper - lower) / (lower*upper)). */
export function deltaA(lower: bigint, upper: bigint, L: bigint): bigint {
  return ceilDiv(L * (upper - lower), lower * upper);
}
/** token_b = ceil(L*(upper - lower) / 2^128). */
export function deltaB(lower: bigint, upper: bigint, L: bigint): bigint {
  return ceilDiv(L * (upper - lower), 1n << 128n);
}

export interface Fixture {
  harness: SurfpoolHarness;
  payer: Keypair;
  mintA: Address;
  mintB: Address;
  payerTokenA: Address;
  payerTokenB: Address;
}

/**
 * Boots surfpool FORKING MAINNET, clones the REAL mainnet Config, fabricates two
 * SPL mints + funded payer token accounts. Extracted verbatim from the original
 * beforeAll — same statements, same order.
 */
export async function startFixture(): Promise<Fixture> {
  const harness = await SurfpoolHarness.start({
    port: 8922,
    fork: "mainnet",
    readyTimeoutMs: 60_000,
  });
  const payer = await Keypair.generate();
  await harness.airdrop(payer.publicKey.toString(), 500_000_000_000);

  // Clone the REAL mainnet Config onto the fork (fetch its exact bytes from
  // mainnet, write them cp-amm-owned). Guarantees the config is present +
  // deterministic regardless of the fork's lazy-fetch behaviour.
  const cfg = await fetchMainnetAccount(REAL_CONFIG);
  await harness.setAccount(REAL_CONFIG.toString(), {
    lamports: 5_000_000,
    owner: meteora.METEORA_DAMM_V2_ID.toString(),
    executable: false,
    data: toHex(cfg),
  });

  // Two fabricated SPL mints (6 dp), authority = payer.
  const mA = await Keypair.generate();
  const mB = await Keypair.generate();
  for (const m of [mA, mB]) {
    await harness.setAccount(m.publicKey.toString(), {
      lamports: 1_000_000_000,
      owner: TOKEN_PROGRAM_ID.toString(),
      executable: false,
      data: toHex(mintBytes(payer.publicKey.toBytes(), 10n ** 18n, 6)),
    });
  }
  const mintA = mA.publicKey;
  const mintB = mB.publicKey;

  // Payer source token accounts, funded far above what init will pull.
  const payerTokenA = await fabricateTokenAccount(harness, mintA, payer.publicKey, 10n ** 15n);
  const payerTokenB = await fabricateTokenAccount(harness, mintB, payer.publicKey, 10n ** 15n);

  return { harness, payer, mintA, mintB, payerTokenA, payerTokenB };
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

export async function fabricateTokenAccount(
  harness: SurfpoolHarness,
  mint: Address,
  owner: Address,
  amount: bigint,
): Promise<Address> {
  const acct = await Keypair.generate();
  await harness.setAccount(acct.publicKey.toString(), {
    lamports: 5_000_000,
    owner: TOKEN_PROGRAM_ID.toString(),
    executable: false,
    data: toHex(tokenAccountBytes(mint.toBytes(), owner.toBytes(), amount)),
  });
  return acct.publicKey;
}

export async function fetchAccount(
  f: Fixture,
  address: Address,
  timeoutMs = 20_000,
): Promise<Uint8Array> {
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
