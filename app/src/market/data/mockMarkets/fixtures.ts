/**
 * Hand-authored fixture DTOs for the Markets flow's offline preview — the market
 * analogue of `src/data/mockOracles/fixtures.ts`. Every DTO here is consumed by
 * the REAL mapper functions in `../markets.ts` (`mapMarketDto`, `mapConfigDto`,
 * `mapContributionDto`), so every pubkey must be a genuinely valid base58-encoded
 * 32-byte address (round-trips through `new Address(...)`, exactly like a live
 * indexer response) and every u64 field a base-10 string.
 *
 * Unlike the oracle fixtures (whose pubkey-shaped strings are only ever
 * stringified, never fed to `new Address(...)`), these pubkeys ARE constructed
 * into real `Address`es by the market data layer — so `fixturePubkey` below
 * builds them from real bytes through the real codec rather than hand-typing a
 * base58-look-alike string (most hand-typed look-alikes are the wrong decoded
 * byte length and throw).
 *
 * Covers, across `MOCK_MARKET_PUBKEYS`:
 *   - a pre-activation `Funding` market (partially funded, own binary oracle)
 *   - an `Active` market (own binary oracle, live) with populated cYES/cNO
 *     reserves so the trade UI + price chart have live-looking data
 *   - a `Resolved` market (own binary oracle, terminal, YES won)
 *   - a `Void` market (own binary oracle, `InvalidDeadend` — both legs paid)
 *   - a `Cancelled` market (never activated, own terminal `InvalidDeadend` oracle)
 *   - a 3-outcome CATEGORICAL group — one oracle (`optionsCount = 3`), three
 *     sub-markets at `outcomeIndex` 0/1/2 (`groupByOracle` collapses these into
 *     one `OracleGroup`; `isCategorical` is true since `optionsCount > 2`)
 *   - a second 3-outcome CATEGORICAL group, every outcome still `Funding` (none
 *     activated yet) — exercises the single cumulative funding bar + the
 *     group-only deposit action on a group that hasn't started resolving
 */
import { Address } from "@solana/web3.js";
import type { CandleDto, ConfigDto, ContributionDto, MarketDetailDto, MarketDto, OracleDto, ReservesDto } from "../../lib/indexer";

// --- deterministic fixture addresses ------------------------------------------

