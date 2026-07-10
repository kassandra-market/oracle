/**
 * F2b — FUTARCHY → METEORA DAO-TREASURY FEE-COLLECTION E2E (GATED, FORKED
 * MAINNET). A documented-partial with a REAL deployed-verification proof.
 *
 * ── Why a documented-partial (the full live sweep is NOT drivable on a fork) ──
 * The futarchy `collect_meteora_damm_fees` handler deployed on mainnet is the
 * `production` build. Its `validate()` (run via `#[access_control]` BEFORE the
 * handler body — futarchy `lib.rs:157`) enforces
 *
 *     #[cfg(feature = "production")]
 *     require_keys_eq!(admin, metadao_admin::ID, InvalidAdmin);  // admin == tSTp6B6k…
 *
 * `metadao_admin` (`tSTp6B6kE9o6ZaTmHm2ZwnJBBtgd3x112tapxFhmBEQ`) is a
 * MetaDAO-controlled key whose secret we do not have, and the handler then stages
 * the actual cp-amm `claim_position_fee` through the DAO's Squads multisig
 * (`vault_transaction_create → proposal_create → proposal_approve →
 * vault_transaction_execute`, all internal CPIs). The end-to-end sweep therefore
 * CANNOT be driven on a fork without forging a MetaDAO-controlled signature, so
 * the live fee collection is DEFERRED to a real MetaDAO-admin context.
 *
 * ── What this test DOES prove (the valuable, genuine partial) ─────────────────
 * `#[access_control(validate())]` runs only AFTER Anchor's `try_accounts` has
 * ACCEPTED the full 27-account layout (order/count/roles) — deserializing +
 * constraint-checking the TYPED accounts among them (the `Account<Dao>` +
 * base/quote-mint `constraint`s, the Squads `Multisig` PDA `seeds`, the
 * `associated_token` fee-recipient ATAs, the `pool_authority` / permissionless
 * `address` constraints, the `Program<…>` ids, and the `#[event_cpi]` tail). So a
 * rejection SPECIFICALLY at `InvalidAdmin` (Anchor custom error **6020**) — and
 * NOT at an earlier `ConstraintSeeds`/`AccountNotInitialized`/`ConstraintAddress`/
 * `ConstraintRaw` error — is a live PROOF that the F2a builder's 27-account wire
 * format is ACCEPTED (correct order / roles / typed-account & PDA-seed & address
 * constraints) on the DEPLOYED futarchy binary. A wire-format bug (wrong order /
 * role / PDA seeds / missing account) would fail EARLIER with a different error,
 * never reaching the admin gate.
 *
 * Arms:
 *   1. REACH-THE-ADMIN-GATE — real `initialize_dao` (creates a genuine `Dao` +
 *      Squads multisig/vault on the fork) → fabricate the two fee-recipient ATAs
 *      (owned by the MetaDAO vault `6awyHMsh…`) → build `collectMeteoraDammFees`
 *      with a STAND-IN admin (our payer, ≠ the production admin) + the real public
 *      permissionless signer → submit to the REAL deployed futarchy → ASSERT it
 *      is rejected at `InvalidAdmin` (6020), proving the 27-account layout is
 *      accepted by `try_accounts` on the deployed binary.
 *   2. REAL-ACCOUNT CROSS-VERIFICATION — clone a REAL mainnet futarchy `Dao`,
 *      derive its Squads multisig/vault via the SAME PDA derivers the builder
 *      uses, and assert the derivations match the Dao's own recorded
 *      `squads_multisig`/`squads_multisig_vault` (present in the deployed bytes)
 *      and that the derived multisig EXISTS on-chain owned by Squads.
 *
 * GATING: `KASSANDRA_E2E=1`; skips (not fails) when surfpool/.so absent. Forks
 * mainnet → needs network + is slower.
 */
import {
  Address,
  ComputeBudgetProgram,
  Keypair,
  Transaction,
  TransactionInstruction,
} from "@solana/web3.js";
import { afterAll, beforeAll, describe, expect, it } from "vitest";

import { TOKEN_PROGRAM_ID } from "../../src/constants.js";
import * as futarchy from "../../src/futarchy/index.js";

import { SurfpoolHarness, mintBytes, surfpoolReady, toHex, tokenAccountBytes } from "./harness.js";

const ENABLED = process.env.KASSANDRA_E2E === "1" && surfpoolReady();

const MAINNET_RPC = "https://api.mainnet-beta.solana.com";

