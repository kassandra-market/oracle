/**
 * T2 integration test (GATED) — the runner's REAL `AnthropicProvider` against a
 * local mock Anthropic server.
 *
 * This drives the runner binary's REAL provider HTTP+parse path (NOT `--mock`):
 *   1. start the mock Anthropic server (set "this oracle resolves to option N");
 *   2. spawn `kassandra-runner run --config <tmp>` with `ANTHROPIC_BASE_URL`
 *      pointed at the mock + a dummy `ANTHROPIC_API_KEY` (the provider requires
 *      a non-empty key) + a zero-fact config (so no real fact-fetch HTTP);
 *   3. assert the RunOutput's `option_index === N` (the mock's answer flowed
 *      through the real provider), and that the three hashes + the 97-byte
 *      `submit_ai_claim_payload_hex` are present and consistent.
 *   4. refusal arm: the mock refuses → the runner exits non-zero with a clear
 *      refusal error and emits NO claim.
 *
 * GATING: only included when `KASSANDRA_E2E=1` (see `vitest.config.ts`). It does
 * NOT need surfpool — only the built runner binary — so it SKIPS (not fails)
 * when the binary is missing. The default `pnpm test` never imports this file.
 */
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { afterAll, beforeAll, describe, expect, it } from "vitest";

import { MockAnthropic } from "./mock-anthropic.js";
import { runRunner, runnerAvailable, type RunOutput } from "./run-runner.js";

const ENABLED = process.env.KASSANDRA_E2E === "1" && runnerAvailable();

describe.skipIf(!ENABLED)("runner real AnthropicProvider against mock server", () => {
  let mock: MockAnthropic;
  let configPath: string;

  beforeAll(async () => {
    mock = await MockAnthropic.start();
    // A 2-option oracle with ZERO agreed facts (avoids real fact-fetch HTTP;
    // the runner accepts zero facts — see its run_core tests / prompt assembly).
    const dir = mkdtempSync(join(tmpdir(), "kassandra-runner-mock-"));
    configPath = join(dir, "config.json");
    writeFileSync(
      configPath,
      JSON.stringify({
        interpretation:
          "Resolve YES if the home team won; otherwise NO. No facts are supplied.",
        options_count: 2,
        option_labels: [
          { index: 0, label: "Yes" },
          { index: 1, label: "No" },
        ],
        facts: [],
      }),
    );
  });

  afterAll(async () => {
    await mock?.stop();
  });

  it("flows the mock's chosen option through the real provider + emits a 97-byte claim payload", async () => {
    const N = 1;
    mock.setOption(N, "claude-opus-4-8");

    const { code, stdout, stderr } = await runRunner(configPath, mock.baseUrl);
    expect(code, `runner failed: ${stderr}`).toBe(0);

    const out = JSON.parse(stdout) as RunOutput;

    // The mock's answer flowed through the REAL provider HTTP+parse path.
    expect(out.option_index).toBe(N);
    expect(out.resolved_model_id).toBe("claude-opus-4-8");

    // The three hashes are 32-byte hex.
    expect(out.model_id_hex).toMatch(/^[0-9a-f]{64}$/);
    expect(out.params_hash_hex).toMatch(/^[0-9a-f]{64}$/);
    expect(out.io_hash_hex).toMatch(/^[0-9a-f]{64}$/);

    // The submit_ai_claim payload is exactly 97 bytes (194 hex chars):
    // model_id[32] ++ params_hash[32] ++ io_hash[32] ++ option[1].
    const payload = out.submit_ai_claim_payload_hex;
    expect(payload).toMatch(/^[0-9a-f]{194}$/);
    expect(payload.slice(0, 64)).toBe(out.model_id_hex);
    expect(payload.slice(64, 128)).toBe(out.params_hash_hex);
    expect(payload.slice(128, 192)).toBe(out.io_hash_hex);
    expect(payload.slice(192, 194)).toBe(N.toString(16).padStart(2, "0"));

    // The runner actually hit the mock's /v1/messages with the real request body.
    expect(mock.requests.length).toBeGreaterThan(0);
    const req = mock.requests.at(-1)!;
    expect(req.model).toBe("claude-opus-4-8");
    expect(req.output_config).toBeDefined();
  }, 30_000);

  it("errors clearly (no claim) when the mock refuses", async () => {
    mock.setRefusal("policy", "Mock declined this request.");

    const { code, stdout, stderr } = await runRunner(configPath, mock.baseUrl);

    // Non-zero exit, no RunOutput JSON, and a clear refusal message.
    expect(code).not.toBe(0);
    expect(stdout.trim()).toBe("");
    expect(stderr.toLowerCase()).toContain("refusal");
  }, 30_000);
});
