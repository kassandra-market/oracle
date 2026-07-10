/**
 * D2 — LITESVM FULL-DRIVE of the MetaDAO futarchy `collect_meteora_damm_fees`
 * instruction, past the admin gate, to COMPLETION (fees swept), via
 * `withSigverify(false)`.
 *
 * ── What this proves (and what it does NOT) ──────────────────────────────────
 * `collect_meteora_damm_fees` is **MetaDAO's protocol-rake operation**: under the
 * `production` build its `validate()` hard-requires `admin == metadao_admin::ID`
 * (`tSTp6B6k…`, a MetaDAO-controlled keeper) and it sweeps the DAO's Meteora LP
 * fees into MetaDAO's OWN multisig vault (`6awyHMsh…`). **Kassandra does NOT call
 * it** — the DAO collects its OWN Meteora treasury fees admin-free via its Squads
 * vault (the D1 path, `sdks/oracles/ts/test/surfpool/dao-meteora-treasury-e2e.test.ts`).
 *
 * This test exists ONLY to prove the F2a 27-account wire format
 * (`futarchy.collectMeteoraDammFees`) drives the DEPLOYED handler END-TO-END:
 *   - F2a (`test/futarchy.test.ts`) proves the byte/meta layout offline.
 *   - F2b (surfpool, `futarchy-meteora-treasury-e2e.test.ts`) reaches the admin
 *     gate on the DEPLOYED binary (proving `try_accounts` ACCEPTS the 27-account
 *     layout) but CANNOT complete — surfpool cannot forge the MetaDAO admin sig.
 *   - D2 (THIS test, litesvm) uses `svm.withSigverify(false)` — the one thing
 *     surfpool cannot do — to present the REAL admin `tSTp6B6k…` as a required
 *     (but unsigned, zero-signature) signer, so the handler runs PAST the admin
 *     gate: the internal Squads `vault_transaction_create → proposal_create →
 *     proposal_approve → vault_transaction_execute` chain and the inner cp-amm
 *     `claim_position_fee` CPI all execute, and the accrued LP fee is swept to the
 *     MetaDAO vault's ATAs. We assert the recipient ATA balance ROSE by the fee.
 *
 * The `withSigverify(false)` bypass is a TEST-ONLY device: the production program
 * still requires the real MetaDAO keeper's signature. This is a completeness proof
 * of MetaDAO's op, not a Kassandra dependency.
 *
 * ── How litesvm hosts the 3 real deployed programs ───────────────────────────
 * All three deployed mainnet programs are loaded from committed `.so` fixtures
 * (dumped via `solana program dump -u m <id> <file.so>`; see
 * `test/fixtures/programs/README` note below):
 *   - futarchy   FUTAREL…  (1.24 MB)
 *   - cp-amm     cpamd…     (2.17 MB)
 *   - Squads v4  SQDS4ep6…  (1.47 MB)
 * plus the real Squads `ProgramConfig` + a real public cp-amm `Config` (cloned as
 * account fixtures). litesvm's default SPL Token / Token-2022 / ATA builtins host
 * the rest. State is built by running the REAL instructions in litesvm:
 * `initialize_dao` (→ Dao + Squads multisig/vault, via the futarchy→Squads
 * `multisig_create_v2` CPI), cp-amm `initialize_pool` (first position owned by the
 * DAO's Squads vault) + `swap`s to accrue a real LP fee.
 *
 * DEFAULT vs GATED: the `.so` fixtures are large (~4.9 MB total), so this test is
 * GATED behind `KASSANDRA_LITESVM_PROGRAMS=1` (and skips when the fixtures are
 * absent). Re-dump them with the commands in
 * `test/fixtures/programs/README.md`.
 */
