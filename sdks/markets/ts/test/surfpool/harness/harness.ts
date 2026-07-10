/**
 * `MarketSurfpoolHarness` — drive a headless surfpool MAINNET-FORK simnet from the
 * (gated) kassandra-market E2E suite.
 *
 * This is the market-flavoured sibling of `../../../../kassandra/sdks/oracles/ts/test/surfpool/harness/harness.ts`.
 * Responsibilities:
 *   1. spawn `surfpool start --no-tui --block-production-mode <mode> --no-deploy
 *      --network mainnet --port <port>` (a mainnet FORK: the deployed MetaDAO
 *      conditional-vault `VLTX…` + AMM v0.4 `AMMyu…` programs are lazily fetched
 *      from the fork on first touch — no local fixtures);
 *   2. poll the RPC `getHealth` until ready (with a timeout);
 *   3. deploy the LOCAL `target/deploy/kassandra_markets_program.so` at the FIXED
 *      {@link MARKET_PROGRAM_ID} via the `surfnet_setAccount` cheatcode (writing the
 *      ELF as a non-upgradeable BPFLoader2 program account);
 *   4. expose a web3.js {@link Connection} + market-specific fabrication helpers
 *      (SPL mints/token accounts, a Kassandra-oracle seeder, `sendIx`);
 *   5. tear the child process down on completion.
 *
 * GATING: {@link surfpoolBinary} returns `null` when the `surfpool` binary is not
 * found, so the suite SKIPS (not fails) when surfpool is unavailable. The default
 * `pnpm test` never imports this file (see `vitest.config.ts`).
 */
import { spawn, type ChildProcess } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";

import {
  Address,
  ComputeBudgetProgram,
  Connection,
  Keypair,
  Transaction,
  TransactionInstruction,
} from "@solana/web3.js";

import { BPF_UPGRADEABLE_LOADER_ID, MARKET_PROGRAM_ID, TOKEN_PROGRAM_ID } from "../../../src/constants.js";
import * as pda from "../../../src/pda.js";
import { mintBytes, oracleBytes, tokenAccountAmount, tokenAccountBytes } from "../../spl-layout.js";

import {
  augmentedPath,
  BPF_LOADER_2,
  FAB_LAMPORTS,
  FIXTURES_DIR,
  type HarnessOptions,
  KASSANDRA_PROGRAM_ID,
  METADAO_FIXTURES,
  SO_PATH,
  surfpoolBinary,
} from "./config.js";
import { toHex } from "./encoding.js";

export class MarketSurfpoolHarness {
  private constructor(
    private readonly child: ChildProcess,
    readonly rpcUrl: string,
    readonly connection: Connection,
  ) {}

  /** Spawn surfpool, wait for readiness, and deploy the program. */
  static async start(opts: HarnessOptions = {}): Promise<MarketSurfpoolHarness> {
    const bin = surfpoolBinary();
    if (!bin) throw new Error("surfpool binary not found (set SURFPOOL_BIN or install it)");
    if (!existsSync(SO_PATH)) {
      throw new Error(`Missing program artifact at ${SO_PATH}. Run \`just build\` first.`);
    }

    const port = opts.port ?? 8899;
    const wsPort = opts.wsPort ?? port + 1;
    const rpcUrl = `http://127.0.0.1:${port}`;
    const mode = opts.blockProductionMode ?? "transaction";
    const fork = opts.fork ?? "mainnet";
    // OFFLINE default from the env, so ALL callers (SDK e2e + Playwright global-setup)
    // pick it up on CI without a code change; an explicit `opts.offline` overrides.
    const offline = opts.offline ?? process.env.SURFPOOL_OFFLINE === "1";

    // Datasource selection.
    //   OFFLINE:  `--offline` (no remote RPC) — the MetaDAO programs are deployed
    //             locally below; mutually exclusive with any datasource flag.
    //   FORK:     a custom mainnet RPC (`--rpc-url`, env `SURFPOOL_DATASOURCE_RPC_URL`)
    //             takes precedence over the predefined `--network mainnet` (public RPC);
    //             the deployed MetaDAO programs are lazily fetched from the fork.
    let datasourceArgs: string[];
    if (offline) {
      datasourceArgs = ["--offline"];
    } else {
      const datasourceRpc = process.env.SURFPOOL_DATASOURCE_RPC_URL;
      datasourceArgs = datasourceRpc ? ["--rpc-url", datasourceRpc] : ["--network", fork];
    }

    const child = spawn(
      bin,
      [
        "start",
        "--no-tui",
        "--block-production-mode",
        mode,
        ...(opts.slotTimeMs ? ["--slot-time", String(opts.slotTimeMs)] : []),
        "--no-deploy",
        ...datasourceArgs,
        "--port",
        String(port),
        "--ws-port",
        String(wsPort),
      ],
      {
        stdio: ["ignore", "ignore", "ignore"],
        env: { ...process.env, PATH: augmentedPath() },
        detached: false,
      },
    );

    const connection = new Connection(rpcUrl, "confirmed");
    const harness = new MarketSurfpoolHarness(child, rpcUrl, connection);

    try {
      await harness.waitForHealth(opts.readyTimeoutMs ?? 60_000);
      await harness.deployProgram();
      // Offline has no mainnet fork to lazily fetch the MetaDAO programs from, so
      // deploy them locally from the vendored fixtures (same ids/bytes as LiteSVM).
      if (offline) await harness.deployMetadaoFixtures();
    } catch (e) {
      await harness.teardown();
      throw e;
    }
    return harness;
  }

