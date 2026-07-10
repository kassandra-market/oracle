/**
 * W1 — runner → SDK bridge tests.
 *
 * The fixture (`fixtures/runner-output.json`) is GENUINE runner output, captured
 * via `kassandra-runner run --mock --config fixtures/runner-config.json` (zero
 * agreed facts + the deterministic mock provider → option 0). So the parity
 * assertions validate the SDK encoding against the real Rust payload, not a
 * TS-fabricated one.
 */
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

import { Ix, SYSTEM_PROGRAM_ID } from "../src/constants.js";
import * as pda from "../src/pda.js";
import { submitAiClaimFromRunner, type RunnerOutput } from "../src/runner-bridge.js";

const FIXTURE_PATH = fileURLToPath(new URL("./fixtures/runner-output.json", import.meta.url));
const RAW_FIXTURE = readFileSync(FIXTURE_PATH, "utf8");

function fixture(): RunnerOutput {
  return JSON.parse(RAW_FIXTURE) as RunnerOutput;
}

// These match the oracle/proposer the fixture's claim_pda_seeds were echoed for.
const ORACLE = "GuBhyNi5GFo9K5YXGKfPMDryWK8GwS5oXe9CJGrzo2sk";
const PROPOSER = "84yVtdReAJ8GiR7Erqj7jyxoJurYWzQ6n9eaBGYBDNqM";
const AUTHORITY = "7bQEwuq9ybNyjjFcbtHBfDPxdH3TuGAsZKVRZdihVN4d";

function hexToBytes(hex: string): number[] {
  const out: number[] = [];
  for (let i = 0; i < hex.length; i += 2) {
    out.push(Number.parseInt(hex.slice(i, i + 2), 16));
  }
  return out;
}

function metaTriples(keys: { pubkey: { toString(): string }; isSigner: boolean; isWritable: boolean }[]) {
  return keys.map((k) => [k.pubkey.toString(), k.isSigner, k.isWritable] as const);
}

describe("submitAiClaimFromRunner — runner-payload byte parity", () => {
  it("(a) data == [Ix.SubmitAiClaim, ...payload_hex bytes] (parity holds against the genuine fixture)", async () => {
    const out = fixture();
    const ix = await submitAiClaimFromRunner(out, { oracle: ORACLE, proposer: PROPOSER, authority: AUTHORITY });

    const expected = new Uint8Array([Ix.SubmitAiClaim, ...hexToBytes(out.submit_ai_claim_payload_hex)]);
    expect(ix.data).toEqual(expected);
    expect(ix.data.length).toBe(98);
    expect(ix.data[0]).toBe(Ix.SubmitAiClaim);
  });

  it("accepts the runner output as a raw JSON string", async () => {
    const ix = await submitAiClaimFromRunner(RAW_FIXTURE, {
      oracle: ORACLE,
      proposer: PROPOSER,
      authority: AUTHORITY,
    });
    const expected = new Uint8Array([Ix.SubmitAiClaim, ...hexToBytes(fixture().submit_ai_claim_payload_hex)]);
    expect(ix.data).toEqual(expected);
  });

  it("(b) accounts in the right order/roles, aiClaim PDA == [b\"claim\", oracle, proposer]", async () => {
    const ix = await submitAiClaimFromRunner(fixture(), {
      oracle: ORACLE,
      proposer: PROPOSER,
      authority: AUTHORITY,
    });
    const aiClaim = await pda.aiClaim(ORACLE, PROPOSER);
    expect(metaTriples(ix.keys)).toEqual([
      [ORACLE, false, true],
      [PROPOSER, false, true],
      [aiClaim.address.toString(), false, true],
      [AUTHORITY, true, true],
      [SYSTEM_PROGRAM_ID.toString(), false, false],
    ]);
  });

  it("(c) a TAMPERED hash (no longer matching the payload) makes the parity guard THROW", async () => {
    const out = fixture();
    // Flip the first byte of model_id_hex: "52..." -> "53...". The structured
    // field now disagrees with the (unchanged, genuine) payload hex.
    const tamperedByte = out.model_id_hex.slice(0, 2) === "53" ? "52" : "53";
    out.model_id_hex = tamperedByte + out.model_id_hex.slice(2);

    await expect(
      submitAiClaimFromRunner(out, { oracle: ORACLE, proposer: PROPOSER, authority: AUTHORITY }),
    ).rejects.toThrow(/parity:.*does not match the runner's/);
  });

  it("(d) a wrong-width hash hex throws", async () => {
    const out = fixture();
    out.params_hash_hex = "dead"; // 2 bytes, not 32
    await expect(
      submitAiClaimFromRunner(out, { oracle: ORACLE, proposer: PROPOSER, authority: AUTHORITY }),
    ).rejects.toThrow(/params_hash_hex must be exactly 32 bytes/);
  });

  it("(e) a claim_pda_seeds mismatch (wrong oracle) throws when seeds are present", async () => {
    const out = fixture();
    expect(out.claim_pda_seeds).toBeDefined();
    await expect(
      submitAiClaimFromRunner(out, {
        oracle: PROPOSER, // deliberately wrong (not the oracle the runner echoed)
        proposer: PROPOSER,
        authority: AUTHORITY,
      }),
    ).rejects.toThrow(/claim_pda_seeds.oracle .* does not match the passed oracle/);
  });
});
