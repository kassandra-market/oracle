/**
 * Paths, constants, binary discovery, and options for {@link MarketSurfpoolHarness}.
 *
 * Extracted from the original `surfpool/harness.ts`. Paths here are relative to
 * this file's location (`surfpool/harness/`), one level deeper than the original
 * single-file module.
 */
import { existsSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { Address } from "@solana/web3.js";

const here = dirname(fileURLToPath(import.meta.url));
/** The local SBF artifact (`just build` produces it). */
export const SO_PATH = resolve(here, "../../../../../../target/deploy/kassandra_markets_program.so");

/** The vendored MetaDAO `.so` fixtures (the LiteSVM harness `include_bytes!`s these). */
export const FIXTURES_DIR = resolve(here, "../../../../../../programs/markets/tests/fixtures");
/**
 * The two MetaDAO v0.4 programs (conditional_vault + amm) at their canonical
 * mainnet program ids. In OFFLINE mode these are deployed locally from the
 * vendored `.so` (same bytes/ids the LiteSVM `load_metadao()` uses) instead of
 * being lazily fetched from a mainnet fork.
 */
export const METADAO_FIXTURES: ReadonlyArray<{ id: string; file: string }> = [
  { id: "VLTX1ishMBbcX3rdBWGssxawAo1Q2X2qxYFYqiGodVg", file: "metadao_conditional_vault.so" },
  { id: "AMMyu265tkBpRW21iGQxKGLaves3gKm2JcMUqfXNSpqD", file: "metadao_amm.so" },
];

/** The deprecated (non-upgradeable) BPF loader: a program account IS its ELF. */
export const BPF_LOADER_2 = "BPFLoader2111111111111111111111111111111111";

/** The external Kassandra oracle program id (owns the accounts `seedOracle` writes). */
export const KASSANDRA_PROGRAM_ID = new Address("KassVxvXUEPr5apSr2MqiGva4VFtJXyYLLDFS3f83nY");

/** Generous lamport balance for every fabricated account (>> rent for ≤392 B). */
export const FAB_LAMPORTS = 1_000_000_000;

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
export function augmentedPath(): string {
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
   * WebSocket port. surfpool otherwise PINS the WS port to a fixed 8900
   * regardless of `--port`, so two simnets on different RPC ports still collide
   * on 8900; deriving it from the RPC port (default `port + 1`) lets instances
   * coexist. Pass explicitly to override.
   */
  wsPort?: number;
  /** Readiness timeout in ms (default 60000 — fork RPC is slower). */
  readyTimeoutMs?: number;
  /** Fork network (default "mainnet"): the deployed MetaDAO programs are lazily fetched. */
  fork?: "mainnet" | "devnet";
  /** Block-production mode (default "transaction" — one block per tx, deterministic). */
  blockProductionMode?: "transaction" | "clock";
  /** Slot time in ms for `clock` mode (`--slot-time`). */
  slotTimeMs?: number;
  /**
   * OFFLINE mode (default: `process.env.SURFPOOL_OFFLINE === "1"`). When on, boots
   * surfpool with `--offline` (NO remote datasource / mainnet fork) and deploys the
   * MetaDAO conditional-vault + AMM programs locally from the vendored `.so`
   * fixtures — fully self-contained, so CI needs no public mainnet RPC. When off
   * (the default for local dev), the mainnet-fork path is used unchanged.
   */
  offline?: boolean;
}
