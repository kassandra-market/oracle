/**
 * RF3 GATED surfpool create-oracle E2E (`KASSANDRA_E2E=1`).
 *
 * Proves the create-oracle ACTION lands a real Oracle on-chain: boot + deploy +
 * init_protocol + mint KASS to a funded USER keypair at its canonical ATA, then
 * drive `buildCreateOracleIxs` through the {@link keypairSender}-backed
 * {@link sendAndConfirm} seam (the SAME action the UI uses) and decode the
 * created Oracle — asserting optionsCount, deadline, and creator == the user.
 *
 * Gated: skips (never fails) unless `KASSANDRA_E2E=1` AND surfpool + the built
 * `.so` are present.
 */
import { Keypair, Transaction, type Address, type TransactionInstruction } from "@solana/web3.js";
import {
  TOKEN_PROGRAM_ID,
  associatedTokenAccount,
  decodeOracle,
  initProtocol,
} from "@kassandra-market/oracles";
import * as pda from "@kassandra-market/oracles";
import { afterAll, beforeAll, describe, expect, it } from "vitest";

import {
  SurfpoolHarness,
  mintBytes,
  surfpoolReady,
  toHex,
  tokenAccountBytes,
} from "../../sdks/oracles/ts/test/surfpool/harness.ts";
import { buildCreateOracleIxs } from "../src/data/actions/create.ts";
import { keypairSender, sendAndConfirm } from "../src/data/send.ts";

const ENABLED = process.env.KASSANDRA_E2E === "1" && surfpoolReady();

interface Fixture {
  harness: SurfpoolHarness;
  payer: Keypair;
  kassMint: Keypair;
  usdcMint: Keypair;
}

describe.skipIf(!ENABLED)("create-oracle action over a real surfpool cluster", () => {
  let f: Fixture;
  let user: Keypair;

  beforeAll(async () => {
    const harness = await SurfpoolHarness.start({ port: 8907 });
    const payer = await Keypair.generate();
    await harness.airdrop(payer.publicKey.toString(), 1_000_000_000_000);

    const mintAuth = await pda.mintAuthority();
    const kassMint = await Keypair.generate();
    const usdcMint = await Keypair.generate();
    await harness.setAccount(kassMint.publicKey.toString(), {
      lamports: 1_000_000_000,
      owner: TOKEN_PROGRAM_ID.toString(),
      executable: false,
      data: toHex(mintBytes(mintAuth.address.toBytes(), 10n ** 18n, 9)),
    });
    await harness.setAccount(usdcMint.publicKey.toString(), {
      lamports: 1_000_000_000,
      owner: TOKEN_PROGRAM_ID.toString(),
      executable: false,
      data: toHex(mintBytes(payer.publicKey.toBytes(), 0n, 6)),
    });

    f = { harness, payer, kassMint, usdcMint };
    await sendIx(f, await initProtocol({
      admin: payer.publicKey,
      kassMint: kassMint.publicKey,
      usdcMint: usdcMint.publicKey,
    }));

    // The USER keypair the action layer drives: funded SOL + KASS at its CANONICAL
    // ATA (the creation-fee burn source the action derives + uses).
    user = await Keypair.generate();
    await harness.airdrop(user.publicKey.toString(), 10_000_000_000);
    const userAta = (await associatedTokenAccount(user.publicKey, kassMint.publicKey)).address;
    await harness.setAccount(userAta.toString(), {
      lamports: 5_000_000,
      owner: TOKEN_PROGRAM_ID.toString(),
      executable: false,
      data: toHex(tokenAccountBytes(kassMint.publicKey.toBytes(), user.publicKey.toBytes(), 10n ** 15n)),
    });
  }, 180_000);

  afterAll(async () => {
    await f?.harness.teardown();
  });

  it("buildCreateOracleIxs + sendAndConfirm lands a real Oracle with the hashed prompt", async () => {
    const nonce = 1n;
    const question = "Did the Kassandra dApp create-flow land on-chain?";
    const optionsCount = 3;
    const nowUnix = await f.harness.clockUnixTimestamp();
    const deadline = nowUnix + 1_000n;

    const sender = keypairSender(f.harness.connection, user);
    const built = await buildCreateOracleIxs({
      connection: f.harness.connection,
      nonce,
      question,
      optionsCount,
      deadline,
      creator: user.publicKey,
      kassMint: f.kassMint.publicKey,
      usdcMint: f.usdcMint.publicKey,
    });
    const { signature } = await sendAndConfirm(f.harness.connection, sender, built.ixs);
    expect(signature).toBeTruthy();

    const oracle = decodeOracle(await fetchAccount(f, built.oracle));
    expect(oracle.optionsCount).toBe(optionsCount);
    expect(oracle.deadline).toBe(deadline);
    expect(oracle.creator.toString()).toBe(user.publicKey.toString());
    expect(oracle.kassMint.toString()).toBe(f.kassMint.publicKey.toString());
    expect(oracle.usdcMint.toString()).toBe(f.usdcMint.publicKey.toString());
  }, 120_000);
});

// ---------------------------------------------------------------------------
// Real-instruction drivers over RPC (mirrors the WF1/read surfpool E2Es).
// ---------------------------------------------------------------------------

async function sendIx(f: Fixture, ix: TransactionInstruction, signers: Keypair[] = []): Promise<void> {
  const conn = f.harness.connection;
  const tx = new Transaction();
  tx.feePayer = f.payer.publicKey;
  tx.recentBlockhash = (await conn.getLatestBlockhash()).blockhash;
  tx.add(ix);
  await tx.sign(f.payer, ...signers);
  const sig = await conn.sendRawTransaction(await tx.serialize(), { skipPreflight: false });
  await f.harness.confirmSignature(sig);
}

async function fetchAccount(f: Fixture, address: Address, timeoutMs = 15_000): Promise<Uint8Array> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const info = await f.harness.connection.getAccountInfo(address);
    if (info && info.data.length > 0) return info.data;
    await new Promise((r) => setTimeout(r, 150));
  }
  throw new Error(`account ${address} did not appear within ${timeoutMs}ms`);
}