import { existsSync, readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { address, getTransactionDecoder, lamports } from "@solana/kit";
import {
  Address,
  ComputeBudgetProgram,
  Keypair,
  Transaction,
  type TransactionInstruction,
} from "@solana/web3.js";
import { Clock, FailedTransactionMetadata, LiteSVM, TransactionMetadata } from "litesvm";
import { describe, expect, it } from "vitest";

import { TOKEN_PROGRAM_ID } from "../src/constants.js";
import * as futarchy from "../src/futarchy/index.js";
import { meteora } from "../src/index.js";

const here = dirname(fileURLToPath(import.meta.url));
const FIX = resolve(here, "fixtures/programs");
const FUTARCHY_SO = resolve(FIX, "futarchy.so");
const CPAMM_SO = resolve(FIX, "cp_amm.so");
const SQUADS_SO = resolve(FIX, "squads_v4.so");
const PROGRAM_CONFIG_FIX = resolve(FIX, "squads-program-config.json");
const CP_AMM_CONFIG_FIX = resolve(FIX, "cp-amm-config.json");

const ALL_FIXTURES = [FUTARCHY_SO, CPAMM_SO, SQUADS_SO, PROGRAM_CONFIG_FIX, CP_AMM_CONFIG_FIX];
const ENABLED = process.env.KASSANDRA_LITESVM_PROGRAMS === "1" && ALL_FIXTURES.every(existsSync);

/** The PUBLIC permissionless multisig member secret (futarchy publishes it by
 * design; F2b uses the same). The collect handler requires it as a signer. */
const PERMISSIONLESS_SECRET = Uint8Array.from([
  249, 158, 188, 171, 243, 143, 1, 48, 87, 243, 209, 153, 144, 106, 23, 88, 161, 209, 65, 217,
  199, 121, 0, 250, 3, 203, 133, 138, 141, 112, 243, 38, 198, 205, 120, 222, 160, 224, 151, 190,
  84, 254, 127, 178, 224, 195, 130, 243, 145, 73, 20, 91, 9, 69, 222, 184, 23, 1, 2, 196, 202,
  206, 153, 192,
]);

// Full-range price bounds baked into the public cp-amm config (Q64.64) — from M2.
const SQRT_PRICE_INIT = 1n << 64n; // price 1.0
const INIT_LIQUIDITY = 1_000_000_000n * (1n << 64n);
const U64_MAX = (1n << 64n) - 1n;

// --- SPL layout fabrication (mirrors test/e2e.test.ts) ---
const MINT_LEN = 82;
const TOKEN_ACCOUNT_LEN = 165;

function mintBytes(authority: Address, supply: bigint, decimals: number): Uint8Array {
  const data = new Uint8Array(MINT_LEN);
  const dv = new DataView(data.buffer);
  dv.setUint32(0, 1, true);
  data.set(authority.toBytes(), 4);
  dv.setBigUint64(36, supply, true);
  data[44] = decimals;
  data[45] = 1;
  return data;
}

function tokenAccountBytes(mint: Address, owner: Address, amount: bigint): Uint8Array {
  const data = new Uint8Array(TOKEN_ACCOUNT_LEN);
  data.set(mint.toBytes(), 0);
  data.set(owner.toBytes(), 32);
  new DataView(data.buffer).setBigUint64(64, amount, true);
  data[108] = 1;
  return data;
}

function putSplAccount(svm: LiteSVM, key: Address, data: Uint8Array): void {
  svm.setAccount({
    address: address(key.toString()),
    data,
    executable: false,
    lamports: lamports(svm.minimumBalanceForRentExemption(BigInt(data.length))),
    programAddress: address(TOKEN_PROGRAM_ID.toString()),
    space: BigInt(data.length),
  });
}

function tokenBalance(svm: LiteSVM, key: Address): bigint {
  const acct = svm.getAccount(address(key.toString()));
  if (!acct || !acct.exists) throw new Error(`token account ${key} not found`);
  return new DataView(acct.data.buffer, acct.data.byteOffset, acct.data.length).getBigUint64(64, true);
}

function fetchData(svm: LiteSVM, key: Address): Uint8Array {
  const acct = svm.getAccount(address(key.toString()));
  if (!acct || !acct.exists) throw new Error(`account ${key} not found`);
  return acct.data;
}

function failLogs(r: FailedTransactionMetadata): string {
  try {
    return (r.meta().logs() as string[]).join("\n");
  } catch {
    return r.toString();
  }
}

/** Sign with the keys we hold + inject zero signatures for any unsigned required
 * signers (e.g. the MetaDAO admin under withSigverify(false)), serialize WITHOUT
 * verification, then decode into the kit Transaction litesvm accepts. */
async function bridgeUnverified(tx: Transaction, signers: Keypair[], unsigned: Address[]) {
  await tx.partialSign(...signers);
  for (const u of unsigned) tx.addSignature(u, new Uint8Array(64));
  const wire = await tx.serialize({ requireAllSignatures: false, verifySignatures: false });
  return getTransactionDecoder().decode(wire);
}

async function submit(
  svm: LiteSVM,
  payer: Keypair,
  ix: TransactionInstruction,
  signers: Keypair[] = [],
): Promise<TransactionMetadata | FailedTransactionMetadata> {
  const tx = new Transaction();
  tx.feePayer = payer.publicKey;
  tx.recentBlockhash = svm.latestBlockhash();
  tx.add(ix);
  const decoded = await bridgeUnverified(tx, [payer, ...signers], []);
  return svm.sendTransaction(decoded);
}

function expectOk(r: TransactionMetadata | FailedTransactionMetadata, what: string): TransactionMetadata {
  if (r instanceof FailedTransactionMetadata) {
    throw new Error(`${what} failed:\n${failLogs(r)}`);
  }
  return r;
}

function loadPrograms(svm: LiteSVM): void {
  svm.addProgramFromFile(address(futarchy.FUTARCHY_ID.toString()), FUTARCHY_SO);
  svm.addProgramFromFile(address(futarchy.METEORA_DAMM_V2_ID.toString()), CPAMM_SO);
  svm.addProgramFromFile(address(futarchy.SQUADS_V4_ID.toString()), SQUADS_SO);
}

function setAccountFromFixture(svm: LiteSVM, path: string): Address {
  const fix = JSON.parse(readFileSync(path, "utf8")) as {
    pubkey: string;
    dataB64: string;
    owner: string;
    lamports: number;
  };
  const data = new Uint8Array(Buffer.from(fix.dataB64, "base64"));
  svm.setAccount({
    address: address(fix.pubkey),
    data,
    executable: false,
    lamports: lamports(BigInt(fix.lamports)),
    programAddress: address(fix.owner),
    space: BigInt(data.length),
  });
  return new Address(fix.pubkey);
}

describe.skipIf(!ENABLED)("D2 litesvm full-drive of MetaDAO collect_meteora_damm_fees (sigverify bypass)", () => {
  it("drives the deployed handler PAST the admin gate to COMPLETION: fee swept to the MetaDAO vault ATAs", async () => {
    const svm = new LiteSVM();
    svm.withSigverify(false); // TEST-ONLY: lets the real MetaDAO admin be an unsigned required signer.
    loadPrograms(svm);

    // Set a realistic mainnet-like clock: cp-amm's fee scheduler / pool activation
    // reads Clock (slot + unix_timestamp); litesvm's default (near-zero) makes the
    // fee math overflow. Match a plausible mainnet slot/timestamp.
    {
      const c = svm.getClock();
      svm.setClock(new Clock(330_000_000n, c.epochStartTimestamp, c.epoch, c.leaderScheduleEpoch, 1_735_000_000n));
    }

    // --- Squads ProgramConfig + cp-amm public Config (cloned real accounts) ----
    const pcFix = JSON.parse(readFileSync(PROGRAM_CONFIG_FIX, "utf8")) as { dataB64: string };
    const treasury = new Address(new Uint8Array(Buffer.from(pcFix.dataB64, "base64")).slice(48, 80));
    setAccountFromFixture(svm, PROGRAM_CONFIG_FIX);
    const config = setAccountFromFixture(svm, CP_AMM_CONFIG_FIX);

    const payer = await Keypair.generate();
    svm.airdrop(payer.address, lamports(1_000_000_000_000n));
    svm.airdrop(address(treasury.toString()), lamports(1_000_000_000n));
    // The collect handler uses `admin` as the rent payer for the Squads
    // vault_transaction/proposal PDAs it creates — fund it (sigverify off).
    svm.airdrop(address(futarchy.METADAO_ADMIN.toString()), lamports(1_000_000_000n));

    // KASS (base, 9dp) + USDC (quote, 6dp). initialize_dao requires quote decimals == 6.
    const kassMint = await Keypair.generate();
    const usdcMint = await Keypair.generate();
    putSplAccount(svm, kassMint.publicKey, mintBytes(payer.publicKey, 0n, 9));
    putSplAccount(svm, usdcMint.publicKey, mintBytes(payer.publicKey, 0n, 6));

    // --- (1) initialize_dao → Dao + Squads multisig/vault (futarchy→Squads CPI) -
    const nonce = 1n;
    const dao = (await futarchy.pda.dao(payer.publicKey, nonce)).address;
    const multisig = (await futarchy.pda.squadsMultisig(dao)).address;
    const vault = (await futarchy.pda.squadsVault(multisig, 0)).address;

    expectOk(
      await submit(
        svm,
        payer,
        await futarchy.initializeDao({
          daoCreator: payer.publicKey,
          payer: payer.publicKey,
          baseMint: kassMint.publicKey,
          quoteMint: usdcMint.publicKey,
          squadsProgramConfigTreasury: treasury,
          twapInitialObservation: 1_000_000_000_000n,
          twapMaxObservationChangePerUpdate: 1_000_000_000_000n,
          twapStartDelaySeconds: 0,
          minQuoteFutarchicLiquidity: 1n,
          minBaseFutarchicLiquidity: 1n,
          baseToStake: 0n,
          passThresholdBps: 0,
          secondsPerProposal: 86_400,
          nonce,
        }),
      ),
      "initialize_dao",
    );
    expect(svm.getAccount(address(dao.toString()))?.exists).toBe(true);
    const msAcct = svm.getAccount(address(multisig.toString()));
    expect(msAcct.exists).toBe(true);
    if (msAcct.exists) {
      expect(msAcct.programAddress.toString()).toBe(futarchy.SQUADS_V4_ID.toString());
    }

    // --- (2) cp-amm pool with the FIRST position OWNED BY THE DAO's Squads vault -
    // initialize_pool mints the position NFT to `creator` (UncheckedAccount, not a
    // signer), so creator == the vault PDA makes the vault the position owner. The
    // liquidity is funded from the payer's token accounts.
    const kassMintA = kassMint.publicKey; // token A = dao.base_mint (KASS)
    const usdcMintB = usdcMint.publicKey; // token B = dao.quote_mint (USDC)
    const poolAddr = (await meteora.pda.pool(config, kassMintA, usdcMintB)).address;
    const tokenAVault = (await meteora.pda.tokenVault(kassMintA, poolAddr)).address;
    const tokenBVault = (await meteora.pda.tokenVault(usdcMintB, poolAddr)).address;

    const payerTokenA = await Keypair.generate();
    const payerTokenB = await Keypair.generate();
    putSplAccount(svm, payerTokenA.publicKey, tokenAccountBytes(kassMintA, payer.publicKey, 10n ** 18n));
    putSplAccount(svm, payerTokenB.publicKey, tokenAccountBytes(usdcMintB, payer.publicKey, 10n ** 18n));

    const posNftMint = await Keypair.generate();
    expectOk(
      await submit(
        svm,
        payer,
        await meteora.initializePool({
          creator: vault, // position owner = the DAO's Squads vault
          payer: payer.publicKey,
          positionNftMint: posNftMint.publicKey,
          config,
          tokenAMint: kassMintA,
          tokenBMint: usdcMintB,
          payerTokenA: payerTokenA.publicKey,
          payerTokenB: payerTokenB.publicKey,
          liquidity: INIT_LIQUIDITY,
          sqrtPrice: SQRT_PRICE_INIT,
        }),
        [posNftMint],
      ),
      "initialize_pool",
    );

    const positionAddr = (await meteora.pda.position(posNftMint.publicKey)).address;
    const posNftAccount = (await meteora.pda.positionNftAccount(posNftMint.publicKey)).address;
    let position = meteora.decodePosition(fetchData(svm, positionAddr));
    expect(position.pool.toString()).toBe(poolAddr.toString());
    expect(position.unlockedLiquidity).toBe(INIT_LIQUIDITY);

    // --- (3) swaps (payer) A→B to accrue a real LP fee (token-B side on this
    // public Config's collect_fee_mode, per the M2 finding). ------------------
    for (const amountIn of [100_000_000n, 200_000_000n, 200_000_000n, 200_000_000n]) {
      expectOk(
        await submit(
          svm,
          payer,
          await meteora.swap({
            pool: poolAddr,
            inputTokenAccount: payerTokenA.publicKey,
            outputTokenAccount: payerTokenB.publicKey,
            tokenAVault,
            tokenBVault,
            tokenAMint: kassMintA,
            tokenBMint: usdcMintB,
            payer: payer.publicKey,
            amountIn,
            minimumAmountOut: 0n,
          }),
        ),
        `swap(${amountIn})`,
      );
    }

    // --- (4) fabricate the MetaDAO fee-recipient ATAs (authority = 6awyHMsh…) ---
    const feeAAccount = await futarchy.ata(futarchy.METADAO_MULTISIG_VAULT, kassMintA);
    const feeBAccount = await futarchy.ata(futarchy.METADAO_MULTISIG_VAULT, usdcMintB);
    putSplAccount(svm, feeAAccount, tokenAccountBytes(kassMintA, futarchy.METADAO_MULTISIG_VAULT, 0n));
    putSplAccount(svm, feeBAccount, tokenAccountBytes(usdcMintB, futarchy.METADAO_MULTISIG_VAULT, 0n));

    const feeABefore = tokenBalance(svm, feeAAccount);
    const feeBBefore = tokenBalance(svm, feeBAccount);

    // --- (5) FULL-DRIVE collect: real admin tSTp6B6k… as an UNSIGNED signer -----
    const permissionless = await Keypair.fromSecretKey(PERMISSIONLESS_SECRET);
    const collectIx = await futarchy.collectMeteoraDammFees({
      dao,
      admin: futarchy.METADAO_ADMIN, // the REAL production admin — required signer, no key held
      transactionIndex: 1n, // multisig.transaction_index (0) + 1
      pool: poolAddr,
      position: positionAddr,
      tokenAVault,
      tokenBVault,
      tokenAMint: kassMintA,
      tokenBMint: usdcMintB,
      positionNftAccount: posNftAccount,
      owner: vault, // position owner == the DAO's Squads vault (signs the inner claim)
    });
    expect(collectIx.keys.length).toBe(27);
    expect(collectIx.keys[1].pubkey.toString()).toBe(futarchy.METADAO_ADMIN.toString());
    expect(collectIx.keys[1].isSigner).toBe(true);

    const tx = new Transaction();
    tx.feePayer = payer.publicKey;
    tx.recentBlockhash = svm.latestBlockhash();
    // The full Squads-wrap + cp-amm claim CPI chain needs well above the 200k default.
    tx.add(ComputeBudgetProgram.setComputeUnitLimit({ units: 1_400_000 }));
    tx.add(collectIx);
    // Sign with payer + the public permissionless member; inject a zero signature
    // for the real MetaDAO admin (sigverify is off, so it is never checked).
    const decoded = await bridgeUnverified(tx, [payer, permissionless], [futarchy.METADAO_ADMIN]);
    const result = svm.sendTransaction(decoded);

    // COMPLETION: the tx SUCCEEDS (drove PAST the admin gate + the full
    // Squads-wrap + cp-amm claim CPI chain).
    const meta = expectOk(result, "collect_meteora_damm_fees");
    expect(meta).toBeInstanceOf(TransactionMetadata);

    // The MetaDAO vault's fee ATAs ROSE by the swept LP fee (nonzero on token B).
    const feeAAfter = tokenBalance(svm, feeAAccount);
    const feeBAfter = tokenBalance(svm, feeBAccount);
    const sweptA = feeAAfter - feeABefore;
    const sweptB = feeBAfter - feeBBefore;
    expect(sweptA + sweptB).toBeGreaterThan(0n); // a real, nonzero sweep
    expect(sweptB).toBeGreaterThan(0n); // token-B fee side (this Config's collect_fee_mode)

    // The position's pending fees cleared (swept out).
    position = meteora.decodePosition(fetchData(svm, positionAddr));
    expect(position.feeBPending).toBe(0n);
  });
});
