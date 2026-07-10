/**
 * Indexer integration test — "test the indexer" against a surfpool MAINNET FORK.
 *
 * Boots the harness (deploys the program), seeds Config + an oracle + a Funding
 * market (+ the creator's Contribution) via `sendIx`, then BUILDS and SPAWNS the
 * real `kassandra-market-indexer` binary pointed at surfpool (http + ws), with a
 * short `INDEXER_RECONCILE_MS` (surfpool doesn't serve `programSubscribe`, so the
 * indexer's getProgramAccounts reconcile loop is the freshness path). It then
 * asserts the indexer's HTTP API reflects the on-chain state AND relays a real
 * signed transaction that reaches `confirmed`:
 *   GET /health, /api/config, /api/markets, /api/markets/:pubkey (+contribution
 *   +oracle, reserves null for Funding), /api/account/:pubkey (present + 404),
 *   /api/blockhash, POST /api/transaction (a signed contribute) → GET
 *   /api/transaction/:sig → confirmed.
 */
import { spawn, type ChildProcess } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { Transaction } from "@solana/web3.js";
import { afterAll, beforeAll, describe, expect, it } from "vitest";

import { Phase } from "../../src/accounts/oracle.js";
import { MarketStatus } from "../../src/constants.js";
import { contribute, createMarket, initConfig } from "../../src/instructions/index.js";
import * as pda from "../../src/pda.js";
import { MARKET_PROGRAM_ID } from "../../src/constants.js";

import { Keypair, MarketSurfpoolHarness, surfpoolReady } from "./harness/index.js";

const ENABLED = process.env.KASSANDRA_MARKET_E2E === "1" && surfpoolReady();

const SURF_PORT = 18981; // RPC (ws = 18982)
const INDEXER_PORT = 19081;

const MIN_LIQ = 1_000_000_000n; // 1 KASS floor
const PRESEED = 600_000_000n; // creator seed (below floor → market stays Funding)
const WALLET_KASS = 10n ** 15n;
const RELAY_CONTRIB = 50_000_000_000n; // 50 KASS contributed THROUGH the indexer relay

const here = dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = resolve(here, "../../../");
const INDEXER_BIN = resolve(REPO_ROOT, "target/debug/kassandra-market-indexer");
const BASE = `http://127.0.0.1:${INDEXER_PORT}`;

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

async function getJson<T>(path: string): Promise<{ status: number; body: T }> {
  const res = await fetch(`${BASE}${path}`);
  const body = res.status === 204 ? (null as T) : ((await res.json()) as T);
  return { status: res.status, body };
}

async function pollJson<T>(
  path: string,
  pred: (v: T) => boolean,
  timeoutMs = 20_000,
): Promise<T> {
  const deadline = Date.now() + timeoutMs;
  let last: unknown;
  while (Date.now() < deadline) {
    const { status, body } = await getJson<T>(path);
    last = { status, body };
    if (status === 200 && pred(body)) return body;
    await sleep(300);
  }
  throw new Error(`poll ${path} timed out; last = ${JSON.stringify(last)}`);
}

