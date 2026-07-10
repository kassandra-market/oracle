/**
 * `MarketTestCtx` — a LiteSVM test harness for the kassandra-market program,
 * the TypeScript mirror of the Rust `programs/markets/tests/common/mod.rs`.
 *
 * It deploys the compiled program `.so` (+ the two MetaDAO fixtures) into a fresh
 * `LiteSVM`, funds a payer, and exposes the same primitives the Rust harness does:
 * SPL mint / token-account fabrication (`svm.setAccount` with packed byte
 * layouts — no `InitializeMint`), a Kassandra-oracle seeder, a clock warp, and a
 * `send(ix, signers)` that bridges a signed legacy web3.js `Transaction` into the
 * kit `Transaction` litesvm accepts (`toLiteSvmTransaction`).
 *
 * Every instruction the round-trip test sends is built by an SDK builder and every
 * account it reads back is decoded by an SDK decoder, so a green test is the proof
 * the hand-written wire format (account orders + payloads + field offsets) matches
 * the deployed program.
 *
 * litesvm@1.2.0 API used: `new LiteSVM()` (loads SPL Token + System by default),
 * `addProgramFromFile(address, path)`, `airdrop(address, lamports)`,
 * `latestBlockhash()`, `sendTransaction(kitTx)`, `getAccount(address)` →
 * `MaybeEncodedAccount` (`.exists`/`.data`), `setAccount(EncodedAccount)`,
 * `minimumBalanceForRentExemption(bigint)`, `getClock()`/`setClock(Clock)`.
 */
import { existsSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { address, lamports } from "@solana/kit";
import {
  Address,
  ComputeBudgetProgram,
  Keypair,
  Transaction,
  TransactionInstruction,
} from "@solana/web3.js";
import { Clock, FailedTransactionMetadata, LiteSVM, TransactionMetadata } from "litesvm";
import { expect } from "vitest";

import {
  decodeConfig,
  decodeContribution,
  decodeMarket,
  type Config,
  type Contribution,
  type Market,
} from "../src/accounts/index.js";
import {
  BPF_UPGRADEABLE_LOADER_ID,
  EXTERNAL_PROGRAM_IDS,
  MARKET_PROGRAM_ID,
  TOKEN_PROGRAM_ID,
} from "../src/constants.js";
import { closeMarket } from "../src/instructions/index.js";
import { toLiteSvmTransaction } from "../src/litesvm-interop.js";
import * as pda from "../src/pda.js";
import { mintBytes, oracleBytes, tokenAccountBytes } from "./spl-layout.js";

const here = dirname(fileURLToPath(import.meta.url));
const MARKET_SO = resolve(here, "../../../../target/deploy/kassandra_markets_program.so");
const VAULT_SO = resolve(here, "../../../../programs/markets/tests/fixtures/metadao_conditional_vault.so");
const AMM_SO = resolve(here, "../../../../programs/markets/tests/fixtures/metadao_amm.so");

/** The external Kassandra oracle program id (owns the accounts `seedOracle` writes). */
export const KASSANDRA_PROGRAM_ID = new Address("KassVxvXUEPr5apSr2MqiGva4VFtJXyYLLDFS3f83nY");

/** Assert a litesvm tx result is `TransactionMetadata` (success), else throw. */
export function expectOk(
  result: TransactionMetadata | FailedTransactionMetadata | null,
  what: string,
): TransactionMetadata {
  if (result === null || result instanceof FailedTransactionMetadata) {
    throw new Error(`${what} failed: ${result === null ? "null result" : result.toString()}`);
  }
  return result;
}

/**
 * Extract the `ProgramError::Custom(u32)` code from a FAILED litesvm tx result,
 * or `null` for a success / non-custom failure. The TS litesvm binding doesn't
 * re-export the `InstructionErrorCustom` class, so we parse it out of the program
 * logs (`... custom program error: 0x<hex>`). Mirrors the Rust harness
 * `custom_code`, so a test can assert a specific {@link MarketError} was raised.
 */
export function customCode(
  result: TransactionMetadata | FailedTransactionMetadata | null,
): number | null {
  if (!(result instanceof FailedTransactionMetadata)) return null;
  for (const line of result.meta().logs()) {
    const match = /custom program error: 0x([0-9a-fA-F]+)/.exec(line);
    if (match) return parseInt(match[1], 16);
  }
  return null;
}

/** LiteSVM-backed test context for the kassandra-market program. */
export class MarketTestCtx {
  private constructor(
    readonly svm: LiteSVM,
    readonly payer: Keypair,
  ) {}

  /** Stand up litesvm + the program + the MetaDAO fixtures + a funded payer. */
  static async new(): Promise<MarketTestCtx> {
    if (!existsSync(MARKET_SO)) {
      throw new Error(`Missing program artifact at ${MARKET_SO}. Run \`just build\` from the repo root first.`);
    }
    const svm = new LiteSVM();
    svm.addProgramFromFile(address(MARKET_PROGRAM_ID.toString()), MARKET_SO);
    svm.addProgramFromFile(address(EXTERNAL_PROGRAM_IDS.conditionalVault.toString()), VAULT_SO);
    svm.addProgramFromFile(address(EXTERNAL_PROGRAM_IDS.ammV04.toString()), AMM_SO);

    const payer = await Keypair.generate();
    svm.airdrop(payer.address, lamports(1_000_000_000_000n));

    const ctx = new MarketTestCtx(svm, payer);
    // Fabricate the program's BPF-Upgradeable-Loader `ProgramData` account so
    // `initConfig` (which requires the caller be the program's upgrade authority)
    // accepts the harness `payer` — the key every litesvm test signs initConfig
    // with. The upgrade authority stored here MUST equal that payer.
    await ctx.setUpgradeAuthority(payer.publicKey);
    return ctx;
  }

  /**
   * Fabricate (or overwrite) the program's `ProgramData` account so `authority`
   * is its on-chain upgrade authority. Builds the 45-byte
   * `UpgradeableLoaderState::ProgramData` metadata the program reads: `u32 LE
   * variant == 3 @0`, `u64 LE slot @4`, `Option::Some tag == 1 @12`, then the
   * 32-byte authority `@13..45`, at the canonical PDA under the BPF loader.
   */
  async setUpgradeAuthority(authority: Address): Promise<void> {
    const programData = (await pda.programData(MARKET_PROGRAM_ID)).address;
    const data = new Uint8Array(45);
    new DataView(data.buffer).setUint32(0, 3, true); // ProgramData variant
    data[12] = 1; // Option::Some
    data.set(authority.toBytes(), 13);
    this.putAccount(programData, data, BPF_UPGRADEABLE_LOADER_ID);
  }

  // ----- transaction submission --------------------------------------------

  /** Build, sign (payer + `signers`), bridge, and submit a single-ix tx. */
  async send(
    ix: TransactionInstruction,
    signers: Keypair[] = [],
  ): Promise<TransactionMetadata | FailedTransactionMetadata | null> {
    this.svm.expireBlockhash();
    const tx = new Transaction();
    tx.feePayer = this.payer.publicKey;
    tx.recentBlockhash = this.svm.latestBlockhash();
    tx.add(ix);
    await tx.sign(this.payer, ...signers);
    return this.svm.sendTransaction(await toLiteSvmTransaction(tx));
  }

  /** `send` + assert success. */
  async sendOk(ix: TransactionInstruction, signers: Keypair[], what: string): Promise<TransactionMetadata> {
    return expectOk(await this.send(ix, signers), what);
  }

  /**
   * Build, sign, bridge, and submit a MULTI-instruction tx, prepending a 1.4M-CU
   * `SetComputeUnitLimit` (the MetaDAO composition + `activate` CPIs exceed the
   * 200k default). Mirrors the Rust harness `send_many`.
   */
  async sendMany(
    ixs: TransactionInstruction[],
    signers: Keypair[] = [],
  ): Promise<TransactionMetadata | FailedTransactionMetadata | null> {
    this.svm.expireBlockhash();
    const tx = new Transaction();
    tx.feePayer = this.payer.publicKey;
    tx.recentBlockhash = this.svm.latestBlockhash();
    tx.add(ComputeBudgetProgram.setComputeUnitLimit({ units: 1_400_000 }));
    for (const ix of ixs) tx.add(ix);
    await tx.sign(this.payer, ...signers);
    return this.svm.sendTransaction(await toLiteSvmTransaction(tx));
  }

  /** `sendMany` + assert success. */
  async sendManyOk(
    ixs: TransactionInstruction[],
    signers: Keypair[],
    what: string,
  ): Promise<TransactionMetadata> {
    return expectOk(await this.sendMany(ixs, signers), what);
  }

  // ----- accessors ---------------------------------------------------------

  /** Raw account bytes (throws if absent). */
  getAccountData(key: Address | string): Uint8Array {
    const acct = this.svm.getAccount(address(key.toString()));
    if (!acct || !acct.exists) throw new Error(`account ${key} not found`);
    return acct.data;
  }

  readConfig(key: Address | string): Config {
    return decodeConfig(this.getAccountData(key));
  }
  readMarket(key: Address | string): Market {
    return decodeMarket(this.getAccountData(key));
  }
  readContribution(key: Address | string): Contribution {
    return decodeContribution(this.getAccountData(key));
  }

  /** SPL token amount (u64 @ offset 64) of a token account. */
  tokenBalance(key: Address | string): bigint {
    const data = this.getAccountData(key);
    return new DataView(data.buffer, data.byteOffset, data.length).getBigUint64(64, true);
  }

  /** Whether an account currently exists on-chain (a closed account returns false). */
  exists(key: Address | string): boolean {
    const acct = this.svm.getAccount(address(key.toString()));
    return !!acct && acct.exists;
  }

  /** Lamport balance of an account (0 if it does not exist). */
  lamportsOf(key: Address | string): bigint {
    const acct = this.svm.getAccount(address(key.toString()));
    return acct && acct.exists ? BigInt(acct.lamports) : 0n;
  }

  // ----- convenience cranks -------------------------------------------------

  /**
   * Build + send `close_market` (Ix 10): reclaim a settled market's rent to its
   * creator. Permissionless (payer signs the tx; `creator` need not sign).
   */
  async closeMarket(
    market: Address | string,
    creator: Address,
    what = "closeMarket",
  ): Promise<TransactionMetadata> {
    return this.sendOk(await closeMarket({ market: new Address(market.toString()), creator }), [], what);
  }

  // ----- fabrication -------------------------------------------------------

  /** Write an account owned by `owner` with `data` (rent-exempt). */
  private putAccount(key: Address, data: Uint8Array, owner: Address): void {
    this.svm.setAccount({
      address: address(key.toString()),
      data,
      executable: false,
      lamports: lamports(this.svm.minimumBalanceForRentExemption(BigInt(data.length))),
      programAddress: address(owner.toString()),
      space: BigInt(data.length),
    });
  }

  /** Fabricate an initialized SPL mint (authority = payer, supply 0). */
  async createMint(decimals: number): Promise<Address> {
    const mint = await Keypair.generate();
    this.putAccount(
      mint.publicKey,
      mintBytes(this.payer.publicKey.toBytes(), 0n, decimals),
      TOKEN_PROGRAM_ID,
    );
    return mint.publicKey;
  }

  /** Fabricate an initialized SPL token account on `mint` owned by `owner`. */
  async createTokenAccount(mint: Address, owner: Address, amount: bigint): Promise<Address> {
    const acct = await Keypair.generate();
    this.putAccount(
      acct.publicKey,
      tokenAccountBytes(mint.toBytes(), owner.toBytes(), amount),
      TOKEN_PROGRAM_ID,
    );
    return acct.publicKey;
  }

  /**
   * Fabricate a Kassandra-oracle-owned account carrying `optionsCount`/`phase`/
   * `resolvedOption` — mirrors the Rust harness `seed_kass_oracle` /
   * `set_oracle_resolved`. Pass `at` to re-seed an EXISTING oracle address in
   * place (moving a market's oracle to a new/terminal phase); omit it for a fresh
   * oracle at a random address. Returns the oracle address.
   */
  async seedOracle(
    optionsCount: number,
    phase: number,
    resolvedOption = 0,
    at?: Address,
  ): Promise<Address> {
    const key = at ?? (await Keypair.generate()).publicKey;
    this.putAccount(key, oracleBytes(optionsCount, phase, resolvedOption), KASSANDRA_PROGRAM_ID);
    return key;
  }

  /** Airdrop 1e12 lamports to `key` so it exists as a funded system account. */
  airdrop(key: Address): void {
    this.svm.airdrop(address(key.toString()), lamports(1_000_000_000_000n));
  }

  /** Advance the clock by `seconds` (and one slot). */
  warp(seconds: bigint): void {
    const c = this.svm.getClock();
    this.svm.setClock(
      new Clock(
        c.slot + 1n,
        c.epochStartTimestamp,
        c.epoch,
        c.leaderScheduleEpoch,
        c.unixTimestamp + seconds,
      ),
    );
  }

  /** A funded signer keypair. */
  async fundedKeypair(): Promise<Keypair> {
    const kp = await Keypair.generate();
    this.airdrop(kp.publicKey);
    return kp;
  }
}

// Re-export so tests can `expect(...)` without importing vitest twice.
export { expect };