/** Anchor custom-error base + `FutarchyError::InvalidAdmin` (variant index 20). */
const ANCHOR_ERROR_BASE = 6000;
const INVALID_ADMIN = ANCHOR_ERROR_BASE + 20; // 6020

/**
 * A REAL mainnet futarchy `Dao` (`FUTAREL…`-owned, PoolState::Spot) used for the
 * cross-verification arm — its recorded `squads_multisig`/`squads_multisig_vault`
 * are cross-checked against the builder's PDA derivations.
 */
const REAL_DAO = new Address("1PAwyDkWNFCcR96GhEReXHJBv3YEFVazCaQgNicVuKv");

/** MetaDAO's PUBLIC permissionless multisig member (futarchy
 * `sdk/permissionless-account.json` → EP3SoC2…) — a fixed Initiate|Execute member
 * whose secret is published by design. The collect handler requires it as a
 * signer (`address = permissionless_account::id()`), checked in `try_accounts`. */
const PERMISSIONLESS_SECRET = Uint8Array.from([
  249, 158, 188, 171, 243, 143, 1, 48, 87, 243, 209, 153, 144, 106, 23, 88, 161, 209, 65, 217,
  199, 121, 0, 250, 3, 203, 133, 138, 141, 112, 243, 38, 198, 205, 120, 222, 160, 224, 151, 190,
  84, 254, 127, 178, 224, 195, 130, 243, 145, 73, 20, 91, 9, 69, 222, 184, 23, 1, 2, 196, 202,
  206, 153, 192,
]);

interface Fixture {
  harness: SurfpoolHarness;
  payer: Keypair;
  kassMint: Keypair;
  usdcMint: Keypair;
}

