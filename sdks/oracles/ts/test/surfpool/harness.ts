/**
 * `SurfpoolHarness` — drive a headless surfpool simnet from the (gated) E2E
 * suite (Task T1).
 *
 * Responsibilities:
 *   1. spawn `surfpool start --no-tui --block-production-mode transaction`
 *      (a standalone simnet; no `--network`/`--rpc-url` fork);
 *   2. poll the RPC `getHealth` until ready (with a timeout);
 *   3. deploy the LOCAL `target/deploy/kassandra_program.so` at the FIXED
 *      program id {@link KASSANDRA_PROGRAM_ID} via the `surfnet_setAccount`
 *      cheatcode (writing the ELF as a non-upgradeable BPFLoader2 program
 *      account — surfpool then JIT-loads + executes it, exactly like
 *      `solana-test-validator --bpf-program`);
 *   4. expose the RPC url + a web3.js {@link Connection};
 *   5. tear the child process down on completion.
 *
 * It also exposes small cheatcode helpers (`setAccount`, `airdrop`,
 * `timeTravelToSlot`) the smoke/lifecycle tests use to fabricate state.
 *
 * GATING: {@link surfpoolBinary} returns `null` when the `surfpool` binary is
 * not found, so the suite can SKIP (not fail) when surfpool is unavailable. The
 * default `pnpm test` never imports this file (see `vitest.config.ts`).
 */
import { spawn, type ChildProcess } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { Connection } from "@solana/web3.js";

import { KASSANDRA_PROGRAM_ID } from "../../src/constants.js";

const here = dirname(fileURLToPath(import.meta.url));
/** The local SBF artifact (`just build` produces it). */
export const SO_PATH = resolve(here, "../../../../../target/deploy/kassandra_program.so");

/** The deprecated (non-upgradeable) BPF loader: a program account IS its ELF. */
const BPF_LOADER_2 = "BPFLoader2111111111111111111111111111111111";

/** The Clock sysvar — the account the program's `now()` reads `unix_timestamp` from. */
const CLOCK_SYSVAR = "SysvarC1ock11111111111111111111111111111111";

/** Candidate locations for the surfpool binary, in priority order. */
function surfpoolCandidates(): string[] {
  const fromEnv = process.env.SURFPOOL_BIN;
  return [
    ...(fromEnv ? [fromEnv] : []),
    join(homedir(), ".local/bin/surfpool"),
    "/usr/local/bin/surfpool",
    "/opt/homebrew/bin/surfpool",
  ];
}

/** Resolve the surfpool binary path, or `null` if it cannot be found. */
export function surfpoolBinary(): string | null {
  for (const c of surfpoolCandidates()) {
    if (existsSync(c)) return c;
  }
  return null;
}

/** True when both surfpool and the built `.so` are present (for `skipIf`). */
export function surfpoolReady(): boolean {
  return surfpoolBinary() !== null && existsSync(SO_PATH);
}

/** PATH augmented with the usual local solana/surfpool bin dirs. */
function augmentedPath(): string {
  const extra = [
    join(homedir(), ".local/bin"),
    join(homedir(), ".local/share/solana/install/active_release/bin"),
  ];
  return [...extra, process.env.PATH ?? ""].join(":");
}

export interface HarnessOptions {
  /** RPC port (default 8899). */
  port?: number;
  /**
   * Websocket (pubsub) port (`--ws-port`). Surfpool defaults its WS listener to a
   * FIXED 8900 regardless of the RPC port, so set this (conventionally RPC port +
   * 1) when a test needs `accountSubscribe`/`programSubscribe` on a known port —
   * e.g. the indexer e2e points the price subscriber's `SOLANA_WS_URL` here.
   * Exposed as {@link SurfpoolHarness.wsUrl} once set.
   */
  wsPort?: number;
  /** Readiness timeout in ms (default 30000). */
  readyTimeoutMs?: number;
  /**
   * Fork a live cluster so its DEPLOYED programs/accounts are lazily fetchable
   * (T4: the MetaDAO conditional-vault / AMM / futarchy programs). Passes
   * `--network <fork>` to surfpool (e.g. `"mainnet"`). When unset the simnet
   * still boots against surfpool's default datasource but the core path stays
   * local (T1-T3). Forking needs network reachable + is slower (RPC fetches).
   */
  fork?: "mainnet" | "devnet";
  /**
   * Block-production mode. Defaults to `"transaction"` (one block per tx, the
   * deterministic mode T1-T3 use). `"clock"` produces blocks on a wall-clock
   * timer so the on-chain **execution** slot (`Clock::get()?.slot`) advances over
   * real time — needed for the v0.4 AMM's SLOT-based crank rate-limit
   * (`surfnet_timeTravel` moves only `getSlot`/`unix_timestamp`, NOT the slot the
   * program sees during execution). Pair with a small {@link slotTimeMs}.
   */
  blockProductionMode?: "transaction" | "clock";
  /** Slot time in ms for `clock` mode (`--slot-time`); smaller ⇒ faster slots. */
  slotTimeMs?: number;
}

