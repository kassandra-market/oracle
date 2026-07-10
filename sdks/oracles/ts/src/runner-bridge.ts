/**
 * Runner → SDK bridge: turn the Rust runner's `run` output into the on-chain
 * `submit_ai_claim` instruction, with a byte-parity guard.
 *
 * The runner (`runner/src/cli.rs`) emits a {@link RunnerOutput} JSON carrying
 * the three 32-byte claim hashes (hex) + the chosen option + a precomputed
 * 97-byte `submit_ai_claim` payload (hex). {@link submitAiClaimFromRunner}
 * rebuilds that payload via the SDK's {@link submitAiClaim} builder and ASSERTS
 * the SDK-encoded bytes reproduce the runner's payload exactly — so any
 * encoding drift between the two implementations (disc, field order, widths) is
 * a hard failure rather than a silent on-chain mismatch.
 */
import { Address, TransactionInstruction } from "@solana/web3.js";

import { Ix } from "./constants.js";
import type { AddressInput } from "./pda.js";
import { submitAiClaim } from "./instructions/dispute.js";

/** The AiClaim PDA seed hint the runner echoes when oracle/proposer are known. */
export interface RunnerClaimPdaSeeds {
  /** The literal seed prefix — always `"claim"`. */
  seed_prefix: string;
  /** The oracle pubkey seed (base58). */
  oracle: string;
  /** The proposer pubkey seed (base58). */
  proposer: string;
}

/**
 * The runner's `run` output — the TypeScript mirror of `RunOutput`
 * (`runner/src/cli.rs`). Field names match the emitted JSON (snake_case).
 */
export interface RunnerOutput {
  /** The chosen categorical option index. */
  option_index: number;
  /** `sha256(model_id_string)` as 64 hex chars (32 bytes). */
  model_id_hex: string;
  /** `params_hash` as 64 hex chars (32 bytes). */
  params_hash_hex: string;
  /** `io_hash` as 64 hex chars (32 bytes). */
  io_hash_hex: string;
  /** The exact 97-byte `submit_ai_claim` payload (`model_id ++ params_hash ++ io_hash ++ option`) as hex. */
  submit_ai_claim_payload_hex: string;
  /** The resolved model identifier string actually recorded. */
  resolved_model_id: string;
  /** The AiClaim PDA seeds, present only when the config carried oracle/proposer. */
  claim_pda_seeds?: RunnerClaimPdaSeeds;
}

/** Options the caller supplies (the runner output carries only the metadata). */
export interface SubmitAiClaimFromRunnerOpts {
  /** The oracle (must be in the `AiClaim` phase). */
  oracle: AddressInput;
  /** The submitter's Proposer PDA. */
  proposer: AddressInput;
  /** Proposer authority (signer): must equal `proposer.authority`. */
  authority: AddressInput;
  programId?: Address;
}

/**
 * Decode a hex string (optional `0x` prefix) to bytes, validating that it
 * yields EXACTLY `expectedLen` bytes. `field` names the source for errors.
 */
function hexToBytes(hex: string, expectedLen: number, field: string): Uint8Array {
  const s = hex.startsWith("0x") || hex.startsWith("0X") ? hex.slice(2) : hex;
  if (s.length % 2 !== 0) {
    throw new Error(`runner-bridge: ${field} has odd-length hex (${s.length} chars)`);
  }
  const out = new Uint8Array(s.length / 2);
  for (let i = 0; i < out.length; i++) {
    const byte = Number.parseInt(s.slice(2 * i, 2 * i + 2), 16);
    if (Number.isNaN(byte)) {
      throw new Error(`runner-bridge: ${field} contains invalid hex`);
    }
    out[i] = byte;
  }
  if (out.length !== expectedLen) {
    throw new Error(
      `runner-bridge: ${field} must be exactly ${expectedLen} bytes, got ${out.length}`,
    );
  }
  return out;
}

/** Whether two byte arrays are equal (length + every byte). */
function bytesEqual(a: Uint8Array, b: Uint8Array): boolean {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) {
    if (a[i] !== b[i]) return false;
  }
  return true;
}

/** Index of the first differing byte, or -1 if equal up to the shorter length. */
function firstDiff(a: Uint8Array, b: Uint8Array): number {
  const n = Math.min(a.length, b.length);
  for (let i = 0; i < n; i++) {
    if (a[i] !== b[i]) return i;
  }
  return a.length === b.length ? -1 : n;
}

