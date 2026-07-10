/**
 * W2 — litesvm END-TO-END proof: a GENUINE runner payload, wired through the
 * SDK bridge, is ACCEPTED by the REAL program, and the resulting on-chain
 * `AiClaim` is byte-identical to the runner's metadata.
 *
 * The full path proven here:
 *   runner-output.json (genuine Rust `run` output, committed in W1)
 *     → `submitAiClaimFromRunner` (the SDK bridge + its byte-parity guard)
 *       → `toLiteSvmTransaction` → the REAL `kassandra_oracles_program.so`
 *         → on-chain `AiClaim` decoded by `decodeAiClaim`.
 *
 * --- Seeding the precondition (vs. driving it live) ---
 * `submit_ai_claim` (processor/submit_ai_claim.rs) only requires that the oracle
 * is a program-owned `Oracle` in `Phase::AiClaim` with the window still open,
 * that the proposer is a program-owned `Proposer` whose `oracle == the oracle`
 * and `authority == the signer` and who is NOT disqualified, and that
 * `option < oracle.options_count`. It does NOT re-derive the Oracle/Proposer
 * addresses (the `[b"oracle", nonce]` / `[b"proposer", oracle, authority]` PDA
 * derivations are only enforced at create/propose time). So we SEED that exact
 * precondition directly with `svm.setAccount` — writing program-owned `Oracle`
 * + `Proposer` bytes (mirroring the Rust harness `seed_disputed_oracle` +
 * `set_phase`, layout per `state.rs`) — rather than driving the whole
 * create → propose×2 → finalize_proposals → submit_fact → finalize_facts →
 * AiClaim-phase pipeline through the SDK. Driving that pipeline live is heavy
 * and is already COVERED BY THE RUST SUITE; here we isolate the runner→bridge→
 * program→AiClaim leg. We seed the Oracle/Proposer at the EXACT addresses the
 * runner fixture's `claim_pda_seeds` names, so the bridge's PDA cross-check
 * passes and the AiClaim PDA `[b"claim", oracle, proposer]` is the runner's.
 */
