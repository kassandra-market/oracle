/**
 * Market READ data layer — pure, side-effect-free functions over the
 * {@link IndexerClient}. The indexer (Phase 1) has ALREADY enumerated + decoded
 * every kassandra-market account, so this layer no longer touches a Solana RPC or
 * decodes bytes: it fetches the indexer's DTOs (`/api/markets`,
 * `/api/markets/{pubkey}`, `/api/config`) and MAPS them into the existing app
 * types (`Market`, `Config`, `Contribution`, `MarketOracle`, `AmmReserves`) the UI
 * + the grouping helpers already consume. NO React, NO hooks — the query hooks
 * wrap these for loading/error/empty states.
 */
import { Address } from "@solana/web3.js";
import {
  AccountType,
  MarketStatus,
  pda,
  type Config,
  type Contribution,
  type Market,
  type MarketOracle,
  type Phase,
  type metadao,
} from "@kassandra-market/sdk";
import type {
  ConfigDto,
  ContributionDto,
  IndexerClient,
  MarketDto,
  OracleDto,
  ReservesDto,
} from "../lib/indexer";

/**
 * The two reserve balances of a market's cYES/cNO pool (raw base units).
 * Re-exported from the SDK ({@link metadao.AmmReserves}) so the app's existing
 * importers keep a single stable name.
 */
export type AmmReserves = metadao.AmmReserves;

/** One enumerated + decoded market. */
export interface MarketSummary {
  /** Base58 market PDA. */
  pubkey: string;
  market: Market;
  /**
   * The market's live cYES/cNO pool reserves — populated (best-effort) for
   * Active markets so the list card can show the live YES probability; `null`
   * for non-Active markets or when the pool read failed.
   */
  reserves: AmmReserves | null;
  /**
   * The linked oracle's `options_count` (best-effort) — how many categorical
   * outcomes the oracle has. Drives the categorical grouping; `null` when the
   * oracle account could not be read.
   */
  oracleOptionsCount: number | null;
}

/**
 * A set of sub-markets that share one oracle — the categorical grouping unit.
 * A binary (2-option) oracle has a single sub-market (`outcome_index = 0`); a
 * categorical oracle has one sub-market per outcome, here sorted by
 * `outcomeIndex` ascending.
 */
export interface OracleGroup {
  /** Base58 oracle pubkey shared by every sub-market in {@link markets}. */
  oracle: string;
  /** The oracle's `options_count`, or `null` when it could not be read. */
  optionsCount: number | null;
  /** The oracle's sub-markets, sorted by `outcomeIndex` ascending. */
  markets: MarketSummary[];
}

/** A market plus its children + linked oracle + live pool reserves — the detail payload. */
export interface MarketDetail {
  pubkey: string;
  market: Market;
  /** Every decoded contribution to this market. */
  contributions: { pubkey: string; contribution: Contribution }[];
  /** The linked Kassandra oracle's read fields, or `null` if unreadable. */
  oracle: MarketOracle | null;
  /** The live cYES/cNO pool reserves (Active markets only), or `null`. */
  reserves: AmmReserves | null;
}

/** Thrown by {@link fetchMarketDetail} when the market account is absent or the wrong type. */
export class MarketNotFoundError extends Error {
  readonly pubkey: string;
  constructor(pubkey: string) {
    super(`Market account ${pubkey} not found (or not a kassandra-market Market).`);
    this.name = "MarketNotFoundError";
    this.pubkey = pubkey;
  }
}

// --- DTO → app-type mapping ---------------------------------------------------
// The indexer emits pubkeys as base58 strings and every `u64` as a string (JS
// precision); we re-hydrate them into the `Address` + `bigint` shapes the UI uses.

/** Map a {@link MarketDto} into the app's decoded {@link Market}. */
export function mapMarketDto(dto: MarketDto): Market {
  return {
    accountType: AccountType.Market,
    oracle: new Address(dto.oracle),
    creator: new Address(dto.creator),
    kassMint: new Address(dto.kassMint),
    escrowVault: new Address(dto.escrowVault),
    minLiquidity: BigInt(dto.minLiquidity),
    totalContributed: BigInt(dto.totalContributed),
    openContributions: dto.openContributions,
    status: dto.status as MarketStatus,
    bump: dto.bump,
    escrowBump: dto.escrowBump,
    question: new Address(dto.question),
    vault: new Address(dto.vault),
    yesMint: new Address(dto.yesMint),
    noMint: new Address(dto.noMint),
    amm: new Address(dto.amm),
    lpMint: new Address(dto.lpMint),
    lpVault: new Address(dto.lpVault),
    lpTotal: BigInt(dto.lpTotal),
    settled: dto.settled !== 0,
    feeBps: dto.feeBps,
    feeCollected: dto.feeCollected !== 0,
    outcomeIndex: dto.outcomeIndex,
  };
}

/** Map a {@link ConfigDto} into the app's decoded {@link Config}. */
export function mapConfigDto(dto: ConfigDto): Config {
  return {
    accountType: AccountType.Config,
    authority: new Address(dto.authority),
    kassMint: new Address(dto.kassMint),
    minLiquidity: BigInt(dto.minLiquidity),
    bump: dto.bump,
    feeBps: dto.feeBps,
    feeDestination: new Address(dto.feeDestination),
  };
}