/** Normalize an address to its canonical base58 string. */
function asBase58(a: AddressInput): string {
  return (a instanceof Address ? a : new Address(a)).toString();
}

/**
 * Build the `submit_ai_claim` instruction from a runner `run` output, with a
 * byte-parity guard against the runner's precomputed payload.
 *
 * Accepts the {@link RunnerOutput} as a parsed object OR the raw JSON string.
 * Hex-decodes the three claim hashes (validating 32-byte width), builds the
 * instruction via {@link submitAiClaim}, and asserts the SDK-encoded payload
 * (`data[1..98]`) byte-equals the runner's `submit_ai_claim_payload_hex`. If
 * `claim_pda_seeds` is present, the oracle/proposer it names must match the
 * passed `oracle`/`proposer` (so the derived AiClaim PDA is the one the runner
 * described). Throws a specific error on any mismatch.
 *
 * @returns the verified, ready-to-sign instruction.
 */
export async function submitAiClaimFromRunner(
  runOutput: RunnerOutput | string,
  opts: SubmitAiClaimFromRunnerOpts,
): Promise<TransactionInstruction> {
  const run: RunnerOutput = typeof runOutput === "string" ? JSON.parse(runOutput) : runOutput;

  const modelId = hexToBytes(run.model_id_hex, 32, "model_id_hex");
  const paramsHash = hexToBytes(run.params_hash_hex, 32, "params_hash_hex");
  const ioHash = hexToBytes(run.io_hash_hex, 32, "io_hash_hex");
  const option = run.option_index;

  // The runner's precomputed payload (97 bytes: model_id ++ params_hash ++ io_hash ++ option).
  const expectedPayload = hexToBytes(run.submit_ai_claim_payload_hex, 97, "submit_ai_claim_payload_hex");

  const instruction = await submitAiClaim({
    oracle: opts.oracle,
    proposer: opts.proposer,
    authority: opts.authority,
    modelId,
    paramsHash,
    ioHash,
    option,
    programId: opts.programId,
  });

  // --- PARITY GUARD: the SDK encoding MUST reproduce the runner's payload. ---
  const data = instruction.data;
  if (data.length !== 98) {
    throw new Error(
      `runner-bridge parity: SDK instruction data is ${data.length} bytes, expected 98 (1 disc + 97 payload)`,
    );
  }
  if (data[0] !== Ix.SubmitAiClaim) {
    throw new Error(
      `runner-bridge parity: SDK discriminant is ${data[0]}, expected Ix.SubmitAiClaim (${Ix.SubmitAiClaim})`,
    );
  }
  const sdkPayload = data.slice(1, 98);
  if (!bytesEqual(sdkPayload, expectedPayload)) {
    const i = firstDiff(sdkPayload, expectedPayload);
    throw new Error(
      `runner-bridge parity: SDK-encoded payload does not match the runner's ` +
        `submit_ai_claim_payload_hex (first differs at byte ${i}: SDK ${sdkPayload[i]} vs runner ${expectedPayload[i]}). ` +
        `This is a runner↔SDK encoding drift (disc/field order/width).`,
    );
  }

  // --- OPTIONAL cross-check: the runner's PDA seeds must name our accounts. ---
  if (run.claim_pda_seeds) {
    const seeds = run.claim_pda_seeds;
    if (seeds.seed_prefix !== "claim") {
      throw new Error(
        `runner-bridge: claim_pda_seeds.seed_prefix is "${seeds.seed_prefix}", expected "claim"`,
      );
    }
    const passedOracle = asBase58(opts.oracle);
    const passedProposer = asBase58(opts.proposer);
    if (asBase58(seeds.oracle) !== passedOracle) {
      throw new Error(
        `runner-bridge: claim_pda_seeds.oracle (${seeds.oracle}) does not match the passed oracle (${passedOracle})`,
      );
    }
    if (asBase58(seeds.proposer) !== passedProposer) {
      throw new Error(
        `runner-bridge: claim_pda_seeds.proposer (${seeds.proposer}) does not match the passed proposer (${passedProposer})`,
      );
    }
  }

  return instruction;
}
