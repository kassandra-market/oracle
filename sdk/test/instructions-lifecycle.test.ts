/**
 * D3a — instruction-builder byte/meta tests.
 *
 * For each protocol + oracle-lifecycle builder we assert:
 *   - `data` == `[disc, ...payload_LE]`, with the expected payload buffer
 *     constructed INDEPENDENTLY here (so a regression in the encoder is caught);
 *   - `keys` == the processor's documented account order, each with the correct
 *     `isSigner`/`isWritable` role, and the PDA-derived accounts in the right
 *     slots (cross-checked against the `*_ix` helpers in
 *     `programs/kassandra/tests/common/mod.rs`).
 *
 * One builder (init_protocol) is additionally driven through the litesvm path:
 * built via the SDK, signed with web3.js v3, bridged into litesvm, and asserted
 * ACCEPTED by the real program (proves the account order + roles match).
 */
import { existsSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { address, lamports } from "@solana/kit";
import { Address, Keypair, Transaction } from "@solana/web3.js";
import { LiteSVM, TransactionMetadata } from "litesvm";
import { describe, expect, it } from "vitest";

import {
  Ix,
  KASSANDRA_PROGRAM_ID,
  SYSTEM_PROGRAM_ID,
  TOKEN_PROGRAM_ID,
} from "../src/constants.js";
import * as pda from "../src/pda.js";
import {
  advancePhase,
  createOracle,
  encodeSetConfigParams,
  finalizeProposals,
  initProtocol,
  kassPrice,
  propose,
  resolveDeadend,
  setConfig,
  setGovernance,
  writeOracleMeta,
  type SetConfigParams,
} from "../src/instructions/index.js";
import { decodeOracleMeta } from "../src/accounts/index.js";
import { toLiteSvmTransaction } from "../src/litesvm-interop.js";

const PROGRAM_ID = "KassVxvXUEPr5apSr2MqiGva4VFtJXyYLLDFS3f83nY";

// Deterministic stand-in keys (valid 32-byte base58 addresses).
const ADMIN = "rqRMW2HFJsi1FE1jb8Rvaz4Qz3xHzNkZDb8am1pqEHE";
const KASS_MINT = "So11111111111111111111111111111111111111112";
const USDC_MINT = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
const CREATOR = "84yVtdReAJ8GiR7Erqj7jyxoJurYWzQ6n9eaBGYBDNqM";
const CREATOR_KASS = "7WCvk98KGRqi2o8D7EWTGrZQuFtikidP8A2D7CDVXwWJ";
const AUTHORITY = "7bQEwuq9ybNyjjFcbtHBfDPxdH3TuGAsZKVRZdihVN4d";
const AUTHORITY_KASS = "EScpWtUwYodKnbZx46YYeJbp2Ci2EpqcLAkF2EdZnZrh";
const ORACLE = "GuBhyNi5GFo9K5YXGKfPMDryWK8GwS5oXe9CJGrzo2sk";
const KASS_DAO = "FYQFL976rxQv8hygbC1zPVZYMfbnQkVntriESv69KaED";

/** Build the same [disc, ...payload] buffer independently for cross-checking. */
function bytesOf(disc: Ix, payload: number[] = []): Uint8Array {
  return new Uint8Array([disc, ...payload]);
}

function leU64(v: bigint): number[] {
  const b = new Uint8Array(8);
  new DataView(b.buffer).setBigUint64(0, v, true);
  return Array.from(b);
}

function leI64(v: bigint): number[] {
  const b = new Uint8Array(8);
  new DataView(b.buffer).setBigInt64(0, v, true);
  return Array.from(b);
}

function metaTriples(keys: { pubkey: Address; isSigner: boolean; isWritable: boolean }[]) {
  return keys.map((k) => [k.pubkey.toString(), k.isSigner, k.isWritable] as const);
}

describe("D3a instruction builders — data bytes + account metas", () => {
  it("initProtocol: empty payload, 5 accounts in order", async () => {
    const ix = await initProtocol({ admin: ADMIN, kassMint: KASS_MINT, usdcMint: USDC_MINT });
    const protocol = await pda.protocol();

    expect(ix.programId.toString()).toBe(PROGRAM_ID);
    expect(ix.data).toEqual(bytesOf(Ix.InitProtocol));
    expect(metaTriples(ix.keys)).toEqual([
      [protocol.address.toString(), false, true],
      [ADMIN, true, true],
      [KASS_MINT, false, false],
      [USDC_MINT, false, false],
      [SYSTEM_PROGRAM_ID.toString(), false, false],
    ]);
  });

  it("createOracle: 25-byte payload (nonce, options, deadline, twap) + 10 accounts", async () => {
    const nonce = 7n;
    const optionsCount = 3;
    const deadline = 1_700_000_000n;
    const twapWindow = 86_400n;

    const ix = await createOracle({
      nonce,
      optionsCount,
      deadline,
      twapWindow,
      creator: CREATOR,
      creatorKassToken: CREATOR_KASS,
      kassMint: KASS_MINT,
      usdcMint: USDC_MINT,
    });

    const expected = bytesOf(Ix.CreateOracle, [
      ...leU64(nonce),
      optionsCount,
      ...leI64(deadline),
      ...leI64(twapWindow),
    ]);
    expect(ix.data).toEqual(expected);
    expect(ix.data.length).toBe(1 + 25);

    const protocol = await pda.protocol();
    const oracle = await pda.oracle(nonce);
    const stakeVault = await pda.stakeVault(oracle.address);
    const mintAuth = await pda.mintAuthority();

    expect(metaTriples(ix.keys)).toEqual([
      [protocol.address.toString(), false, true],
      [oracle.address.toString(), false, true],
      [stakeVault.address.toString(), false, true],
      [CREATOR, true, true],
      [KASS_MINT, false, true],
      [USDC_MINT, false, false],
      [TOKEN_PROGRAM_ID.toString(), false, false],
      [SYSTEM_PROGRAM_ID.toString(), false, false],
      [CREATOR_KASS, false, true],
      [mintAuth.address.toString(), false, false],
    ]);
  });

  it("writeOracleMeta: length-prefixed body round-trips through decodeOracleMeta", async () => {
    const oracle = (await pda.oracle(7n)).address;
    const subject = "Which team wins?";
    const options = ["Red", "Blue", "Draw"];
    const uri = "https://app.example/api/oracle/x/metadata.json";
    const uriHash = new Uint8Array(32).fill(0xab);

    const ix = await writeOracleMeta({
      oracle,
      creator: CREATOR,
      subject,
      options,
      uri,
      uriHash,
    });

    expect(ix.data[0]).toBe(Ix.WriteOracleMeta);
    // Accounts: creator(signer,w), oracle(ro), oracle_meta(w), system(ro).
    const meta = await pda.oracleMeta(oracle);
    expect(metaTriples(ix.keys)).toEqual([
      [CREATOR, true, true],
      [oracle.toString(), false, false],
      [meta.address.toString(), false, true],
      [SYSTEM_PROGRAM_ID.toString(), false, false],
    ]);

    // Rebuild the on-chain account body (header + the ix payload) and decode it.
    const body = ix.data.slice(1);
    const account = new Uint8Array(34 + body.length);
    account[0] = 8; // AccountType.OracleMeta
    account.set(oracle.toBytes(), 2);
    account.set(body, 34);
    const decoded = decodeOracleMeta(account);
    expect(decoded.subject).toBe(subject);
    expect(decoded.options).toEqual(options);
    expect(decoded.uri).toBe(uri);
    expect(Array.from(decoded.uriHash)).toEqual(Array.from(uriHash));
  });

  it("propose: option u8 ++ bond u64 + 7 accounts", async () => {
    const option = 2;
    const bond = 1_500n;
    const ix = await propose({
      oracle: ORACLE,
      authority: AUTHORITY,
      authorityKass: AUTHORITY_KASS,
      option,
      bond,
    });

    expect(ix.data).toEqual(bytesOf(Ix.Propose, [option, ...leU64(bond)]));

    const proposer = await pda.proposer(ORACLE, AUTHORITY);
    const stakeVault = await pda.stakeVault(ORACLE);
    expect(metaTriples(ix.keys)).toEqual([
      [ORACLE, false, true],
      [proposer.address.toString(), false, true],
      [AUTHORITY, true, true],
      [AUTHORITY_KASS, false, true],
      [stakeVault.address.toString(), false, true],
      [TOKEN_PROGRAM_ID.toString(), false, false],
      [SYSTEM_PROGRAM_ID.toString(), false, false],
    ]);
  });

  it("finalizeProposals: empty payload, oracle(w) + read-only proposer tail", async () => {
    const proposers = [AUTHORITY, CREATOR, KASS_MINT];
    const ix = await finalizeProposals({ oracle: ORACLE, proposers });

    expect(ix.data).toEqual(bytesOf(Ix.FinalizeProposals));
    expect(metaTriples(ix.keys)).toEqual([
      [ORACLE, false, true],
      [AUTHORITY, false, false],
      [CREATOR, false, false],
      [KASS_MINT, false, false],
    ]);
  });

  it("advancePhase: empty payload, single oracle(w), no signer", async () => {
    const ix = await advancePhase({ oracle: ORACLE });
    expect(ix.data).toEqual(bytesOf(Ix.AdvancePhase));
    expect(metaTriples(ix.keys)).toEqual([[ORACLE, false, true]]);
  });

  it("setGovernance: dao_authority[32] ++ kass_dao[32] + protocol(w), authority(ro signer), kass_dao(ro)", async () => {
    const daoAuthority = AUTHORITY;
    const kassDao = KASS_DAO;
    const ix = await setGovernance({ authority: ADMIN, daoAuthority, kassDao });

    const daoBytes = Array.from(new Address(daoAuthority).toBytes());
    const kassDaoBytes = Array.from(new Address(kassDao).toBytes());
    expect(ix.data).toEqual(bytesOf(Ix.SetGovernance, [...daoBytes, ...kassDaoBytes]));
    expect(ix.data.length).toBe(1 + 64);

    const protocol = await pda.protocol();
    expect(metaTriples(ix.keys)).toEqual([
      [protocol.address.toString(), false, true],
      [ADMIN, true, false],
      [KASS_DAO, false, false],
    ]);
  });

  it("setConfig: 25-field 200-byte LE payload, exact field order", async () => {
    // Distinct values per field index so a misordering is caught.
    const params: SetConfigParams = {
      emissionNum: 100n,
      emissionDen: 101n,
      totalSupplyCap: 102n,
      feeEmaHalflife: 103n,
      feePerEmaUnit: 104n,
      feeEmaIncrement: 105n,
      thresholdNum: 106n,
      thresholdDen: 107n,
      marketThresholdNum: 108n,
      marketThresholdDen: 109n,
      flipSlashNum: 110n,
      flipSlashDen: 111n,
      phaseWindow: 112n,
      proposalWindow: 113n,
      factVoteSlashNum: 114n,
      factVoteSlashDen: 115n,
      rewardProposerWeight: 116n,
      rewardFactWeight: 117n,
      challengeFailUsdcFeeNum: 118n,
      challengeFailUsdcFeeDen: 119n,
      challengeSuccessKassFeeNum: 120n,
      challengeSuccessKassFeeDen: 121n,
      stakeFloorEmaThreshold: 122n,
      stakeFloorEmaCap: 123n,
      stakeFloorMax: 124n,
    };

    // Independently pack the 22 fields in the documented order (indices 0..=21,
    // matching set_config.rs `u64_at`/`i64_at`).
    const ordered: bigint[] = [
      params.emissionNum,
      params.emissionDen,
      params.totalSupplyCap,
      params.feeEmaHalflife,
      params.feePerEmaUnit,
      params.feeEmaIncrement,
      params.thresholdNum,
      params.thresholdDen,
      params.marketThresholdNum,
      params.marketThresholdDen,
      params.flipSlashNum,
      params.flipSlashDen,
      params.phaseWindow,
      params.proposalWindow,
      params.factVoteSlashNum,
      params.factVoteSlashDen,
      params.rewardProposerWeight,
      params.rewardFactWeight,
      params.challengeFailUsdcFeeNum,
      params.challengeFailUsdcFeeDen,
      params.challengeSuccessKassFeeNum,
      params.challengeSuccessKassFeeDen,
      params.stakeFloorEmaThreshold,
      params.stakeFloorEmaCap,
      params.stakeFloorMax,
    ];
    const payload: number[] = [];
    for (const v of ordered) payload.push(...leU64(v));
    expect(payload.length).toBe(200);

    // The encoder helper must match the independent packing.
    expect(encodeSetConfigParams(params)).toEqual(new Uint8Array(payload));

    const ix = await setConfig({ authority: AUTHORITY, params });
    expect(ix.data).toEqual(bytesOf(Ix.SetConfig, payload));
    expect(ix.data.length).toBe(1 + 200);

    const protocol = await pda.protocol();
    expect(metaTriples(ix.keys)).toEqual([
      [protocol.address.toString(), false, true],
      [AUTHORITY, true, false],
    ]);
  });

  it("resolveDeadend: option u8, protocol(ro) + oracle(w) + authority(ro signer)", async () => {
    const option = 1;
    const ix = await resolveDeadend({ oracle: ORACLE, authority: AUTHORITY, option });
    expect(ix.data).toEqual(bytesOf(Ix.ResolveDeadend, [option]));

    const protocol = await pda.protocol();
    expect(metaTriples(ix.keys)).toEqual([
      [protocol.address.toString(), false, false],
      [ORACLE, false, true],
      [AUTHORITY, true, false],
    ]);
  });

  it("kassPrice: empty payload, protocol(ro) + kass_dao(ro)", async () => {
    const ix = await kassPrice({ kassDao: KASS_DAO });
    expect(ix.data).toEqual(bytesOf(Ix.KassPrice));

    const protocol = await pda.protocol();
    expect(metaTriples(ix.keys)).toEqual([
      [protocol.address.toString(), false, false],
      [KASS_DAO, false, false],
    ]);
  });

  it("respects a programId override", async () => {
    const other = new Address("Vote111111111111111111111111111111111111111");
    const ix = await advancePhase({ oracle: ORACLE, programId: other });
    expect(ix.programId.toString()).toBe(other.toString());
  });
});

// ---------------------------------------------------------------------------
// litesvm acceptance: build init_protocol VIA THE SDK and prove the real
// program accepts it (account order + roles correct).
// ---------------------------------------------------------------------------
const here = dirname(fileURLToPath(import.meta.url));
const SO_PATH = resolve(here, "../../target/deploy/kassandra_program.so");

describe("D3a litesvm acceptance — initProtocol via the SDK", () => {
  it("the real program accepts the SDK-built init_protocol", async () => {
    if (!existsSync(SO_PATH)) {
      throw new Error(`Missing program artifact at ${SO_PATH}. Run \`just build\` first.`);
    }
    const svm = new LiteSVM();
    svm.addProgramFromFile(address(PROGRAM_ID), SO_PATH);

    const payer = await Keypair.generate();
    svm.airdrop(payer.address, lamports(10_000_000_000n));

    // Fabricate two SPL-token-program-owned mint accounts (init_protocol checks
    // the mints are token-program owned). Minimal 82-byte Mint buffers suffice.
    const kassMint = await Keypair.generate();
    const usdcMint = await Keypair.generate();
    for (const m of [kassMint, usdcMint]) {
      svm.setAccount({
        address: m.address,
        data: new Uint8Array(82),
        executable: false,
        lamports: lamports(1_000_000_000n),
        programAddress: address(TOKEN_PROGRAM_ID.toString()),
        space: 82n,
      });
    }

    const ix = await initProtocol({
      admin: payer.publicKey,
      kassMint: kassMint.publicKey,
      usdcMint: usdcMint.publicKey,
    });

    const tx = new Transaction();
    tx.feePayer = payer.publicKey;
    tx.recentBlockhash = svm.latestBlockhash();
    tx.add(ix);
    await tx.sign(payer);

    const result = svm.sendTransaction(await toLiteSvmTransaction(tx));
    expect(result).toBeInstanceOf(TransactionMetadata);

    // The Protocol PDA now exists and is program-owned.
    const protocol = await pda.protocol();
    const acct = svm.getAccount(address(protocol.address.toString()));
    if (!acct || !acct.exists) throw new Error("protocol account not found after init");
    expect(acct.programAddress.toString()).toBe(KASSANDRA_PROGRAM_ID.toString());
  });
});