/** Map a {@link ContributionDto} into the app's decoded {@link Contribution}. */
export function mapContributionDto(dto: ContributionDto): Contribution {
  return {
    accountType: AccountType.Contribution,
    market: new Address(dto.market),
    contributor: new Address(dto.contributor),
    amount: BigInt(dto.amount),
    claimed: dto.claimed,
    bump: dto.bump,
  };
}

/** Map an {@link OracleDto} into the app's {@link MarketOracle}. */
function mapOracleDto(dto: OracleDto): MarketOracle {
  return {
    optionsCount: dto.optionsCount,
    phase: dto.phase as Phase,
    resolvedOption: dto.resolvedOption,
  };
}

/** Map a {@link ReservesDto} into the app's {@link AmmReserves}. */
function mapReservesDto(dto: ReservesDto): AmmReserves {
  return { base: BigInt(dto.base), quote: BigInt(dto.quote) };
}

/**
 * Every market from the indexer, sorted by `totalContributed` descending
 * (most-funded first). Each market is enriched (best-effort, in parallel) with
 * its detail — the linked oracle's `options_count` (categorical grouping) and, for
 * Active markets, the live cYES/cNO pool reserves (the list card's YES
 * probability). A failed detail read degrades that market's enrichment to `null`.
 */
export async function fetchMarkets(indexer: IndexerClient): Promise<MarketSummary[]> {
  const markets = await indexer.getMarkets();
  const summaries = await Promise.all(
    markets.map(async (dto) => {
      const detail = await indexer.getMarket(dto.address).catch(() => null);
      return {
        pubkey: dto.address,
        market: mapMarketDto(dto),
        reserves: detail?.reserves ? mapReservesDto(detail.reserves) : null,
        oracleOptionsCount: detail?.oracle?.optionsCount ?? null,
      };
    }),
  );
  return summaries.sort((a, b) =>
    b.market.totalContributed > a.market.totalContributed
      ? 1
      : b.market.totalContributed < a.market.totalContributed
        ? -1
        : 0,
  );
}

/**
 * Fetch one market plus all of its contributions, its linked Kassandra oracle's
 * read fields, and (when Active) its live pool reserves — all in the single
 * `/api/markets/{pubkey}` detail call the indexer already assembles. Throws
 * {@link MarketNotFoundError} when the market is not indexed (404). A missing
 * oracle yields `oracle: null`; a missing pool yields `reserves: null`.
 */
export async function fetchMarketDetail(
  indexer: IndexerClient,
  marketPubkey: string,
): Promise<MarketDetail> {
  const detail = await indexer.getMarket(marketPubkey);
  if (!detail) throw new MarketNotFoundError(marketPubkey);

  // The Contribution DTO carries no address; re-derive the Contribution PDA
  // (seeds [b"contribution", market, contributor]) for a stable React key.
  const contributions = await Promise.all(
    detail.contributions.map(async (c) => ({
      pubkey: (await pda.contribution(marketPubkey, c.contributor)).address.toString(),
      contribution: mapContributionDto(c),
    })),
  );

  return {
    pubkey: marketPubkey,
    market: mapMarketDto(detail.market),
    contributions,
    oracle: detail.oracle ? mapOracleDto(detail.oracle) : null,
    reserves: detail.reserves ? mapReservesDto(detail.reserves) : null,
  };
}

/**
 * Group markets by their shared oracle → one {@link OracleGroup} per oracle,
 * each with its sub-markets sorted by `outcomeIndex` ascending. Group order
 * follows first appearance in the input (so a pre-sorted list keeps its order).
 * `optionsCount` is taken from the group's summaries (first non-null); a
 * categorical oracle's N sub-markets thus collapse into one group.
 */
export function groupByOracle(summaries: MarketSummary[]): OracleGroup[] {
  const byOracle = new Map<string, MarketSummary[]>();
  for (const summary of summaries) {
    const oracle = summary.market.oracle.toString();
    const list = byOracle.get(oracle);
    if (list) list.push(summary);
    else byOracle.set(oracle, [summary]);
  }
  const groups: OracleGroup[] = [];
  for (const [oracle, markets] of byOracle) {
    markets.sort((a, b) => a.market.outcomeIndex - b.market.outcomeIndex);
    const optionsCount = markets.find((m) => m.oracleOptionsCount !== null)?.oracleOptionsCount ?? null;
    groups.push({ oracle, optionsCount, markets });
  }
  return groups;
}

/**
 * Whether a group is a categorical (N>2) question rather than a plain binary
 * market — true when the oracle has more than two options, or (defensively,
 * when the oracle read failed) more than one sub-market has been created.
 */
export function isCategorical(group: OracleGroup): boolean {
  if (group.optionsCount !== null) return group.optionsCount > 2;
  return group.markets.length > 1;
}

/**
 * Read the `Config` singleton (`/api/config`) for the KASS mint + funding-floor.
 * Returns `null` when the program is not yet initialised (no Config account, the
 * indexer 404s).
 */
export async function fetchConfig(indexer: IndexerClient): Promise<Config | null> {
  const dto = await indexer.getConfig();
  return dto ? mapConfigDto(dto) : null;
}