describe.skipIf(!ENABLED)("indexer integration: index + relay against surfpool", () => {
  let h: MarketSurfpoolHarness;
  let indexer: ChildProcess;
  let indexerLog = "";

  // Seeded handles for the assertions.
  let wallet: Keypair;
  let kassMint: string;
  let market: string;
  let oracle: string;
  let walletKassAta: string;

  beforeAll(async () => {
    h = await MarketSurfpoolHarness.start({ port: SURF_PORT, fork: "mainnet" });

    // ── Seed Config + a funded wallet + a Funding market via sendIx ──────────
    wallet = await Keypair.generate();
    await h.airdrop(wallet.publicKey.toString(), 50_000_000_000);
    // The wallet pays init_config, so it must be the program's upgrade authority.
    await h.setUpgradeAuthority(wallet.publicKey);
    const kass = await h.createMint(9, wallet.publicKey);
    kassMint = kass.toString();
    const ata = await h.fundTokenAccount(kass, wallet.publicKey, WALLET_KASS);
    walletKassAta = ata.toString();
    const feeDestination = await h.createTokenAccount(kass, wallet.publicKey, 0n);
    await h.sendIx(wallet, [
      await initConfig({
        payer: wallet.publicKey,
        kassMint: kass,
        authority: wallet.publicKey,
        minLiquidity: MIN_LIQ,
        feeBps: 100,
        feeDestination,
      }),
    ]);

    const oracleAddr = await h.seedOracle({ optionsCount: 2, phase: Phase.Proposal });
    oracle = oracleAddr.toString();
    const marketAddr = (await pda.market(oracleAddr, 0)).address;
    market = marketAddr.toString();
    await h.sendIx(wallet, [
      await createMarket({
        creator: wallet.publicKey,
        oracle: oracleAddr,
        kassMint: kass,
        creatorKassAta: ata,
        seedAmount: PRESEED,
        outcomeIndex: 0,
      }),
    ]);

    // ── Build + spawn the indexer binary pointed at surfpool ────────────────
    indexer = spawn(INDEXER_BIN, [], {
      cwd: REPO_ROOT,
      env: {
        ...process.env,
        SOLANA_RPC_URL: h.rpcUrl,
        SOLANA_WS_URL: `ws://127.0.0.1:${SURF_PORT + 1}`,
        PORT: String(INDEXER_PORT),
        MARKET_PROGRAM_ID: MARKET_PROGRAM_ID.toString(),
        INDEXER_RECONCILE_MS: "1000",
        RUST_LOG: "info",
      },
      stdio: ["ignore", "pipe", "pipe"],
    });
    indexer.stdout?.on("data", (d) => (indexerLog += String(d)));
    indexer.stderr?.on("data", (d) => (indexerLog += String(d)));

    // Poll /health until the gateway is up.
    const deadline = Date.now() + 30_000;
    let up = false;
    while (Date.now() < deadline) {
      if (indexer.exitCode !== null) {
        throw new Error(`indexer exited early (code ${indexer.exitCode}):\n${indexerLog}`);
      }
      try {
        const res = await fetch(`${BASE}/health`);
        if (res.ok) {
          up = true;
          break;
        }
      } catch {
        // not up yet
      }
      await sleep(250);
    }
    if (!up) throw new Error(`indexer /health never came up:\n${indexerLog}`);
  }, 120_000);

  afterAll(async () => {
    if (indexer && indexer.exitCode === null) {
      await new Promise<void>((res) => {
        indexer.once("exit", () => res());
        indexer.kill("SIGKILL");
        setTimeout(res, 2_000);
      });
    }
    await h?.teardown();
  });

  it("GET /api/config reflects the seeded Config", async () => {
    const cfg = await pollJson<{ kassMint: string; minLiquidity: string; feeBps: number }>(
      "/api/config",
      (c) => c.kassMint === kassMint,
    );
    expect(cfg.kassMint).toBe(kassMint);
    expect(cfg.minLiquidity).toBe(MIN_LIQ.toString());
    expect(cfg.feeBps).toBe(100);
  }, 30_000);

  it("GET /api/markets includes the created market", async () => {
    const markets = await pollJson<Array<{ address: string; statusLabel: string; creator: string }>>(
      "/api/markets",
      (arr) => arr.some((m) => m.address === market),
    );
    const m = markets.find((x) => x.address === market)!;
    expect(m.statusLabel).toBe("funding");
    expect(m.creator).toBe(wallet.publicKey.toString());
  }, 30_000);

  it("GET /api/markets/:pubkey returns the market + contribution + oracle, reserves null for Funding", async () => {
    type Detail = {
      market: { address: string; status: number; totalContributed: string };
      contributions: Array<{ contributor: string; amount: string }>;
      oracle: { optionsCount: number; phase: number } | null;
      reserves: unknown | null;
    };
    const detail = await pollJson<Detail>(
      `/api/markets/${market}`,
      (d) => d.contributions.length > 0,
    );
    expect(detail.market.address).toBe(market);
    expect(detail.market.status).toBe(MarketStatus.Funding);
    // The creator's seed created a Contribution recording PRESEED.
    const contrib = detail.contributions.find((c) => c.contributor === wallet.publicKey.toString());
    expect(contrib).toBeDefined();
    expect(BigInt(contrib!.amount)).toBe(PRESEED);
    // Oracle enrichment (on-demand RPC read of the linked Kassandra oracle).
    expect(detail.oracle).not.toBeNull();
    expect(detail.oracle!.optionsCount).toBe(2);
    expect(detail.oracle!.phase).toBe(Phase.Proposal);
    // A Funding market has no AMM yet → reserves null.
    expect(detail.reserves).toBeNull();
  }, 30_000);

  it("GET /api/account/:pubkey returns an existing account and 404s a missing one", async () => {
    const present = await getJson<{ owner: string; data: string }>(`/api/account/${kassMint}`);
    expect(present.status).toBe(200);
    expect(present.body.owner).toBeTruthy();
    expect(present.body.data.length).toBeGreaterThan(0);

    const missing = await getJson(`/api/account/${(await Keypair.generate()).publicKey.toString()}`);
    expect(missing.status).toBe(404);
  });

  it("GET /api/blockhash returns a blockhash", async () => {
    const { status, body } = await getJson<{ blockhash: string }>("/api/blockhash");
    expect(status).toBe(200);
    expect(typeof body.blockhash).toBe("string");
    expect(body.blockhash.length).toBeGreaterThan(30);
  });

  it("POST /api/transaction relays a signed contribute → GET /api/transaction/:sig confirms", async () => {
    // Build a contribute ix, stamp an indexer-served blockhash, sign, base64.
    const ix = await contribute({
      contributor: wallet.publicKey,
      market,
      contributorKassAta: walletKassAta,
      amount: RELAY_CONTRIB,
    });
    const { body: bh } = await getJson<{ blockhash: string }>("/api/blockhash");
    const tx = new Transaction();
    tx.feePayer = wallet.publicKey;
    tx.recentBlockhash = bh.blockhash as Transaction["recentBlockhash"];
    tx.add(ix);
    await tx.sign(wallet);
    const txBase64 = Buffer.from(await tx.serialize()).toString("base64");

    const res = await fetch(`${BASE}/api/transaction`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ tx: txBase64 }),
    });
    expect(res.status, `relay body: ${JSON.stringify(await res.clone().json())}`).toBe(200);
    const { signature } = (await res.json()) as { signature: string };
    expect(typeof signature).toBe("string");

    // The relayed tx reaches confirmed/finalized.
    const status = await pollJson<{ status: string; err: string | null }>(
      `/api/transaction/${signature}`,
      (s) => s.status === "confirmed" || s.status === "finalized",
      30_000,
    );
    expect(status.err).toBeNull();

    // …and the reconcile picks up the new total (PRESEED + RELAY_CONTRIB).
    const detail = await pollJson<{ market: { totalContributed: string } }>(
      `/api/markets/${market}`,
      (d) => BigInt(d.market.totalContributed) === PRESEED + RELAY_CONTRIB,
      15_000,
    );
    expect(BigInt(detail.market.totalContributed)).toBe(PRESEED + RELAY_CONTRIB);
  }, 60_000);
});
