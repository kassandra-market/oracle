/**
 * D2 (cont.) — the STRONGEST account-decoder check: a litesvm test that creates
 * a REAL `Protocol` account via the program's `init_protocol`, fetches the raw
 * bytes, and decodes them with the SDK — proving the offsets against the real
 * program, not just the synthetic buffers in accounts.test.ts. Shared fixtures
 * live in ./helpers/accounts.ts.
 */
import { existsSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { address, lamports } from "@solana/kit";
import { Address, Keypair, Transaction, TransactionInstruction } from "@solana/web3.js";
import { LiteSVM, TransactionMetadata } from "litesvm";
import { beforeAll, describe, expect, it } from "vitest";

import { AccountType, ACCOUNT_SIZES, Ix } from "../src/constants.js";
import { decodeProtocol } from "../src/accounts/index.js";
import * as pda from "../src/pda.js";
import { toLiteSvmTransaction } from "../src/litesvm-interop.js";
import { PROGRAM_ID, SYSTEM_PROGRAM_ID, TOKEN_PROGRAM_ID } from "./helpers/accounts.js";

const here = dirname(fileURLToPath(import.meta.url));
const SO_PATH = resolve(here, "../../../../target/deploy/kassandra_oracles_program.so");

describe("decode a REAL program-created Protocol (litesvm + init_protocol)", () => {
  beforeAll(() => {
    if (!existsSync(SO_PATH)) {
      throw new Error(
        `Missing program artifact at ${SO_PATH}. Run \`just build\` from the repo root first.`,
      );
    }
  });

  it("init_protocol → fetch bytes → decodeProtocol matches the inputs", async () => {
    const svm = new LiteSVM();
    svm.addProgramFromFile(address(PROGRAM_ID), SO_PATH);

    const payer = await Keypair.generate();
    svm.airdrop(payer.address, lamports(10_000_000_000n));

    // Fabricate the canonical KASS/USDC mints: init_protocol only requires they
    // be owned by the SPL token program (it does not parse the mint layout), so
    // a token-program-owned buffer suffices to exercise the real processor.
    const kassMint = await Keypair.generate();
    const usdcMint = await Keypair.generate();
    for (const mint of [kassMint, usdcMint]) {
      svm.setAccount({
        address: mint.address,
        data: new Uint8Array(82), // SPL mint size; contents irrelevant to init_protocol
        executable: false,
        lamports: lamports(1_000_000_000n),
        programAddress: address(TOKEN_PROGRAM_ID),
        space: 82n,
      });
    }

    const protocolPda = await pda.protocol();

    // init_protocol accounts (processor order):
    //   0 protocol PDA (w) | 1 admin (signer, w) | 2 kass_mint | 3 usdc_mint | 4 system program
    const ix = new TransactionInstruction({
      programId: new Address(PROGRAM_ID),
      keys: [
        { pubkey: protocolPda.address, isSigner: false, isWritable: true },
        { pubkey: payer.publicKey, isSigner: true, isWritable: true },
        { pubkey: kassMint.publicKey, isSigner: false, isWritable: false },
        { pubkey: usdcMint.publicKey, isSigner: false, isWritable: false },
        { pubkey: new Address(SYSTEM_PROGRAM_ID), isSigner: false, isWritable: false },
      ],
      data: new Uint8Array([Ix.InitProtocol]), // disc 9, no payload
    });

    const tx = new Transaction();
    tx.feePayer = payer.publicKey;
    tx.recentBlockhash = svm.latestBlockhash();
    tx.add(ix);
    await tx.sign(payer);

    const result = svm.sendTransaction(await toLiteSvmTransaction(tx));
    expect(result).toBeInstanceOf(TransactionMetadata);

    // Fetch the raw account bytes and decode via the SDK.
    const fetched = svm.getAccount(address(protocolPda.address.toString()));
    if (!fetched || !fetched.exists) throw new Error("protocol account not found after init");
    const data = fetched.data;
    expect(data.length).toBe(ACCOUNT_SIZES.Protocol);

    const p = decodeProtocol(data);
    expect(p.accountType).toBe(AccountType.Protocol);
    expect(p.admin.toString()).toBe(payer.publicKey.toString());
    expect(p.kassMint.toString()).toBe(kassMint.publicKey.toString());
    expect(p.usdcMint.toString()).toBe(usdcMint.publicKey.toString());
    expect(p.bump).toBe(protocolPda.bump);
    // init_protocol defaults (config.rs consts): genesis fee-EMA is zeroed,
    // governance unset, monetary params at their documented defaults.
    expect(p.feeEma).toBe(0n);
    expect(p.lastCreationUnix).toBe(0n);
    expect(p.governanceSet).toBe(false);
    // Emission ON by default (config.rs: EMISSION_NUM/DEN, TOTAL_SUPPLY_CAP).
    expect(p.emissionNum).toBe(1n);
    expect(p.emissionDen).toBe(1_000_000n);
    expect(p.totalSupplyCap).toBe(1_000_000_000_000_000_000n);
    expect(p.thresholdNum).toBe(2n);
    expect(p.thresholdDen).toBe(3n);
    expect(p.challengeFailUsdcFeeDen).toBe(100n);
    expect(p.challengeSuccessKassFeeDen).toBe(100n);
  });
});
