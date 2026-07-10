/**
 * Kassandra SDK — end-to-end quickstart (illustrative + typechecked).
 *
 * Shows the real shape of using the SDK against the live program:
 *   1. derive PDAs            (`pda.protocol()`, `pda.oracle(nonce)`, …)
 *   2. build instructions     (`initProtocol`, `createOracle`, `propose`, …)
 *   3. sign + send            (web3.js v3 — note `sign()`/`serialize()` are async)
 *   4. bridge to litesvm      (`toLiteSvmTransaction`) for local testing
 *   5. decode accounts        (`decodeProtocol`, `decodeOracle`, …)
 *
 * This file is compiled by `pnpm typecheck`. To actually RUN it you need the
 * program artifact: from the repo root run `just build` to produce
 * `target/deploy/kassandra_program.so`, then `pnpm dlx tsx examples/quickstart.ts`.
 *
 * It is the example-shaped twin of `test/e2e.test.ts`. For brevity it fabricates
 * the SPL KASS/USDC mints + token accounts directly (exactly like the test
 * harness) rather than running InitializeMint.
 */
import { existsSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { address, lamports } from "@solana/kit";
import { Address, Keypair, Transaction, type TransactionInstruction } from "@solana/web3.js";
import {
  Clock,
  FailedTransactionMetadata,
  LiteSVM,
  type TransactionMetadata,
} from "litesvm";

// Everything below comes from the SDK's single public entry point.
import {
  AccountType,
  CLAIM_OPTION_NONE,
  Phase,
  TOKEN_PROGRAM_ID,
  createOracle,
  decodeOracle,
  decodeProposer,
  decodeProtocol,
  finalizeProposals,
  initProtocol,
  pda,
  propose,
  toLiteSvmTransaction,
} from "../src/index.js";

const PROGRAM_ID = "KassVxvXUEPr5apSr2MqiGva4VFtJXyYLLDFS3f83nY";

const here = dirname(fileURLToPath(import.meta.url));
const SO_PATH = resolve(here, "../../../../target/deploy/kassandra_program.so");

// --- tiny SPL helpers (fabricate mint/token bytes, mirrors the test harness) ---

function mintBytes(authority: Address, decimals: number): Uint8Array {
  const data = new Uint8Array(82);
  const dv = new DataView(data.buffer);
  dv.setUint32(0, 1, true); // mint_authority COption tag = Some
  data.set(authority.toBytes(), 4);
  data[44] = decimals;
  data[45] = 1; // is_initialized
  return data;
}

function tokenAccountBytes(mint: Address, owner: Address, amount: bigint): Uint8Array {
  const data = new Uint8Array(165);
  data.set(mint.toBytes(), 0);
  data.set(owner.toBytes(), 32);
  new DataView(data.buffer).setBigUint64(64, amount, true);
  data[108] = 1; // state = Initialized
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

/** Advance the litesvm clock (the program's `now()` reads Clock.unix_timestamp). */
function warp(svm: LiteSVM, seconds: bigint): void {
  const c = svm.getClock();
  svm.setClock(new Clock(c.slot + 1n, c.epochStartTimestamp, c.epoch, c.leaderScheduleEpoch, c.unixTimestamp + seconds));
}

/**
 * Sign + send one instruction. THIS is the web3.js-v3 → litesvm path:
 *   - build a legacy `Transaction`, set feePayer + recentBlockhash, `.add(ix)`
 *   - `await tx.sign(...)`               (async — WebCrypto)
 *   - `toLiteSvmTransaction(tx)`         (serialize → kit Transaction bridge)
 * Against a real RPC you would instead `await tx.serialize()` and send the wire
 * bytes via your `Connection`/RPC of choice.
 */
async function submit(
  svm: LiteSVM,
  payer: Keypair,
  ix: TransactionInstruction,
  extraSigners: Keypair[] = [],
): Promise<TransactionMetadata> {
  const tx = new Transaction();
  tx.feePayer = payer.publicKey;
  tx.recentBlockhash = svm.latestBlockhash();
  tx.add(ix);
  await tx.sign(payer, ...extraSigners);
  const result = svm.sendTransaction(await toLiteSvmTransaction(tx));
  if (result instanceof FailedTransactionMetadata) {
    throw new Error(`transaction failed: ${result.toString()}`);
  }
  return result;
}

function fetchData(svm: LiteSVM, key: Address): Uint8Array {
  const acct = svm.getAccount(address(key.toString()));
  if (!acct || !acct.exists) throw new Error(`account ${key} not found`);
  return acct.data;
}

async function main(): Promise<void> {
  if (!existsSync(SO_PATH)) {
    throw new Error(`Missing ${SO_PATH}. Run \`just build\` from the repo root first.`);
  }

  // --- stand up litesvm + load the program + fabricate KASS/USDC mints ---
  const svm = new LiteSVM();
  svm.addProgramFromFile(address(PROGRAM_ID), SO_PATH);

  const payer = await Keypair.generate();
  svm.airdrop(payer.address, lamports(100_000_000_000n));

  // KASS mint authority is the program's mint-authority PDA (derive #1).
  const mintAuth = await pda.mintAuthority();
  const kassMint = await Keypair.generate();
  const usdcMint = await Keypair.generate();
  putSplAccount(svm, kassMint.publicKey, mintBytes(mintAuth.address, 9));
  putSplAccount(svm, usdcMint.publicKey, mintBytes(payer.publicKey, 6));

  // --- 1. init_protocol -------------------------------------------------------
  await submit(
    svm,
    payer,
    await initProtocol({
      admin: payer.publicKey,
      kassMint: kassMint.publicKey,
      usdcMint: usdcMint.publicKey,
    }),
  );

  // Derive the singleton Protocol PDA and decode the fresh account.
  const protocolPda = await pda.protocol();
  const protocolAcct = decodeProtocol(fetchData(svm, protocolPda.address));
  console.log("protocol admin:", protocolAcct.admin.toString());
  console.log("protocol.accountType === Protocol:", protocolAcct.accountType === AccountType.Protocol);

  // --- 2. create_oracle (nonce 1, 3 options) ---------------------------------
  const nonce = 1n;
  const oraclePda = await pda.oracle(nonce); // PDA from the u64-LE nonce seed

  // The creator's KASS token account is the fee-burn source (genesis fee == 0).
  const creatorKass = await Keypair.generate();
  putSplAccount(svm, creatorKass.publicKey, tokenAccountBytes(kassMint.publicKey, payer.publicKey, 1_000_000n));

  const deadline = svm.getClock().unixTimestamp + 1_000n;
  await submit(
    svm,
    payer,
    await createOracle({
      nonce,
      optionsCount: 3,
      deadline,
      twapWindow: 600n,
      creator: payer.publicKey,
      creatorKassToken: creatorKass.publicKey,
      kassMint: kassMint.publicKey,
      usdcMint: usdcMint.publicKey,
    }),
  );

  let oracleAcct = decodeOracle(fetchData(svm, oraclePda.address));
  console.log("oracle.phase === Proposal:", oracleAcct.phase === Phase.Proposal);
  console.log("oracle.optionsCount:", oracleAcct.optionsCount);

  // Warp to the deadline so the proposal window opens.
  warp(svm, 1_000n);

  // --- 3. propose ×3, all agreeing on option 1 -------------------------------
  const agreedOption = 1;
  const bond = 5_000n;
  const proposers: Address[] = [];
  for (let i = 0; i < 3; i++) {
    const authority = await Keypair.generate();
    svm.airdrop(authority.address, lamports(10_000_000_000n));
    const authorityKass = await Keypair.generate();
    putSplAccount(svm, authorityKass.publicKey, tokenAccountBytes(kassMint.publicKey, authority.publicKey, bond * 10n));

    await submit(
      svm,
      payer,
      await propose({
        oracle: oraclePda.address,
        authority: authority.publicKey,
        authorityKass: authorityKass.publicKey,
        option: agreedOption,
        bond,
      }),
      [authority], // the proposing authority co-signs
    );

    const proposerPda = await pda.proposer(oraclePda.address, authority.publicKey);
    const proposerAcct = decodeProposer(fetchData(svm, proposerPda.address));
    console.log(
      `proposer ${i}: option=${proposerAcct.originalOption} bond=${proposerAcct.bond} claimOption=${
        proposerAcct.claimOption === CLAIM_OPTION_NONE ? "NONE" : proposerAcct.claimOption
      }`,
    );
    proposers.push(proposerPda.address);
  }

  // --- 4. warp past the proposal window, then finalize -----------------------
  warp(svm, 3_601n);
  await submit(svm, payer, await finalizeProposals({ oracle: oraclePda.address, proposers }));

  oracleAcct = decodeOracle(fetchData(svm, oraclePda.address));
  console.log("oracle.phase === Resolved:", oracleAcct.phase === Phase.Resolved);
  console.log("oracle.resolvedOption:", oracleAcct.resolvedOption, "(expected", agreedOption + ")");
}

main().catch((err) => {
  console.error(err);
  process.exitCode = 1;
});