import { existsSync, readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { address, lamports } from "@solana/kit";
import { Address, Keypair, Transaction, type TransactionInstruction } from "@solana/web3.js";
import { FailedTransactionMetadata, LiteSVM, TransactionMetadata } from "litesvm";
import { beforeAll, describe, expect, it } from "vitest";

import { decodeAiClaim } from "../src/accounts/index.js";
import { AccountType, ACCOUNT_SIZES, CLAIM_OPTION_NONE, KASSANDRA_PROGRAM_ID, Phase } from "../src/constants.js";
import { toLiteSvmTransaction } from "../src/litesvm-interop.js";
import * as pda from "../src/pda.js";
import { submitAiClaimFromRunner, type RunnerOutput } from "../src/runner-bridge.js";

const PROGRAM_ID = KASSANDRA_PROGRAM_ID.toString();

const here = dirname(fileURLToPath(import.meta.url));
const SO_PATH = resolve(here, "../../../../target/deploy/kassandra_oracles_program.so");
const FIXTURE_PATH = resolve(here, "fixtures/runner-output.json");

/** Write a program-owned account into litesvm at `key` holding `data`. */
function putProgramAccount(svm: LiteSVM, key: Address, data: Uint8Array): void {
  svm.setAccount({
    address: address(key.toString()),
    data,
    executable: false,
    lamports: lamports(svm.minimumBalanceForRentExemption(BigInt(data.length))),
    programAddress: address(PROGRAM_ID),
    space: BigInt(data.length),
  });
}

/**
 * Fabricate the 392-byte `Oracle` Pod bytes (state.rs layout, offsets per the
 * SDK `decodeOracle`) in `Phase::AiClaim` with `options_count` options and the
 * phase window open until `phaseEndsAt`. Mirrors the Rust harness seeding: only
 * the fields `submit_ai_claim` reads need be meaningful; the rest stay zeroed.
 */
function oracleBytes(opts: {
  optionsCount: number;
  phaseEndsAt: bigint;
  proposerCount: number;
}): Uint8Array {
  const data = new Uint8Array(ACCOUNT_SIZES.Oracle);
  const dv = new DataView(data.buffer);
  data[0] = AccountType.Oracle; // account_type @0
  // creator/kass_mint/usdc_mint/stake_vault (@8/@40/@72/@104) — unread here, left zero.
  dv.setBigInt64(136, 0n, true); // deadline — unread by submit_ai_claim
  dv.setBigInt64(144, opts.phaseEndsAt, true); // phase_ends_at — require_before_end gate
  dv.setBigInt64(152, 600n, true); // twap_window
  data[160] = opts.optionsCount; // options_count @160
  data[161] = Phase.AiClaim; // phase @161
  dv.setUint16(162, opts.proposerCount, true); // proposer_count @162
  dv.setUint16(164, opts.proposerCount, true); // surviving_count @164
  return data;
}

/**
 * Fabricate the 96-byte `Proposer` Pod bytes (state.rs layout, offsets per the
 * SDK `decodeProposer`) bound to `oracle` and controlled by `authority`, with
 * `claim_option == CLAIM_OPTION_NONE` and NOT disqualified — exactly what
 * `submit_ai_claim` requires of a fresh, locked-in proposer.
 */
function proposerBytes(opts: {
  oracle: Address;
  authority: Address;
  originalOption: number;
}): Uint8Array {
  const data = new Uint8Array(ACCOUNT_SIZES.Proposer);
  const dv = new DataView(data.buffer);
  data[0] = AccountType.Proposer; // account_type @0
  data.set(opts.oracle.toBytes(), 8); // oracle @8
  data.set(opts.authority.toBytes(), 40); // authority @40
  dv.setBigUint64(72, 0n, true); // bond @72 — unread by submit_ai_claim
  data[80] = opts.originalOption; // original_option @80
  data[81] = CLAIM_OPTION_NONE; // claim_option @81 (no claim yet)
  // disqualified/slashed/flipped/ai_finalized (@82..@86) — all 0 (zeroed).
  return data;
}

describe("W2 litesvm proof — genuine runner payload accepted by submit_ai_claim", () => {
  beforeAll(() => {
    if (!existsSync(SO_PATH)) {
      throw new Error(
        `Missing program artifact at ${SO_PATH}. Run \`just build\` from the repo root first.`,
      );
    }
  });

  it("seeds AiClaim-phase oracle+proposer, submits the bridge-built ix, and the on-chain AiClaim matches the fixture", async () => {
    const fixture: RunnerOutput = JSON.parse(readFileSync(FIXTURE_PATH, "utf8"));
    expect(fixture.claim_pda_seeds).toBeDefined();
    const oracle = new Address(fixture.claim_pda_seeds!.oracle);
    const proposer = new Address(fixture.claim_pda_seeds!.proposer);

    const svm = new LiteSVM();
    svm.addProgramFromFile(address(PROGRAM_ID), SO_PATH);

    // Fee payer + the proposer's human authority (signs submit_ai_claim, funds
    // the AiClaim rent). authority must equal proposer.authority.
    const payer = await Keypair.generate();
    svm.airdrop(payer.address, lamports(100_000_000_000n));
    const authority = await Keypair.generate();
    svm.airdrop(authority.address, lamports(10_000_000_000n));

    // --- Seed the precondition directly (covered-by-the-Rust-suite upstream) ---
    const baseUnix = svm.getClock().unixTimestamp;
    putProgramAccount(
      svm,
      oracle,
      oracleBytes({ optionsCount: 2, phaseEndsAt: baseUnix + 100_000n, proposerCount: 1 }),
    );
    putProgramAccount(
      svm,
      proposer,
      proposerBytes({ oracle, authority: authority.publicKey, originalOption: 0 }),
    );

    // --- Build via the bridge (parity guard + PDA cross-check run here) --------
    const ix: TransactionInstruction = await submitAiClaimFromRunner(fixture, {
      oracle,
      proposer,
      authority: authority.publicKey,
    });

    // --- Sign, bridge, submit to the REAL program -----------------------------
    const tx = new Transaction();
    tx.feePayer = payer.publicKey;
    tx.recentBlockhash = svm.latestBlockhash();
    tx.add(ix);
    await tx.sign(payer, authority);
    const result = svm.sendTransaction(await toLiteSvmTransaction(tx));

    // --- Assert ACCEPTANCE (surface the real program error if it failed) ------
    if (result instanceof FailedTransactionMetadata) {
      throw new Error(`submit_ai_claim was rejected by the real program: ${result.toString()}`);
    }
    expect(result).toBeInstanceOf(TransactionMetadata);

    // --- Assert the on-chain AiClaim matches the runner fixture ---------------
    const claimPda = await pda.aiClaim(oracle, proposer);
    const acct = svm.getAccount(address(claimPda.address.toString()));
    if (!acct || !acct.exists) throw new Error(`AiClaim PDA ${claimPda.address} was not created`);
    const claimData = acct.data;
    expect(claimData.length).toBe(ACCOUNT_SIZES.AiClaim); // 208 bytes

    const claim = decodeAiClaim(claimData);
    expect(claim.accountType).toBe(AccountType.AiClaim);
    expect(claim.oracle.toString()).toBe(oracle.toString());
    expect(claim.proposer.toString()).toBe(proposer.toString());
    // The runner's metadata is now on-chain, byte-for-byte.
    expect(Buffer.from(claim.modelId).toString("hex")).toBe(fixture.model_id_hex);
    expect(Buffer.from(claim.paramsHash).toString("hex")).toBe(fixture.params_hash_hex);
    expect(Buffer.from(claim.ioHash).toString("hex")).toBe(fixture.io_hash_hex);
    expect(claim.option).toBe(fixture.option_index); // 0
    // The submit-time authority was recorded on the claim.
    expect(claim.authority.toString()).toBe(authority.publicKey.toString());
  });
});