export class SurfpoolHarness {
  private constructor(
    private readonly child: ChildProcess,
    readonly rpcUrl: string,
    readonly connection: Connection,
    /** Effective slot time (s) — surfpool maps unix_timestamp ≈ slot × slotTime,
     * so timeTravel jumps are sized from this. Default 0.4 (surfpool's default). */
    private readonly slotTimeSec: number = 0.4,
    /** The pubsub websocket url, when a {@link HarnessOptions.wsPort} was set. */
    readonly wsUrl: string | undefined = undefined,
  ) {}

  /** Spawn surfpool, wait for readiness, and deploy the program. */
  static async start(opts: HarnessOptions = {}): Promise<SurfpoolHarness> {
    const bin = surfpoolBinary();
    if (!bin) throw new Error("surfpool binary not found (set SURFPOOL_BIN or install it)");
    if (!existsSync(SO_PATH)) {
      throw new Error(`Missing program artifact at ${SO_PATH}. Run \`just build\` first.`);
    }

    const port = opts.port ?? 8899;
    const rpcUrl = `http://127.0.0.1:${port}`;

    const mode = opts.blockProductionMode ?? "transaction";
    const child = spawn(
      bin,
      [
        "start",
        "--no-tui",
        "--block-production-mode",
        mode,
        ...(opts.slotTimeMs ? ["--slot-time", String(opts.slotTimeMs)] : []),
        "--no-deploy",
        ...(opts.fork ? ["--network", opts.fork] : []),
        "--port",
        String(port),
        ...(opts.wsPort ? ["--ws-port", String(opts.wsPort)] : []),
      ],
      {
        stdio: ["ignore", "ignore", "ignore"],
        env: { ...process.env, PATH: augmentedPath() },
        detached: false,
      },
    );

    const connection = new Connection(rpcUrl, "confirmed");
    const harness = new SurfpoolHarness(
      child,
      rpcUrl,
      connection,
      opts.slotTimeMs ? opts.slotTimeMs / 1000 : 0.4,
      opts.wsPort ? `ws://127.0.0.1:${opts.wsPort}` : undefined,
    );

    try {
      await harness.waitForHealth(opts.readyTimeoutMs ?? 30_000);
      await harness.deployProgram();
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

  /**
   * Write the local ELF at the fixed program id as a non-upgradeable BPFLoader2
   * program account. surfpool's `surfnet_setAccount` takes the account `data` as
   * a HEX string.
   */
  private async deployProgram(): Promise<void> {
    const elfHex = readFileSync(SO_PATH).toString("hex");
    await this.setAccount(KASSANDRA_PROGRAM_ID.toString(), {
      lamports: 5_000_000_000,
      owner: BPF_LOADER_2,
      executable: true,
      data: elfHex,
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
  async airdrop(pubkey: string, lamports: number): Promise<void> {
    await this.rpc("requestAirdrop", [pubkey, lamports]);
    const deadline = Date.now() + 10_000;
    while (Date.now() < deadline) {
      const bal = await this.rpc<{ value: number }>("getBalance", [pubkey]);
      if (bal.value >= lamports) return;
      await new Promise((r) => setTimeout(r, 200));
    }
    throw new Error(`airdrop to ${pubkey} did not settle`);
  }

  /** `surfnet_timeTravel` cheatcode: jump the clock to `absoluteSlot` (for T3). */
  async timeTravelToSlot(absoluteSlot: number): Promise<void> {
    await this.rpc("surfnet_timeTravel", [{ absoluteSlot }]);
  }

  /**
   * Read the on-chain Clock sysvar's `unix_timestamp` — the EXACT value the
   * program's `now()` reads (`Clock::get()?.unix_timestamp`), so phase-window
   * gates (`now >= phase_ends_at`) are checked against it.
   */
  async clockUnixTimestamp(): Promise<bigint> {
    const info = await this.rpc<{ value: { data: [string, string] } | null }>("getAccountInfo", [
      CLOCK_SYSVAR,
      { encoding: "base64" },
    ]);
    const b64 = info.value?.data?.[0];
    if (!b64) throw new Error("Clock sysvar not readable");
    // Clock layout: slot(8) ++ epoch_start_ts(8) ++ epoch(8) ++ leader_sched_epoch(8)
    // ++ unix_timestamp(8, i64 LE) — unix_timestamp at offset 32.
    return Buffer.from(b64, "base64").readBigInt64LE(32);
  }

  /** Current absolute slot (`getSlot`). */
  async currentSlot(): Promise<number> {
    return this.rpc<number>("getSlot", []);
  }

  /**
   * Advance the on-chain clock until `unix_timestamp >= targetUnix` by jumping
   * the absolute slot forward — surfpool's `surfnet_timeTravel({absoluteSlot})`
   * moves `unix_timestamp` at ~0.4 s/slot (empirically verified in T3). This is
   * the mechanism that crosses the program's phase windows. Verifies + re-jumps
   * until satisfied (per-tx slot drift is small relative to the windows).
   */
  async advanceToUnix(targetUnix: bigint): Promise<void> {
    for (let attempt = 0; attempt < 12; attempt++) {
      const cur = await this.clockUnixTimestamp();
      if (cur >= targetUnix) return;
      const slot = await this.currentSlot();
      const needSec = Number(targetUnix - cur);
      // unix_timestamp ≈ slot × slotTime; divide by 0.95×slotTime + buffer so we
      // overshoot (works for both the 0.4s default and a fast clock-mode slot-time).
      const slotJump = Math.ceil(needSec / (this.slotTimeSec * 0.95)) + 50;
      await this.timeTravelToSlot(slot + slotJump);
      await new Promise((r) => setTimeout(r, 150));
    }
    const cur = await this.clockUnixTimestamp();
    if (cur < targetUnix) {
      throw new Error(`advanceToUnix: clock ${cur} still < target ${targetUnix}`);
    }
  }

  /** Poll `getSignatureStatuses` until the tx confirms (throws on error/timeout). */
  async confirmSignature(sig: string, timeoutMs = 20_000): Promise<void> {
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
      await new Promise((r) => setTimeout(r, 150));
    }
    throw new Error(`tx ${sig} not confirmed within ${timeoutMs}ms`);
  }

  /** Kill the surfpool child process. */
  async teardown(): Promise<void> {
    if (this.child.exitCode !== null) return;
    await new Promise<void>((resolveDone) => {
      this.child.once("exit", () => resolveDone());
      this.child.kill("SIGKILL");
      // Safety net if `exit` never fires.
      setTimeout(() => resolveDone(), 2_000);
    });
  }
}

// ---------------------------------------------------------------------------
// SPL layout fabrication (mirrors `test/e2e.test.ts`): minimal canonical Mint /
// token-Account byte layouts, written token-program-owned via `setAccount`.
// ---------------------------------------------------------------------------

const MINT_LEN = 82;

/** Pack an 82-byte SPL `Mint` (COption authority tag 1 = Some). */
export function mintBytes(authority: Uint8Array, supply: bigint, decimals: number): Uint8Array {
  const data = new Uint8Array(MINT_LEN);
  const dv = new DataView(data.buffer);
  dv.setUint32(0, 1, true); // mint_authority COption tag = Some
  data.set(authority, 4);
  dv.setBigUint64(36, supply, true);
  data[44] = decimals;
  data[45] = 1; // is_initialized
  return data;
}

const TOKEN_ACCOUNT_LEN = 165;

/** Pack a 165-byte SPL token `Account` holding `amount` of `mint`, owned by `owner`. */
export function tokenAccountBytes(mint: Uint8Array, owner: Uint8Array, amount: bigint): Uint8Array {
  const data = new Uint8Array(TOKEN_ACCOUNT_LEN);
  const dv = new DataView(data.buffer);
  data.set(mint, 0); // mint
  data.set(owner, 32); // owner
  dv.setBigUint64(64, amount, true); // amount
  data[108] = 1; // state = Initialized (delegate/is_native/close_authority COptions stay None)
  return data;
}

/** Read the `amount` (u64 @ offset 64) out of SPL token-account bytes. */
export function tokenAccountAmount(data: Uint8Array): bigint {
  return new DataView(data.buffer, data.byteOffset, data.length).getBigUint64(64, true);
}

/** Hex-encode a byte array for the `surfnet_setAccount` `data` field. */
export function toHex(bytes: Uint8Array): string {
  return Buffer.from(bytes).toString("hex");
}
