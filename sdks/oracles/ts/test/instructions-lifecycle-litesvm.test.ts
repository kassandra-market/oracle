/**
 * D3a (cont.) — litesvm acceptance: build init_protocol VIA THE SDK and prove
 * the real program accepts it (account order + roles correct). Split out of
 * instructions-lifecycle.test.ts; shared fixtures live in
 * ./helpers/instructions-lifecycle.ts.
 */
import { existsSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { address, lamports } from "@solana/kit";
import { Keypair, Transaction } from "@solana/web3.js";
import { LiteSVM, TransactionMetadata } from "litesvm";
import { describe, expect, it } from "vitest";

import { KASSANDRA_PROGRAM_ID, TOKEN_PROGRAM_ID } from "../src/constants.js";
import * as pda from "../src/pda.js";
import { initProtocol } from "../src/instructions/index.js";
import { toLiteSvmTransaction } from "../src/litesvm-interop.js";
import { PROGRAM_ID } from "./helpers/instructions-lifecycle.js";

const here = dirname(fileURLToPath(import.meta.url));
const SO_PATH = resolve(here, "../../../../target/deploy/kassandra_oracles_program.so");

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
