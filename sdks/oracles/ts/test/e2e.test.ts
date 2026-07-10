/**
 * D4 — litesvm END-TO-END lifecycle, driven THROUGH THE SDK.
 *
 * This is the proof the hand-written SDK matches the deployed program: every
 * instruction below is built by an SDK builder (`src/instructions/*`), bridged
 * into litesvm via `toLiteSvmTransaction`, submitted to the REAL
 * `target/deploy/kassandra_program.so`, and the resulting accounts are decoded
 * by the SDK decoders (`src/accounts/*`) — no synthetic buffers. It mirrors the
 * Rust reference `tests/lifecycle_e2e.rs` (`e2e_happy_uncontested_resolves` +
 * the `FactProposal` dispute slice from `dispute_via_real_flow`).
 *
 * Flow (happy / uncontested-resolve):
 *   1. init_protocol  → decode Protocol, assert admin/mints.
 *   2. create_oracle (3 options, near deadline) → decode Oracle, assert Proposal.
 *   3. propose ×3, ALL the same option, distinct funded authorities → decode each
 *      Proposer, assert bond/original_option/claim_option == CLAIM_OPTION_NONE.
 *   4. warp the clock past the proposal window.
 *   5. finalize_proposals (full proposer set) → decode Oracle, assert Resolved +
 *      resolved_option == the agreed option (and stake-vault conservation).
 * Plus a dispute slice: propose 2 DISTINCT options → finalize_proposals lands in
 * FactProposal with dispute_bond_total == Σ bonds.
 *
 * --- SPL funding ---
 * The Rust harness fabricates SPL mint/token-account bytes directly via
 * `set_account` (it never runs InitializeMint). We mirror that: `mintBytes` /
 * `tokenAccountBytes` pack the canonical SPL layouts (82-byte Mint, 165-byte
 * Account) and `svm.setAccount` writes them token-program-owned. The program's
 * own CPIs (create_oracle's `InitializeAccount3` on the vault, propose's
 * `Transfer`) run against the SPL Token program that `new LiteSVM()` loads by
 * default (`withDefaultPrograms`). The KASS mint authority is set to the
 * mint-authority PDA to mirror the harness bootstrap; this IS load-bearing —
 * emission is ON by default, so create_oracle mints `reward_emission` into the
 * stake vault, program-signed by that PDA (a wrong authority would trip the
 * BadMintAuthority guard). The genesis creation fee is 0 (fee_ema == 0), so no
 * KASS is burned at create — only the emission is minted.
 *
 * --- clock warp ---
 * litesvm exposes `getClock()` / `setClock(Clock)`. The program's `now()` reads
 * `Clock.unix_timestamp`, and the phase gates compare `now >= phase_ends_at`, so
 * advancing time == bumping `unixTimestamp` (and `slot`) then `setClock`, exactly
 * what the Rust harness's `warp` does with `set_sysvar::<Clock>`.
 */
import { existsSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { address, lamports } from "@solana/kit";
import { Address, Keypair, Transaction, type TransactionInstruction } from "@solana/web3.js";
import { Clock, FailedTransactionMetadata, LiteSVM, TransactionMetadata } from "litesvm";
import { beforeAll, describe, expect, it } from "vitest";

import { decodeOracle, decodeProposer, decodeProtocol } from "../src/accounts/index.js";
import { AccountType, CLAIM_OPTION_NONE, Phase, TOKEN_PROGRAM_ID } from "../src/constants.js";
import { createOracle, finalizeProposals, initProtocol, propose } from "../src/instructions/index.js";
import { toLiteSvmTransaction } from "../src/litesvm-interop.js";
import * as pda from "../src/pda.js";

const PROGRAM_ID = "KassVxvXUEPr5apSr2MqiGva4VFtJXyYLLDFS3f83nY";

const here = dirname(fileURLToPath(import.meta.url));
const SO_PATH = resolve(here, "../../../../target/deploy/kassandra_program.so");

// --- SPL layout fabrication (mirrors the Rust harness `create_mint` /
//     `create_token_account`, packing the canonical spl-token byte layouts). ---

const MINT_LEN = 82;
const TOKEN_ACCOUNT_LEN = 165;

/** A 32-byte SPL `Mint` writer over `data` at `offset`, COption tag 1 = Some. */
function mintBytes(authority: Address, supply: bigint, decimals: number): Uint8Array {
  const data = new Uint8Array(MINT_LEN);
  const dv = new DataView(data.buffer);
  // mint_authority: COption<Pubkey> = u32 tag (1 = Some) ++ 32-byte pubkey.
  dv.setUint32(0, 1, true);
  data.set(authority.toBytes(), 4);
  dv.setBigUint64(36, supply, true); // supply
  data[44] = decimals; // decimals
  data[45] = 1; // is_initialized
  // freeze_authority: COption<Pubkey> tag 0 = None (rest stays zero).
  return data;
}

/** A 165-byte SPL token `Account` holding `amount` of `mint`, owned by `owner`. */
function tokenAccountBytes(mint: Address, owner: Address, amount: bigint): Uint8Array {
  const data = new Uint8Array(TOKEN_ACCOUNT_LEN);
  const dv = new DataView(data.buffer);
  data.set(mint.toBytes(), 0); // mint
  data.set(owner.toBytes(), 32); // owner
  dv.setBigUint64(64, amount, true); // amount
  // delegate COption tag 0 (offset 72), state = Initialized (offset 108),
  // is_native COption tag 0 (offset 109), delegated_amount 0 (121),
  // close_authority COption tag 0 (129) — all default to the zeroed buffer
  // except `state`, which must be 1 (Initialized).
  data[108] = 1;
  return data;
}

/** Write a token-program-owned SPL account into litesvm at `key`. */
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

/** Read the `amount` (u64 @ offset 64) of a fabricated/real SPL token account. */
function tokenBalance(svm: LiteSVM, key: Address): bigint {
  const acct = svm.getAccount(address(key.toString()));
  if (!acct || !acct.exists) throw new Error(`token account ${key} not found`);
  return new DataView(acct.data.buffer, acct.data.byteOffset, acct.data.length).getBigUint64(
    64,
    true,
  );
}

/**
 * Advance the litesvm clock by `seconds` (and one slot), exactly like the Rust
 * harness `warp`: the program's `now()` reads `Clock.unix_timestamp`, so this is
 * what crosses `phase_ends_at`.
 */
function warp(svm: LiteSVM, seconds: bigint): void {
  const c = svm.getClock();
  svm.setClock(
    new Clock(
      c.slot + 1n,
      c.epochStartTimestamp,
      c.epoch,
      c.leaderScheduleEpoch,
      c.unixTimestamp + seconds,
    ),
  );
}

/** Build, sign (payer + extra signers), bridge, and submit a single ix. */
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
  await tx.sign(payer, ...signers);
  return svm.sendTransaction(await toLiteSvmTransaction(tx));
}

function expectOk(
  result: TransactionMetadata | FailedTransactionMetadata,
  what: string,
): TransactionMetadata {
  if (result instanceof FailedTransactionMetadata) {
    throw new Error(`${what} failed: ${result.toString()}`);
  }
  return result;
}

/** Fetch raw account bytes from litesvm (throws if absent). */
function fetchData(svm: LiteSVM, key: Address): Uint8Array {
  const acct = svm.getAccount(address(key.toString()));
  if (!acct || !acct.exists) throw new Error(`account ${key} not found`);
  return acct.data;
}

interface Fixture {
  svm: LiteSVM;
  payer: Keypair;
  kassMint: Keypair;
  usdcMint: Keypair;
  baseUnix: bigint;
}

/** Stand up litesvm + program + canonical KASS/USDC mints + a funded payer. */
async function setupFixture(): Promise<Fixture> {
  const svm = new LiteSVM();
  svm.addProgramFromFile(address(PROGRAM_ID), SO_PATH);

  const payer = await Keypair.generate();
  svm.airdrop(payer.address, lamports(100_000_000_000n));

  // KASS mint authority = the mint-authority PDA (mirrors the harness bootstrap;
  // not load-bearing here since emissions are disabled). USDC authority = payer.
  const mintAuth = await pda.mintAuthority();
  const kassMint = await Keypair.generate();
  const usdcMint = await Keypair.generate();
  putSplAccount(svm, kassMint.publicKey, mintBytes(mintAuth.address, 0n, 9));
  putSplAccount(svm, usdcMint.publicKey, mintBytes(payer.publicKey, 0n, 6));

  const baseUnix = svm.getClock().unixTimestamp;
  return { svm, payer, kassMint, usdcMint, baseUnix };
}