describe.skipIf(!ENABLED)("surfpool futarchy→Meteora treasury fee-collection on FORKED mainnet (F2b)", () => {
  let f: Fixture;

  beforeAll(async () => {
    const harness = await SurfpoolHarness.start({
      port: 8923,
      fork: "mainnet",
      readyTimeoutMs: 60_000,
    });
    const payer = await Keypair.generate();
    await harness.airdrop(payer.publicKey.toString(), 1_000_000_000_000);

    // Real KASS (9dp) + USDC (MUST be 6dp — initialize_dao `mint::decimals = 6`).
    const kassMint = await Keypair.generate();
    const usdcMint = await Keypair.generate();
    await harness.setAccount(kassMint.publicKey.toString(), {
      lamports: 1_000_000_000,
      owner: TOKEN_PROGRAM_ID.toString(),
      executable: false,
      data: toHex(mintBytes(payer.publicKey.toBytes(), 0n, 9)),
    });
    await harness.setAccount(usdcMint.publicKey.toString(), {
      lamports: 1_000_000_000,
      owner: TOKEN_PROGRAM_ID.toString(),
      executable: false,
      data: toHex(mintBytes(payer.publicKey.toBytes(), 0n, 6)),
    });

    // Warm the fork's copies of the programs the collect ix references as
    // `Program<…>` (validated in try_accounts): the cp-amm + Squads v4.
    await harness.connection.getAccountInfo(futarchy.METEORA_DAMM_V2_ID);
    await harness.connection.getAccountInfo(futarchy.SQUADS_V4_ID);

    f = { harness, payer, kassMint, usdcMint };
  }, 120_000);

  afterAll(async () => {
    await f?.harness.teardown();
  });

  it("REAL mainnet Dao cross-verifies the Squads multisig/vault PDA derivations", async () => {
    const daoData = await fetchMainnetAccount(REAL_DAO);
    // Owned by the deployed futarchy program, with the `Dao` account discriminator.
    const info = await fetchMainnetAccountInfo(REAL_DAO);
    expect(info.owner).toBe(futarchy.FUTARCHY_ID.toString());
    expect(daoData.slice(0, 8)).toEqual(futarchy.ACCOUNT_DISC.dao);

    // Derive via the SAME derivers the builder uses.
    const multisig = (await futarchy.pda.squadsMultisig(REAL_DAO)).address;
    const vault = (await futarchy.pda.squadsVault(multisig, 0)).address;

    // The Dao struct records both — assert their 32-byte keys appear in the
    // genuine deployed bytes (robust to the variable PoolState offset).
    expect(contains(daoData, multisig.toBytes())).toBe(true);
    expect(contains(daoData, vault.toBytes())).toBe(true);

    // The derived multisig EXISTS on-chain, owned by the Squads v4 program.
    const msInfo = await fetchMainnetAccountInfo(multisig);
    expect(msInfo.owner).toBe(futarchy.SQUADS_V4_ID.toString());
  }, 90_000);

  it("REACH-THE-ADMIN-GATE: collectMeteoraDammFees 27-account layout accepted by the DEPLOYED futarchy, rejected at InvalidAdmin (6020)", async () => {
    // --- (1) real initialize_dao → genuine Dao + Squads multisig/vault --------
    // Squads ProgramConfig.treasury is NOT a PDA — read it live (treasury @48).
    const programConfig = (await futarchy.pda.squadsProgramConfig()).address;
    const pcInfo = await f.harness.connection.getAccountInfo(programConfig);
    expect(pcInfo, "Squads ProgramConfig not on the fork").not.toBeNull();
    const treasury = new Address(pcInfo!.data.slice(48, 80));

    const nonce = 1n;
    const dao = (await futarchy.pda.dao(f.payer.publicKey, nonce)).address;
    const multisig = (await futarchy.pda.squadsMultisig(dao)).address;
    const vault = (await futarchy.pda.squadsVault(multisig, 0)).address;

    await sendIx(
      f,
      await futarchy.initializeDao({
        daoCreator: f.payer.publicKey,
        payer: f.payer.publicKey,
        baseMint: f.kassMint.publicKey,
        quoteMint: f.usdcMint.publicKey,
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
      [],
      1_400_000,
    );

    // The Dao + its Squads multisig really exist (created by the deployed program).
    const daoInfo = await f.harness.connection.getAccountInfo(dao);
    expect(daoInfo!.owner.toString()).toBe(futarchy.FUTARCHY_ID.toString());
    const msInfo = await f.harness.connection.getAccountInfo(multisig);
    expect(msInfo!.owner.toString()).toBe(futarchy.SQUADS_V4_ID.toString());

    // --- (2) fabricate the two fee-recipient ATAs (owned by 6awyHMsh…) --------
    // The collect handler's `token_{a,b}_account` are `Account<TokenAccount>` with
    // `associated_token::authority = metadao_multisig_vault::ID` — they MUST exist
    // as canonical ATAs of the MetaDAO vault or try_accounts fails BEFORE the
    // admin gate. The builder defaults them to exactly these addresses.
    const feeA = await futarchy.ata(futarchy.METADAO_MULTISIG_VAULT, f.kassMint.publicKey);
    const feeB = await futarchy.ata(futarchy.METADAO_MULTISIG_VAULT, f.usdcMint.publicKey);
    await fabricateTokenAccount(f, feeA, f.kassMint.publicKey, futarchy.METADAO_MULTISIG_VAULT);
    await fabricateTokenAccount(f, feeB, f.usdcMint.publicKey, futarchy.METADAO_MULTISIG_VAULT);

    // --- (3) cp-amm accounts. `pool`/`position`/vaults/`nft`/`owner` are all
    // UncheckedAccounts in the futarchy handler (validated only in the inner
    // cp-amm CPI, which never runs — validate() rejects first), so deterministic
    // stand-ins are sufficient to reach the admin gate. `pool_authority` +
    // `damm_v2_program` are the only cp-amm accounts the futarchy layer checks;
    // the builder fills those from the pinned constants.
    const pool = (await Keypair.generate()).publicKey;
    const position = (await Keypair.generate()).publicKey;
    const tokenAVault = (await Keypair.generate()).publicKey;
    const tokenBVault = (await Keypair.generate()).publicKey;
    const positionNftAccount = (await Keypair.generate()).publicKey;
    const positionOwner = vault; // the position owner is usually the DAO squads vault

    // --- (4) build the F2a builder with a STAND-IN admin (≠ production admin) --
    const permissionless = await Keypair.fromSecretKey(PERMISSIONLESS_SECRET);
    const ix = await futarchy.collectMeteoraDammFees({
      dao,
      admin: f.payer.publicKey, // STAND-IN — deliberately NOT tSTp6B6k…
      transactionIndex: 1n, // fresh multisig.transaction_index (0) + 1
      pool,
      position,
      tokenAVault,
      tokenBVault,
      tokenAMint: f.kassMint.publicKey, // == dao.base_mint (dao constraint)
      tokenBMint: f.usdcMint.publicKey, // == dao.quote_mint
      positionNftAccount,
      owner: positionOwner,
      // permissionlessAccount + tokenA/BAccount default to EP3SoC2… + the ATAs.
    });
    expect(ix.keys.length).toBe(27);
    // The stand-in admin sits at meta index 1 and is our payer (the fee payer).
    expect(ix.keys[1].pubkey.toString()).toBe(f.payer.publicKey.toString());
    expect(ix.keys[1].pubkey.toString()).not.toBe(futarchy.METADAO_ADMIN.toString());

    // --- (5) submit to the REAL deployed futarchy; capture the rejection ------
    const { err, logs } = await simulate(f, ix, [permissionless]);

    // Structured proof: rejected at instruction 0 with Anchor custom 6020.
    const custom = (err as { InstructionError?: [number, { Custom?: number }] })?.InstructionError;
    expect(custom, `expected an InstructionError, got ${JSON.stringify(err)}`).toBeTruthy();
    expect(custom![0]).toBe(0);
    expect(
      custom![1]?.Custom,
      `rejected with ${JSON.stringify(custom![1])} — expected InvalidAdmin (${INVALID_ADMIN}). ` +
        `A code other than ${INVALID_ADMIN} means an EARLIER (account-layout) failure, not the admin gate.`,
    ).toBe(INVALID_ADMIN);

    // Log-level proof: the deployed futarchy threw InvalidAdmin in validate(),
    // which is only reachable AFTER all 27 accounts deserialized in try_accounts.
    const joined = logs.join("\n");
    expect(joined).toContain(futarchy.FUTARCHY_ID.toString());
    expect(joined).toMatch(/InvalidAdmin|Error Number: 6020/);
    // Guard: it must NOT be an earlier account-layout rejection.
    expect(joined).not.toMatch(/ConstraintSeeds|AccountNotInitialized|ConstraintAssociated|ConstraintAddress|AccountOwnedByWrongProgram/);

    // --- (6) belt-and-braces: a real send (skipPreflight:false) is REJECTED ---
    await expect(sendIx(f, ix, [permissionless])).rejects.toThrow();
  }, 240_000);
});

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

async function sendIx(
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

/** Simulate a SINGLE-instruction tx (ix at index 0) against the real program. */
async function simulate(
  f: Fixture,
  ix: TransactionInstruction,
  signers: Keypair[] = [],
): Promise<{ err: unknown; logs: string[] }> {
  const conn = f.harness.connection;
  const tx = new Transaction();
  tx.feePayer = f.payer.publicKey;
  tx.recentBlockhash = (await conn.getLatestBlockhash()).blockhash;
  tx.add(ix);
  await tx.sign(f.payer, ...signers);
  const b64 = Buffer.from(await tx.serialize()).toString("base64");
  const res = await f.harness.rpc<{ value: { err: unknown; logs: string[] | null } }>(
    "simulateTransaction",
    [b64, { encoding: "base64", commitment: "confirmed", sigVerify: false }],
  );
  return { err: res.value.err, logs: res.value.logs ?? [] };
}

async function fabricateTokenAccount(
  f: Fixture,
  address: Address,
  mint: Address,
  owner: Address,
): Promise<void> {
  await f.harness.setAccount(address.toString(), {
    lamports: 5_000_000,
    owner: TOKEN_PROGRAM_ID.toString(),
    executable: false,
    data: toHex(tokenAccountBytes(mint.toBytes(), owner.toBytes(), 0n)),
  });
}

/** Fetch an account's raw data straight from mainnet (NOT the fork). */
async function fetchMainnetAccount(address: Address): Promise<Uint8Array> {
  return (await fetchMainnetAccountInfo(address)).data;
}

async function fetchMainnetAccountInfo(address: Address): Promise<{ owner: string; data: Uint8Array }> {
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
    result?: { value?: { data: [string, string]; owner: string } | null };
    error?: { message: string };
  };
  if (json.error) throw new Error(`mainnet getAccountInfo failed: ${json.error.message}`);
  const v = json.result?.value;
  if (!v) throw new Error(`mainnet account ${address} not found`);
  return { owner: v.owner, data: new Uint8Array(Buffer.from(v.data[0], "base64")) };
}

/** True if `needle` (32-byte key) appears anywhere in `hay`. */
function contains(hay: Uint8Array, needle: Uint8Array): boolean {
  outer: for (let i = 0; i + needle.length <= hay.length; i++) {
    for (let j = 0; j < needle.length; j++) if (hay[i + j] !== needle[j]) continue outer;
    return true;
  }
  return false;
}