/** FNV-1a (32-bit) — a small, deterministic, dependency-free string hash. */
function fnv1a(str: string): number {
  let h = 0x811c9dc5;
  for (let i = 0; i < str.length; i++) {
    h ^= str.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  return h >>> 0;
}

/**
 * A deterministic, genuinely-valid 32-byte pubkey derived from a short label.
 * Every byte comes from re-hashing `label:index:prevHash`, then the 32 raw bytes
 * are encoded through the real `Address` codec — so the result is a real base58
 * pubkey (unlike a hand-typed look-alike, which is very likely the wrong decoded
 * byte length or contains an invalid base58 character).
 */
function fixturePubkey(label: string): string {
  const bytes = new Uint8Array(32);
  let h = fnv1a(label);
  for (let i = 0; i < 32; i++) {
    h = fnv1a(`${label}:${i}:${h}`);
    bytes[i] = h & 0xff;
  }
  return new Address(bytes).toBase58();
}

/** The all-zero pubkey (`system program`) — the on-chain sentinel for the
 * MetaDAO composition fields (`question`/`vault`/`yesMint`/…) before `activate`. */
const ZERO = "11111111111111111111111111111111";

const KASS_MINT = fixturePubkey("kass-mint");

/** KASS has 9 decimals; base units from a whole-KASS count. */
const SCALE = 10n ** 9n;
const kass = (whole: number): string => (BigInt(whole) * SCALE).toString();

// --- oracles (one per standalone market, one shared by the categorical group) -

const O_FUNDING = fixturePubkey("oracle-funding");
const O_ACTIVE = fixturePubkey("oracle-active");
const O_RESOLVED = fixturePubkey("oracle-resolved");
const O_VOID = fixturePubkey("oracle-void");
const O_CANCELLED = fixturePubkey("oracle-cancelled");
const O_CATEGORICAL = fixturePubkey("oracle-categorical");
const O_CATEGORICAL_FUNDING = fixturePubkey("oracle-categorical-funding");

const ORACLES: Record<string, OracleDto> = {
  [O_FUNDING]: { optionsCount: 2, phase: 1 /* Proposal */, resolvedOption: 0 },
  [O_ACTIVE]: { optionsCount: 2, phase: 3 /* FactVoting */, resolvedOption: 0 },
  [O_RESOLVED]: { optionsCount: 2, phase: 7 /* Resolved */, resolvedOption: 0 /* YES (outcomeIndex 0) won */ },
  [O_VOID]: { optionsCount: 2, phase: 8 /* InvalidDeadend */, resolvedOption: 0xff },
  [O_CANCELLED]: { optionsCount: 2, phase: 8 /* InvalidDeadend */, resolvedOption: 0xff },
  // Resolved with option 2 winning — CAT_2 pays YES, CAT_0/CAT_1 pay NO. CAT_1's
  // own `resolve_market` crank hasn't run yet (its `status` is still `Active`),
  // a realistic "resolution pending" categorical state.
  [O_CATEGORICAL]: { optionsCount: 3, phase: 7 /* Resolved */, resolvedOption: 2 },
  // A categorical group still pre-activation — every outcome Funding, none has
  // met its own floor yet. Exercises the single cumulative funding bar + the
  // group-only deposit action (no per-outcome contribute form) on a group that
  // hasn't started resolving at all yet.
  [O_CATEGORICAL_FUNDING]: { optionsCount: 3, phase: 1 /* Proposal */, resolvedOption: 0 },
};

// --- markets -------------------------------------------------------------------

interface MarketFixture {
  dto: MarketDto;
  oracle: OracleDto;
  reserves: ReservesDto | null;
  contributions: ContributionDto[];
}

/** Full-shape `MarketDto` with sensible pre-activation defaults; callers override. */
function makeMarket(label: string, over: Partial<MarketDto> & Pick<MarketDto, "address" | "oracle" | "status" | "statusLabel">): MarketDto {
  const base: MarketDto = {
    address: over.address,
    status: over.status,
    statusLabel: over.statusLabel,
    oracle: over.oracle,
    creator: fixturePubkey(`${label}-creator`),
    kassMint: KASS_MINT,
    escrowVault: fixturePubkey(`${label}-escrow`),
    minLiquidity: kass(500_000),
    totalContributed: kass(0),
    openContributions: 1,
    bump: 253,
    escrowBump: 252,
    outcomeIndex: 0,
    feeBps: 250,
    feeCollected: 0,
    settled: 0,
    question: ZERO,
    vault: ZERO,
    yesMint: ZERO,
    noMint: ZERO,
    amm: ZERO,
    lpMint: ZERO,
    lpVault: ZERO,
    lpTotal: kass(0),
    activationLp: kass(0),
    activationContributed: kass(0),
    grossLpTotal: kass(0),
    slot: "1000",
  };
  return { ...base, ...over };
}

/** A single deterministic `ContributionDto` for `market`, from `contributor`. */
function makeContribution(
  market: string,
  label: string,
  amountWhole: number,
  opts: { claimed?: boolean; lateLpWhole?: number; slot?: string } = {},
): ContributionDto {
  return {
    market,
    contributor: fixturePubkey(`${label}-contributor`),
    amount: kass(amountWhole),
    claimed: opts.claimed ?? false,
    bump: 254,
    lateLp: kass(opts.lateLpWhole ?? 0),
    slot: opts.slot ?? "1000",
  };
}

const MKT_FUNDING = fixturePubkey("market-funding-binary");
const MKT_ACTIVE = fixturePubkey("market-active-binary");
const MKT_RESOLVED = fixturePubkey("market-resolved-binary");
const MKT_VOID = fixturePubkey("market-void-binary");
const MKT_CANCELLED = fixturePubkey("market-cancelled-binary");
const MKT_CAT_0 = fixturePubkey("market-categorical-outcome-0");
const MKT_CAT_1 = fixturePubkey("market-categorical-outcome-1");
const MKT_CAT_2 = fixturePubkey("market-categorical-outcome-2");
const MKT_CAT_FUNDING_0 = fixturePubkey("market-categorical-funding-outcome-0");
const MKT_CAT_FUNDING_1 = fixturePubkey("market-categorical-funding-outcome-1");
const MKT_CAT_FUNDING_2 = fixturePubkey("market-categorical-funding-outcome-2");

const FIXTURES: MarketFixture[] = [
  {
    // Pre-activation, partially funded — below its 500,000 KASS floor.
    dto: makeMarket("funding", {
      address: MKT_FUNDING,
      oracle: O_FUNDING,
      status: 0 /* Funding */,
      statusLabel: "Funding",
      totalContributed: kass(235_000),
      openContributions: 2,
      slot: "1001",
    }),
    oracle: ORACLES[O_FUNDING],
    reserves: null,
    contributions: [
      makeContribution(MKT_FUNDING, "funding-a", 150_000, { slot: "1001" }),
      makeContribution(MKT_FUNDING, "funding-b", 85_000, { slot: "1000" }),
    ],
  },
  {
    // Fully activated + trading live — populated cYES/cNO reserves for the
    // trade UI + price chart (implied YES ≈ 40%).
    dto: makeMarket("active", {
      address: MKT_ACTIVE,
      oracle: O_ACTIVE,
      status: 1 /* Active */,
      statusLabel: "Active",
      totalContributed: kass(620_000),
      openContributions: 2,
      question: fixturePubkey("active-question"),
      vault: fixturePubkey("active-vault"),
      yesMint: fixturePubkey("active-yes-mint"),
      noMint: fixturePubkey("active-no-mint"),
      amm: fixturePubkey("active-amm"),
      lpMint: fixturePubkey("active-lp-mint"),
      lpVault: fixturePubkey("active-lp-vault"),
      lpTotal: kass(640_000),
      activationLp: kass(620_000),
      activationContributed: kass(620_000),
      grossLpTotal: kass(640_000),
      slot: "1010",
    }),
    oracle: ORACLES[O_ACTIVE],
    reserves: { base: kass(600_000), quote: kass(400_000) },
    contributions: [
      makeContribution(MKT_ACTIVE, "active-a", 420_000, { slot: "1005" }),
      makeContribution(MKT_ACTIVE, "active-b", 200_000, { lateLpWhole: 20_000, slot: "1010" }),
    ],
  },
  {
    // Terminal + resolved — YES won (resolvedOption 0 == outcomeIndex 0).
    dto: makeMarket("resolved", {
      address: MKT_RESOLVED,
      oracle: O_RESOLVED,
      status: 2 /* Resolved */,
      statusLabel: "Resolved",
      totalContributed: kass(500_000),
      openContributions: 1,
      feeCollected: 1,
      settled: 1,
      question: fixturePubkey("resolved-question"),
      vault: fixturePubkey("resolved-vault"),
      yesMint: fixturePubkey("resolved-yes-mint"),
      noMint: fixturePubkey("resolved-no-mint"),
      amm: fixturePubkey("resolved-amm"),
      lpMint: fixturePubkey("resolved-lp-mint"),
      lpVault: fixturePubkey("resolved-lp-vault"),
      lpTotal: kass(500_000),
      activationLp: kass(500_000),
      activationContributed: kass(500_000),
      grossLpTotal: kass(500_000),
      slot: "1020",
    }),
    oracle: ORACLES[O_RESOLVED],
    reserves: null,
    contributions: [
      makeContribution(MKT_RESOLVED, "resolved-a", 300_000, { claimed: true, slot: "1015" }),
      makeContribution(MKT_RESOLVED, "resolved-b", 200_000, { claimed: false, slot: "1020" }),
    ],
  },
  {
    // Terminal via a dead-end oracle — every leg pays (both cYES and cNO redeem).
    dto: makeMarket("void", {
      address: MKT_VOID,
      oracle: O_VOID,
      status: 3 /* Void */,
      statusLabel: "Void",
      totalContributed: kass(480_000),
      openContributions: 1,
      feeBps: 0,
      feeCollected: 1, // `feeBps == 0` ⇒ stamped directly by `resolve_market`.
      settled: 1,
      question: fixturePubkey("void-question"),
      vault: fixturePubkey("void-vault"),
      yesMint: fixturePubkey("void-yes-mint"),
      noMint: fixturePubkey("void-no-mint"),
      amm: fixturePubkey("void-amm"),
      lpMint: fixturePubkey("void-lp-mint"),
      lpVault: fixturePubkey("void-lp-vault"),
      lpTotal: kass(480_000),
      activationLp: kass(480_000),
      activationContributed: kass(480_000),
      grossLpTotal: kass(480_000),
      slot: "1030",
    }),
    oracle: ORACLES[O_VOID],
    reserves: null,
    contributions: [makeContribution(MKT_VOID, "void-a", 480_000, { claimed: false, slot: "1025" })],
  },
  {
    // Never activated — cancelled once its oracle dead-ended; refunds pending.
    dto: makeMarket("cancelled", {
      address: MKT_CANCELLED,
      oracle: O_CANCELLED,
      status: 4 /* Cancelled */,
      statusLabel: "Cancelled",
      totalContributed: kass(150_000),
      openContributions: 2,
      slot: "1004",
    }),
    oracle: ORACLES[O_CANCELLED],
    reserves: null,
    contributions: [
      makeContribution(MKT_CANCELLED, "cancelled-a", 100_000, { claimed: false, slot: "1002" }),
      makeContribution(MKT_CANCELLED, "cancelled-b", 50_000, { claimed: true, slot: "1004" }),
    ],
  },
  {
    // Categorical outcome 0/3 — already resolved (loses to option 2).
    dto: makeMarket("cat0", {
      address: MKT_CAT_0,
      oracle: O_CATEGORICAL,
      status: 2 /* Resolved */,
      statusLabel: "Resolved",
      outcomeIndex: 0,
      totalContributed: kass(300_000),
      openContributions: 1,
      feeCollected: 1,
      settled: 1,
      question: fixturePubkey("cat0-question"),
      vault: fixturePubkey("cat0-vault"),
      yesMint: fixturePubkey("cat0-yes-mint"),
      noMint: fixturePubkey("cat0-no-mint"),
      amm: fixturePubkey("cat0-amm"),
      lpMint: fixturePubkey("cat0-lp-mint"),
      lpVault: fixturePubkey("cat0-lp-vault"),
      lpTotal: kass(300_000),
      activationLp: kass(300_000),
      activationContributed: kass(300_000),
      grossLpTotal: kass(300_000),
      slot: "1040",
    }),
    oracle: ORACLES[O_CATEGORICAL],
    reserves: null,
    contributions: [makeContribution(MKT_CAT_0, "cat0-a", 300_000, { claimed: true, slot: "1035" })],
  },
  {
    // Categorical outcome 1/3 — oracle terminal, but this sub-market's own
    // `resolve_market` crank hasn't run yet: still `Active`, still trading.
    dto: makeMarket("cat1", {
      address: MKT_CAT_1,
      oracle: O_CATEGORICAL,
      status: 1 /* Active */,
      statusLabel: "Active",
      outcomeIndex: 1,
      totalContributed: kass(250_000),
      openContributions: 1,
      question: fixturePubkey("cat1-question"),
      vault: fixturePubkey("cat1-vault"),
      yesMint: fixturePubkey("cat1-yes-mint"),
      noMint: fixturePubkey("cat1-no-mint"),
      amm: fixturePubkey("cat1-amm"),
      lpMint: fixturePubkey("cat1-lp-mint"),
      lpVault: fixturePubkey("cat1-lp-vault"),
      lpTotal: kass(250_000),
      activationLp: kass(250_000),
      activationContributed: kass(250_000),
      grossLpTotal: kass(250_000),
      slot: "1036",
    }),
    oracle: ORACLES[O_CATEGORICAL],
    reserves: { base: kass(350_000), quote: kass(150_000) },
    contributions: [makeContribution(MKT_CAT_1, "cat1-a", 250_000, { slot: "1033" })],
  },
  {
    // Categorical outcome 2/3 — the winner (resolvedOption == 2 == outcomeIndex).
    dto: makeMarket("cat2", {
      address: MKT_CAT_2,
      oracle: O_CATEGORICAL,
      status: 2 /* Resolved */,
      statusLabel: "Resolved",
      outcomeIndex: 2,
      totalContributed: kass(200_000),
      openContributions: 1,
      feeCollected: 1,
      settled: 1,
      question: fixturePubkey("cat2-question"),
      vault: fixturePubkey("cat2-vault"),
      yesMint: fixturePubkey("cat2-yes-mint"),
      noMint: fixturePubkey("cat2-no-mint"),
      amm: fixturePubkey("cat2-amm"),
      lpMint: fixturePubkey("cat2-lp-mint"),
      lpVault: fixturePubkey("cat2-lp-vault"),
      lpTotal: kass(200_000),
      activationLp: kass(200_000),
      activationContributed: kass(200_000),
      grossLpTotal: kass(200_000),
      slot: "1041",
    }),
    oracle: ORACLES[O_CATEGORICAL],
    reserves: null,
    contributions: [makeContribution(MKT_CAT_2, "cat2-a", 200_000, { claimed: false, slot: "1038" })],
  },
  {
    // Funding categorical outcome 0/3 — barely started.
    dto: makeMarket("cat-funding-0", {
      address: MKT_CAT_FUNDING_0,
      oracle: O_CATEGORICAL_FUNDING,
      status: 0 /* Funding */,
      statusLabel: "Funding",
      outcomeIndex: 0,
      totalContributed: kass(5_000),
      openContributions: 1,
      slot: "1050",
    }),
    oracle: ORACLES[O_CATEGORICAL_FUNDING],
    reserves: null,
    contributions: [makeContribution(MKT_CAT_FUNDING_0, "cat-funding-0-a", 5_000, { slot: "1050" })],
  },
  {
    // Funding categorical outcome 1/3 — partially funded, still under floor.
    dto: makeMarket("cat-funding-1", {
      address: MKT_CAT_FUNDING_1,
      oracle: O_CATEGORICAL_FUNDING,
      status: 0 /* Funding */,
      statusLabel: "Funding",
      outcomeIndex: 1,
      totalContributed: kass(80_000),
      openContributions: 1,
      slot: "1051",
    }),
    oracle: ORACLES[O_CATEGORICAL_FUNDING],
    reserves: null,
    contributions: [makeContribution(MKT_CAT_FUNDING_1, "cat-funding-1-a", 80_000, { slot: "1051" })],
  },
  {
    // Funding categorical outcome 2/3 — partially funded, still under floor.
    dto: makeMarket("cat-funding-2", {
      address: MKT_CAT_FUNDING_2,
      oracle: O_CATEGORICAL_FUNDING,
      status: 0 /* Funding */,
      statusLabel: "Funding",
      outcomeIndex: 2,
      totalContributed: kass(120_000),
      openContributions: 1,
      slot: "1052",
    }),
    oracle: ORACLES[O_CATEGORICAL_FUNDING],
    reserves: null,
    contributions: [makeContribution(MKT_CAT_FUNDING_2, "cat-funding-2-a", 120_000, { slot: "1052" })],
  },
];

const BY_PUBKEY = new Map<string, MarketFixture>(FIXTURES.map((f) => [f.dto.address, f]));

/** Every fixture market's pubkey, in fixture-declaration order. */
export const MOCK_MARKET_PUBKEYS: string[] = FIXTURES.map((f) => f.dto.address);

/** Every fixture `MarketDto` (the `GET /api/markets` mock). */
export function mockMarketDtos(): MarketDto[] {
  return FIXTURES.map((f) => f.dto);
}

/** The `GET /api/markets/{pubkey}` mock — `null` for an unknown pubkey (mirrors a 404). */
export function mockMarketDetailFor(pubkey: string): MarketDetailDto | null {
  const fixture = BY_PUBKEY.get(pubkey);
  if (!fixture) return null;
  return {
    market: fixture.dto,
    contributions: fixture.contributions,
    oracle: fixture.oracle,
    reserves: fixture.reserves,
  };
}

// --- candles ---------------------------------------------------------------

/**
 * `mulberry32` — a tiny, dependency-free, deterministic PRNG (given the same
 * 32-bit seed it always produces the same sequence). No `Math.random`.
 */
function mulberry32(seed: number): () => number {
  let state = seed >>> 0;
  return () => {
    state = (state + 0x6d2b79f5) | 0;
    let t = Math.imul(state ^ (state >>> 15), 1 | state);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/** Fixed anchor for candle timestamps — NOT wall-clock time, so output is reproducible. */
const CANDLE_EPOCH = 1_700_000_000;

/**
 * A deterministic synthetic OHLC series of implied YES probability, mean-
 * reverting around 0.5. Same `(pubkey, intervalSecs, limit)` always produces the
 * same candles' VALUES — seeded by a hash of the inputs, not the wall clock or
 * `Math.random`. `limit` candles spaced `intervalSecs` apart, oldest first.
 *
 * Timestamps are anchored to `nowSecs` when given: the last candle lands on the
 * `intervalSecs`-aligned bucket at/before `nowSecs`, walking backwards `limit`
 * candles from there. When `nowSecs` is omitted, timestamps fall back to the
 * fixed, non-wall-clock `CANDLE_EPOCH` anchor so callers that need fully
 * reproducible output (e.g. unit tests) keep getting it. Either way the OHLC
 * values themselves are the same deterministic random walk — only the time
 * axis moves.
 */
export function mockCandlesFor(pubkey: string, intervalSecs: number, limit: number, nowSecs?: number): CandleDto[] {
  const rand = mulberry32(fnv1a(`${pubkey}:${intervalSecs}`));
  const lastBucket = nowSecs === undefined ? CANDLE_EPOCH + (limit - 1) * intervalSecs : Math.floor(nowSecs / intervalSecs) * intervalSecs;
  const startBucket = lastBucket - (limit - 1) * intervalSecs;
  const candles: CandleDto[] = [];
  let price = 0.5;
  for (let i = 0; i < limit; i++) {
    const open = price;
    const drift = (rand() - 0.5) * 0.04 + (0.5 - open) * 0.02; // gentle mean reversion
    const close = Math.min(0.98, Math.max(0.02, open + drift));
    const wickUp = rand() * 0.01;
    const wickDown = rand() * 0.01;
    const high = Math.min(0.99, Math.max(open, close) + wickUp);
    const low = Math.max(0.01, Math.min(open, close) - wickDown);
    candles.push({ time: startBucket + i * intervalSecs, open, high, low, close });
    price = close;
  }
  return candles;
}

// --- config ------------------------------------------------------------------

/** The `GET /api/config` mock — the program `Config` singleton. */
export function mockConfigDto(): ConfigDto {
  return {
    address: fixturePubkey("config"),
    authority: fixturePubkey("futarchy-authority"),
    kassMint: KASS_MINT,
    minLiquidity: kass(500_000),
    bump: 254,
    feeBps: 250,
    feeDestination: fixturePubkey("fee-destination"),
    slot: "999",
  };
}