/** init_protocol via the SDK, decode the resulting Protocol, assert + return it. */
async function initProtocolAndDecode(f: Fixture) {
  const ix = await initProtocol({
    admin: f.payer.publicKey,
    kassMint: f.kassMint.publicKey,
    usdcMint: f.usdcMint.publicKey,
  });
  expectOk(await submit(f.svm, f.payer, ix), "init_protocol");

  const protocolPda = await pda.protocol();
  const p = decodeProtocol(fetchData(f.svm, protocolPda.address));
  expect(p.accountType).toBe(AccountType.Protocol);
  expect(p.admin.toString()).toBe(f.payer.publicKey.toString());
  expect(p.kassMint.toString()).toBe(f.kassMint.publicKey.toString());
  expect(p.usdcMint.toString()).toBe(f.usdcMint.publicKey.toString());
  return p;
}

/**
 * create_oracle via the SDK at `nonce`, then warp the clock to the deadline so
 * the proposal window opens. Returns the oracle PDA + the agreed timing. The
 * creator's KASS token account exists but is not charged (genesis fee == 0).
 */
async function createOracleAndOpen(
  f: Fixture,
  nonce: bigint,
  optionsCount: number,
): Promise<Address> {
  const oraclePda = await pda.oracle(nonce);

  // The creator's KASS account (fee-burn source). At genesis fee == 0, so the
  // balance is never read, but we fund it for realism.
  const creatorKass = await Keypair.generate();
  putSplAccount(f.svm, creatorKass.publicKey, tokenAccountBytes(f.kassMint.publicKey, f.payer.publicKey, 1_000_000n));

  const deadline = f.svm.getClock().unixTimestamp + 1_000n; // near future
  const ix = await createOracle({
    nonce,
    optionsCount,
    deadline,
    twapWindow: 600n,
    creator: f.payer.publicKey,
    creatorKassToken: creatorKass.publicKey,
    kassMint: f.kassMint.publicKey,
    usdcMint: f.usdcMint.publicKey,
  });
  expectOk(await submit(f.svm, f.payer, ix), "create_oracle");

  // Warp to the deadline: proposals open at `deadline`, window then open
  // (deadline + PROPOSAL_WINDOW = phase_ends_at, 3600s ahead).
  warp(f.svm, 1_000n);
  return oraclePda.address;
}

/**
 * propose via the SDK from a FRESH funded authority holding `bond` KASS. Returns
 * the authority + proposer PDA; decodes the Proposer and asserts its fields.
 */
async function proposeAndDecode(
  f: Fixture,
  oracle: Address,
  option: number,
  bond: bigint,
): Promise<{ authority: Keypair; proposer: Address }> {
  const authority = await Keypair.generate();
  f.svm.airdrop(authority.address, lamports(10_000_000_000n));
  const authorityKass = await Keypair.generate();
  putSplAccount(
    f.svm,
    authorityKass.publicKey,
    tokenAccountBytes(f.kassMint.publicKey, authority.publicKey, bond * 10n),
  );

  const ix = await propose({
    oracle,
    authority: authority.publicKey,
    authorityKass: authorityKass.publicKey,
    option,
    bond,
  });
  expectOk(await submit(f.svm, f.payer, ix, [authority]), `propose(option=${option})`);

  const proposerPda = await pda.proposer(oracle, authority.publicKey);
  const decoded = decodeProposer(fetchData(f.svm, proposerPda.address));
  expect(decoded.accountType).toBe(AccountType.Proposer);
  expect(decoded.oracle.toString()).toBe(oracle.toString());
  expect(decoded.authority.toString()).toBe(authority.publicKey.toString());
  expect(decoded.bond).toBe(bond);
  expect(decoded.originalOption).toBe(option);
  // claim_option starts at the CLAIM_OPTION_NONE sentinel (set at registration).
  expect(decoded.claimOption).toBe(CLAIM_OPTION_NONE);
  expect(decoded.disqualified).toBe(false);
  expect(decoded.slashed).toBe(false);

  return { authority, proposer: proposerPda.address };
}

