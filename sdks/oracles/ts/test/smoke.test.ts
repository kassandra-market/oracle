/**
 * D0 smoke test — the TypeScript parallel to `programs/kassandra/tests/smoke.rs`.
 *
 * Proves three things at once:
 *   1. The compiled `.so` loads into litesvm without panicking.
 *   2. A transaction BUILT + SIGNED with `@solana/web3.js@3.0.0-rc.2` (legacy
 *      API) round-trips through the kit interop bridge into litesvm.
 *   3. The real Kassandra program runs and REJECTS an unknown instruction
 *      discriminant with `InvalidInstructionData`
 *      (see `processor/mod.rs`: `Ix::from_u8(disc).ok_or(InvalidInstructionData)`).
 *
 * Build the artifact first: `just build` (repo root) produces
 * `target/deploy/kassandra_program.so`.
 */
import { existsSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { address, lamports } from "@solana/kit";
import { Address, Keypair, Transaction, TransactionInstruction } from "@solana/web3.js";
import { FailedTransactionMetadata, LiteSVM } from "litesvm";
import { beforeAll, describe, expect, it } from "vitest";

import { toLiteSvmTransaction } from "../src/litesvm-interop.js";

const PROGRAM_ID = "KassVxvXUEPr5apSr2MqiGva4VFtJXyYLLDFS3f83nY";

const here = dirname(fileURLToPath(import.meta.url));
const SO_PATH = resolve(here, "../../../../target/deploy/kassandra_program.so");

describe("kassandra program smoke (litesvm + web3.js v3)", () => {
  beforeAll(() => {
    if (!existsSync(SO_PATH)) {
      throw new Error(
        `Missing program artifact at ${SO_PATH}. Run \`just build\` from the repo root first.`,
      );
    }
  });

  it("loads the .so without panicking", () => {
    const svm = new LiteSVM();
    svm.addProgramFromFile(address(PROGRAM_ID), SO_PATH);
    // Loading without throwing is the assertion (mirrors smoke.rs).
  });

  it("rejects an unknown instruction discriminant with InvalidInstructionData", async () => {
    const svm = new LiteSVM();
    svm.addProgramFromFile(address(PROGRAM_ID), SO_PATH);

    // Fund a fee payer (litesvm wants kit Address + Lamports).
    const payer = await Keypair.generate();
    svm.airdrop(payer.address, lamports(1_000_000_000n));

    // A dummy account just so the instruction carries one (the program
    // rejects on the discriminant before touching accounts).
    const dummy = await Keypair.generate();

    // Build a legacy web3.js v3 instruction with a bogus 1-byte discriminant
    // (0xFE = 254; not in Ix 0..=21 => from_u8 returns None => rejected).
    const ix = new TransactionInstruction({
      programId: new Address(PROGRAM_ID),
      keys: [{ pubkey: dummy.publicKey, isSigner: false, isWritable: false }],
      data: new Uint8Array([0xfe]),
    });

    const tx = new Transaction();
    tx.feePayer = payer.publicKey;
    tx.recentBlockhash = svm.latestBlockhash();
    tx.add(ix);
    await tx.sign(payer);

    const result = svm.sendTransaction(await toLiteSvmTransaction(tx));

    // The program must reject: litesvm surfaces this as FailedTransactionMetadata.
    expect(result).toBeInstanceOf(FailedTransactionMetadata);
    const failed = result as FailedTransactionMetadata;
    // The instruction error class is InvalidInstructionData.
    expect(failed.toString()).toContain("InvalidInstructionData");
  });
});
