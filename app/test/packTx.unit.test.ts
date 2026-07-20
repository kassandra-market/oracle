/**
 * Offline unit tests for `src/market/data/actions/packTx.ts` — the greedy
 * step-to-transaction packer. Pure (no chain/network); instructions are
 * synthesized with controlled data sizes to pin exact packing boundaries.
 */
import {
  ComputeBudgetProgram,
  Keypair,
  PACKET_DATA_SIZE,
  SystemProgram,
  TransactionInstruction,
  type AccountMeta,
  type Address,
} from "@solana/web3.js";
import { beforeAll, describe, expect, it } from "vitest";

import { packSteps, MAX_TX_COMPUTE_UNITS } from "../src/market/data/actions/packTx";
import type { ActivateStep } from "../src/market/data/actions/activate";

const PROGRAM_ID = SystemProgram.programId;
let ACCOUNT: Address;

beforeAll(async () => {
  ACCOUNT = (await Keypair.generate()).publicKey;
});

function ix(dataLen: number, keys: AccountMeta[] = []): TransactionInstruction {
  return new TransactionInstruction({ programId: PROGRAM_ID, keys, data: new Uint8Array(dataLen) });
}

function step(label: string, opts: { computeUnits?: number; dataLen?: number; extraIxs?: TransactionInstruction[] } = {}): ActivateStep {
  return {
    label,
    computeUnits: opts.computeUnits,
    ixs: [...(opts.extraIxs ?? []), ix(opts.dataLen ?? 8)],
    checkAccount: ACCOUNT,
  };
}

describe("packSteps", () => {
  it("packs every step into one batch when small enough", () => {
    const steps = [step("a"), step("b"), step("c")];
    const batches = packSteps(ACCOUNT, steps);
    expect(batches.length).toBe(1);
    expect(batches[0].steps).toEqual(steps);
  });

  it("preserves step order across and within batches", () => {
    const steps = [step("a"), step("b", { dataLen: 900 }), step("c", { dataLen: 900 }), step("d")];
    const batches = packSteps(ACCOUNT, steps);
    const flattened = batches.flatMap((b) => b.steps);
    expect(flattened).toEqual(steps);
  });

  it("splits into a new batch once the combined wire size would exceed PACKET_DATA_SIZE", () => {
    // Two ~900-byte-data steps can't share one legacy transaction (1232-byte cap).
    const steps = [step("big1", { dataLen: 900 }), step("big2", { dataLen: 900 })];
    const batches = packSteps(ACCOUNT, steps);
    expect(batches.length).toBe(2);
    expect(batches[0].steps).toEqual([steps[0]]);
    expect(batches[1].steps).toEqual([steps[1]]);
  });

  it("needs more than one batch once enough small steps overflow PACKET_DATA_SIZE", () => {
    const steps = Array.from({ length: 8 }, (_, i) => step(`s${i}`, { dataLen: 200 }));
    const batches = packSteps(ACCOUNT, steps);
    expect(batches.length).toBeGreaterThan(1);
  });

  it("keeps a single step that alone exceeds the size limit in its own batch", () => {
    const huge = step("huge", { dataLen: PACKET_DATA_SIZE });
    const steps = [huge, step("small")];
    const batches = packSteps(ACCOUNT, steps);
    expect(batches[0].steps).toEqual([huge]);
  });

  it("merges per-step compute units into ONE SetComputeUnitLimit per batch", () => {
    const steps = [step("a", { computeUnits: 200_000 }), step("b", { computeUnits: 200_000 })];
    const batches = packSteps(ACCOUNT, steps);
    expect(batches.length).toBe(1);
    // First ix is the merged compute-budget ix; the rest are the steps' own ixs in order.
    expect(batches[0].ixs.length).toBe(1 + steps.length);
  });

  it("extracts an EMBEDDED SetComputeUnitLimit (bulk-liquidity-style step) instead of duplicating it", () => {
    // Mirrors buildAddLiquidityIxs: no `computeUnits` field, the compute-budget
    // ix is already the front of `ixs`.
    const embedded = ComputeBudgetProgram.setComputeUnitLimit({ units: 600_000 });
    const a = step("a", { extraIxs: [embedded] });
    const b = step("b", { extraIxs: [ComputeBudgetProgram.setComputeUnitLimit({ units: 600_000 })] });
    const batches = packSteps(ACCOUNT, [a, b]);
    expect(batches.length).toBe(1);
    const computeIxs = batches[0].ixs.filter((i) => i.programId.toString() === embedded.programId.toString());
    expect(computeIxs.length).toBe(1); // exactly one merged ix, not two
  });

  it("defaults an undeclared step to the 200k Solana default when summing", () => {
    // MAX + an undeclared step: undeclared is assumed to need up to the 200k
    // default, so MAX (already at the hard ceiling) can never absorb it.
    const steps = [step("max", { computeUnits: MAX_TX_COMPUTE_UNITS }), step("undeclared")];
    const batches = packSteps(ACCOUNT, steps);
    expect(batches.length).toBe(2);
  });

  it("packs two steps whose summed compute units exactly equals the ceiling", () => {
    const steps = [step("a", { computeUnits: 700_000 }), step("b", { computeUnits: 700_000 })];
    const batches = packSteps(ACCOUNT, steps);
    expect(batches.length).toBe(1);
  });

  it("never combines a step whose compute units alone hit the ceiling with anything else", () => {
    const steps = [step("small"), step("max", { computeUnits: MAX_TX_COMPUTE_UNITS }), step("small2")];
    const batches = packSteps(ACCOUNT, steps);
    const maxBatch = batches.find((b) => b.steps.some((s) => s.label === "max"))!;
    expect(maxBatch.steps).toEqual([steps[1]]);
  });

  it("splits once the summed compute units would exceed the ceiling even if bytes fit", () => {
    const steps = [step("a", { computeUnits: 800_000 }), step("b", { computeUnits: 800_000 })];
    const batches = packSteps(ACCOUNT, steps);
    expect(batches.length).toBe(2);
  });

  it("returns no batches for an empty step list", () => {
    expect(packSteps(ACCOUNT, [])).toEqual([]);
  });
});