describe("D4 litesvm end-to-end lifecycle via the SDK", () => {
  beforeAll(() => {
    if (!existsSync(SO_PATH)) {
      throw new Error(
        `Missing program artifact at ${SO_PATH}. Run \`just build\` from the repo root first.`,
      );
    }
  });

  it("happy uncontested path: init → create → propose×3 (same option) → finalize → Resolved", async () => {
    const f = await setupFixture();

    // 1. init_protocol.
    await initProtocolAndDecode(f);

    // 2. create_oracle (3 options) → Proposal.
    const nonce = 1n;
    const oracle = await createOracleAndOpen(f, nonce, 3);

    let o = decodeOracle(fetchData(f.svm, oracle));
    expect(o.accountType).toBe(AccountType.Oracle);
    expect(o.phase).toBe(Phase.Proposal);
    expect(o.creator.toString()).toBe(f.payer.publicKey.toString());
    expect(o.kassMint.toString()).toBe(f.kassMint.publicKey.toString());
    expect(o.usdcMint.toString()).toBe(f.usdcMint.publicKey.toString());
    expect(o.optionsCount).toBe(3);
    expect(o.proposerCount).toBe(0);

    // 3. propose ×3, ALL agreeing on option 1.
    const agreedOption = 1;
    const bond = 5_000n;
    const proposers: Address[] = [];
    for (let i = 0; i < 3; i++) {
      const { proposer } = await proposeAndDecode(f, oracle, agreedOption, bond);
      proposers.push(proposer);
    }

    // KASS conservation at the proposal boundary (no facts yet): total_oracle_stake
    // is exactly Σ bonds, and the stake vault holds Σ bonds PLUS the emission
    // create_oracle mints into it (emission is ON by default).
    o = decodeOracle(fetchData(f.svm, oracle));
    expect(o.proposerCount).toBe(3);
    expect(o.survivingCount).toBe(3);
    const sumBonds = bond * 3n;
    expect(o.totalOracleStake).toBe(sumBonds);
    const stakeVault = await pda.stakeVault(oracle);
    expect(tokenBalance(f.svm, stakeVault.address)).toBe(sumBonds + o.rewardEmission);

    // 4. warp past the proposal window (deadline + PROPOSAL_WINDOW = +3600).
    warp(f.svm, 3_601n);

    // 5. finalize_proposals (full proposer set) → Resolved with the agreed option.
    const finIx = await finalizeProposals({ oracle, proposers });
    expectOk(await submit(f.svm, f.payer, finIx), "finalize_proposals");

    o = decodeOracle(fetchData(f.svm, oracle));
    expect(o.phase).toBe(Phase.Resolved);
    expect(o.resolvedOption).toBe(agreedOption);
    expect(o.disputeBondTotal).toBe(0n); // no dispute opened
    // No token CPI on the resolve path: the vault is untouched — still Σ bonds +
    // the minted emission (the uncontested reward pool the S2 claims draw from).
    expect(tokenBalance(f.svm, stakeVault.address)).toBe(sumBonds + o.rewardEmission);
  });

  it("dispute slice: propose×2 (distinct options) → finalize → FactProposal with dispute_bond_total set", async () => {
    const f = await setupFixture();
    await initProtocolAndDecode(f);

    const nonce = 1n;
    const oracle = await createOracleAndOpen(f, nonce, 2);
    const bond = 1_000n;

    // Two proposers on DISTINCT options 0 and 1 → conflict.
    const p0 = await proposeAndDecode(f, oracle, 0, bond);
    const p1 = await proposeAndDecode(f, oracle, 1, bond);

    warp(f.svm, 3_601n);
    const finIx = await finalizeProposals({ oracle, proposers: [p0.proposer, p1.proposer] });
    expectOk(await submit(f.svm, f.payer, finIx), "finalize_proposals (dispute)");

    const o = decodeOracle(fetchData(f.svm, oracle));
    // Conflict opens the dispute core: FactProposal with dispute_bond_total ==
    // Σ bonds (the fixed fact-quorum denominator).
    expect(o.phase).toBe(Phase.FactProposal);
    expect(o.proposerCount).toBe(2);
    const sumBonds = bond * 2n;
    expect(o.totalOracleStake).toBe(sumBonds);
    expect(o.disputeBondTotal).toBe(sumBonds);
    // Vault still holds Σ bonds + the minted emission (no token CPI on the
    // open-dispute path either).
    const stakeVault = await pda.stakeVault(oracle);
    expect(tokenBalance(f.svm, stakeVault.address)).toBe(sumBonds + o.rewardEmission);
  });
});