  /** A raw JSON-RPC call (used for the `surfnet_*` cheatcodes). */
  async rpc<T = unknown>(method: string, params: unknown[]): Promise<T> {
    const res = await fetch(this.rpcUrl, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ jsonrpc: "2.0", id: 1, method, params }),
    });
    const json = (await res.json()) as { result?: T; error?: { message: string } };
    if (json.error) throw new Error(`${method} failed: ${json.error.message}`);
    return json.result as T;
  }

  /** Poll `getHealth` until "ok" or the timeout elapses. */
  private async waitForHealth(timeoutMs: number): Promise<void> {
    const deadline = Date.now() + timeoutMs;
    let lastErr = "";
    while (Date.now() < deadline) {
      if (this.child.exitCode !== null) {
        throw new Error(`surfpool exited early (code ${this.child.exitCode})`);
      }
      try {
        const health = await this.rpc<string>("getHealth", []);
        if (health === "ok") return;
      } catch (e) {
        lastErr = String(e);
      }
      await new Promise((r) => setTimeout(r, 250));
    }
    throw new Error(`surfpool did not become healthy within ${timeoutMs}ms (${lastErr})`);
  }

  /** Write the local ELF at the fixed program id as a BPFLoader2 program account. */
  private async deployProgram(): Promise<void> {
    const elfHex = readFileSync(SO_PATH).toString("hex");
    await this.setAccount(MARKET_PROGRAM_ID.toString(), {
      lamports: 5_000_000_000,
      owner: BPF_LOADER_2,
      executable: true,
      data: elfHex,
    });
  }

  /**
   * OFFLINE deploy of the two MetaDAO v0.4 programs (conditional_vault + amm) at
   * their canonical mainnet ids from the vendored `.so` fixtures — mirrors
   * {@link deployProgram} (BPFLoader2 executable + hex ELF) and the LiteSVM
   * `load_metadao()`. This replaces the mainnet-fork lazy fetch so the node is
   * fully self-contained.
   */
  private async deployMetadaoFixtures(): Promise<void> {
    for (const { id, file } of METADAO_FIXTURES) {
      const path = join(FIXTURES_DIR, file);
      if (!existsSync(path)) throw new Error(`Missing MetaDAO fixture at ${path}.`);
      const elfHex = readFileSync(path).toString("hex");
      await this.setAccount(id, {
        lamports: 5_000_000_000,
        owner: BPF_LOADER_2,
        executable: true,
        data: elfHex,
      });
    }
  }

  /**
   * Fabricate (or overwrite) the program's BPF-Upgradeable-Loader `ProgramData`
   * account so `authority` is its on-chain upgrade authority — the precondition
   * for `initConfig`, which requires the caller be that authority. Call this with
   * the initConfig payer BEFORE sending initConfig.
   *
   * Builds the 45-byte `UpgradeableLoaderState::ProgramData` metadata the program
   * reads: `u32 LE variant == 3 @0`, `u64 LE slot @4`, `Option::Some tag == 1 @12`,
   * then the 32-byte authority `@13..45`, at the canonical PDA under the BPF
   * upgradeable loader (which also owns it). The program was deployed here under
   * the non-upgradeable BPFLoader2, so this ProgramData does not otherwise exist.
   */
  async setUpgradeAuthority(authority: Address): Promise<void> {
    const programData = (await pda.programData(MARKET_PROGRAM_ID)).address;
    const data = new Uint8Array(45);
    new DataView(data.buffer).setUint32(0, 3, true); // ProgramData variant
    data[12] = 1; // Option::Some
    data.set(authority.toBytes(), 13);
    await this.setAccount(programData.toString(), {
      lamports: FAB_LAMPORTS,
      owner: BPF_UPGRADEABLE_LOADER_ID.toString(),
      executable: false,
      data: toHex(data),
    });
  }

  /** `surfnet_setAccount` cheatcode: write/overwrite an account at `pubkey`. */
  async setAccount(
    pubkey: string,
    update: { lamports?: number; owner?: string; executable?: boolean; data?: string },
  ): Promise<void> {
    await this.rpc("surfnet_setAccount", [pubkey, update]);
  }

  /** Airdrop `lamports` to `pubkey` and wait until the balance reflects it. */
  async airdrop(pubkey: string, lamports = 5_000_000_000): Promise<void> {
    await this.rpc("requestAirdrop", [pubkey, lamports]);
    const deadline = Date.now() + 15_000;
    while (Date.now() < deadline) {
      const bal = await this.rpc<{ value: number }>("getBalance", [pubkey]);
      if (bal.value >= lamports) return;
      await new Promise((r) => setTimeout(r, 200));
    }
    throw new Error(`airdrop to ${pubkey} did not settle`);
  }

  /** Poll `getSignatureStatuses` until the tx confirms (throws on error/timeout). */
  async confirmSignature(sig: string, timeoutMs = 45_000): Promise<void> {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
      const r = await this.rpc<{
        value: Array<{ confirmationStatus?: string; err: unknown } | null>;
      }>("getSignatureStatuses", [[sig], { searchTransactionHistory: true }]);
      const st = r.value?.[0];
      if (st) {
        if (st.err) throw new Error(`tx ${sig} failed: ${JSON.stringify(st.err)}`);
        if (st.confirmationStatus === "confirmed" || st.confirmationStatus === "finalized") return;
      }
      await new Promise((r) => setTimeout(r, 200));
    }
    throw new Error(`tx ${sig} not confirmed within ${timeoutMs}ms`);
  }

  /** Kill the surfpool child process. */
  async teardown(): Promise<void> {
    if (this.child.exitCode !== null) return;
    await new Promise<void>((resolveDone) => {
      this.child.once("exit", () => resolveDone());
      this.child.kill("SIGKILL");
      setTimeout(() => resolveDone(), 2_000);
    });
  }

  // ----- accessors ---------------------------------------------------------

  /** Raw account bytes over RPC (null if the account does not exist). */
  async getAccountData(address: Address | string): Promise<Uint8Array | null> {
    const info = await this.connection.getAccountInfo(
      address instanceof Address ? address : new Address(address),
    );
    return info && info.data.length > 0 ? info.data : null;
  }

  /** Poll `getAccountInfo` until the account exists, returning its raw bytes. */
  async waitForAccount(address: Address | string, timeoutMs = 20_000): Promise<Uint8Array> {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
      const data = await this.getAccountData(address);
      if (data) return data;
      await new Promise((r) => setTimeout(r, 200));
    }
    throw new Error(`account ${String(address)} did not appear within ${timeoutMs}ms`);
  }

  /** SPL token amount (u64 @ offset 64) of a token account, over RPC. */
  async tokenBalance(address: Address | string): Promise<bigint> {
    const data = await this.waitForAccount(address);
    return tokenAccountAmount(data);
  }

  // ----- transaction submission --------------------------------------------

  /**
   * Build a legacy tx (feePayer = `payer`, blockhash from `getLatestBlockhash`),
   * optionally prepend a `SetComputeUnitLimit`, sign with `payer` + `signers`,
   * send raw, and confirm. Returns the signature.
   */
  async sendIx(
    payer: Keypair,
    ixs: TransactionInstruction[],
    signers: Keypair[] = [],
    computeUnits?: number,
  ): Promise<string> {
    const tx = new Transaction();
    tx.feePayer = payer.publicKey;
    tx.recentBlockhash = (await this.connection.getLatestBlockhash()).blockhash;
    if (computeUnits) tx.add(ComputeBudgetProgram.setComputeUnitLimit({ units: computeUnits }));
    for (const ix of ixs) tx.add(ix);
    await tx.sign(payer, ...signers);
    const sig = await this.connection.sendRawTransaction(await tx.serialize(), {
      skipPreflight: false,
    });
    await this.confirmSignature(sig);
    return sig;
  }

  // ----- fabrication -------------------------------------------------------

  /** Fabricate an initialized SPL mint (authority = `authority`, supply 0). */
  async createMint(decimals: number, authority: Address): Promise<Address> {
    const mint = await Keypair.generate();
    await this.setAccount(mint.publicKey.toString(), {
      lamports: FAB_LAMPORTS,
      owner: TOKEN_PROGRAM_ID.toString(),
      executable: false,
      data: toHex(mintBytes(authority.toBytes(), 0n, decimals)),
    });
    return mint.publicKey;
  }

  /** Fabricate an SPL token account on `mint` owned by `owner` at a RANDOM address. */
  async createTokenAccount(mint: Address, owner: Address, amount: bigint): Promise<Address> {
    const acct = await Keypair.generate();
    await this.setAccount(acct.publicKey.toString(), {
      lamports: FAB_LAMPORTS,
      owner: TOKEN_PROGRAM_ID.toString(),
      executable: false,
      data: toHex(tokenAccountBytes(mint.toBytes(), owner.toBytes(), amount)),
    });
    return acct.publicKey;
  }

  /** Fabricate a FUNDED token account at the derived ATA of (`owner`, `mint`). */
  async fundTokenAccount(mint: Address, owner: Address, amount: bigint): Promise<Address> {
    const ata = (await pda.associatedTokenAccount(owner, mint)).address;
    await this.setAccount(ata.toString(), {
      lamports: FAB_LAMPORTS,
      owner: TOKEN_PROGRAM_ID.toString(),
      executable: false,
      data: toHex(tokenAccountBytes(mint.toBytes(), owner.toBytes(), amount)),
    });
    return ata;
  }

  /**
   * Fabricate a Kassandra-oracle-owned account (owner = {@link KASSANDRA_PROGRAM_ID})
   * carrying `optionsCount`/`phase`/`resolvedOption`. Pass `at` to re-seed an
   * existing oracle in place; omit for a fresh oracle at a random address.
   */
  async seedOracle(params: {
    optionsCount?: number;
    phase: number;
    resolvedOption?: number;
    at?: Address;
  }): Promise<Address> {
    const key = params.at ?? (await Keypair.generate()).publicKey;
    const data = oracleBytes(params.optionsCount ?? 2, params.phase, params.resolvedOption ?? 0xff);
    await this.setAccount(key.toString(), {
      lamports: FAB_LAMPORTS,
      owner: KASSANDRA_PROGRAM_ID.toString(),
      executable: false,
      data: toHex(data),
    });
    return key;
  }

  /** Re-seed `oracle` to Resolved (phase 7) with the winning option. */
  async setOracleResolved(oracle: Address, resolvedOption: number): Promise<void> {
    await this.seedOracle({ phase: 7, resolvedOption, at: oracle });
  }

  /** Re-seed `oracle` to a new phase (keeps a sentinel resolved_option). */
  async setOraclePhase(oracle: Address, phase: number): Promise<void> {
    await this.seedOracle({ phase, at: oracle });
  }
}
