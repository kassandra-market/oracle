/**
 * Shared helper for invoking the Kassandra runner binary's REAL
 * `AnthropicProvider` path against a (mock) Anthropic server — factored out of
 * the T2 integration test so the T3 lifecycle E2E reuses the exact same
 * invocation + env trick (the T2 reviewer flagged it was local to that test).
 *
 * The env trick (mirrors `runner/src/anthropic.rs::resolve_messages_url` + the
 * `--mock` guard): set `ANTHROPIC_BASE_URL` to the mock's base, a NON-EMPTY
 * `ANTHROPIC_API_KEY` (the provider requires a key), and force
 * `KASSANDRA_RUNNER_MOCK=""` so the fixed-output `MockProvider` path is NOT
 * taken — i.e. the runner exercises the genuine HTTP+parse provider against the
 * controllable mock responses.
 */
import { spawn } from "node:child_process";
import { existsSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));

/** The debug runner binary (`cargo build -p kassandra-runner`). */
export const RUNNER_BIN = resolve(here, "../../../target/debug/kassandra-runner");

/** True when the built runner binary is present (for `skipIf`). */
export function runnerAvailable(): boolean {
  return existsSync(RUNNER_BIN);
}

/** The RunOutput JSON shape the runner prints on stdout (mirror of `RunOutput`). */
export interface RunOutput {
  option_index: number;
  model_id_hex: string;
  params_hash_hex: string;
  io_hash_hex: string;
  submit_ai_claim_payload_hex: string;
  resolved_model_id: string;
  claim_pda_seeds?: { seed_prefix: string; oracle: string; proposer: string };
}

/** Captured stdout/stderr/exit-code of a runner invocation. */
export interface RunResult {
  code: number | null;
  stdout: string;
  stderr: string;
}

/** The runner config the CLI consumes (`--config`). */
export interface RunnerConfig {
  interpretation: string;
  options_count: number;
  option_labels?: Array<{ index: number; label: string }>;
  facts: Array<{ content_hash: string; uri: string }>;
  /** Optional — when present the runner echoes the AiClaim PDA seeds. */
  oracle?: string;
  /** Optional — paired with `oracle` for the PDA-seed echo. */
  proposer?: string;
}

/** Write a runner config to a fresh temp file and return its path. */
export function writeRunnerConfig(config: RunnerConfig): string {
  const dir = mkdtempSync(join(tmpdir(), "kassandra-runner-cfg-"));
  const path = join(dir, "config.json");
  writeFileSync(path, JSON.stringify(config));
  return path;
}

/**
 * Spawn `kassandra-runner run --config <configPath>` against `baseUrl` (the mock
 * Anthropic server), forcing the REAL provider path. Resolves with the captured
 * stdout/stderr/exit-code.
 */
export function runRunner(configPath: string, baseUrl: string): Promise<RunResult> {
  return new Promise((resolveRun, reject) => {
    const child = spawn(RUNNER_BIN, ["run", "--config", configPath], {
      env: {
        ...process.env,
        ANTHROPIC_BASE_URL: baseUrl,
        ANTHROPIC_API_KEY: "sk-mock-dummy-key",
        // Belt-and-suspenders: ensure the fixed-output MockProvider is NOT used.
        KASSANDRA_RUNNER_MOCK: "",
      },
      stdio: ["ignore", "pipe", "pipe"],
    });
    let stdout = "";
    let stderr = "";
    child.stdout.on("data", (c) => (stdout += c));
    child.stderr.on("data", (c) => (stderr += c));
    child.on("error", reject);
    child.on("close", (code) => resolveRun({ code, stdout, stderr }));
  });
}
