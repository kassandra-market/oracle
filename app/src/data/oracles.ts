/**
 * Oracle READ data layer — pure, side-effect-free functions over a web3.js
 * {@link Connection} that enumerate + decode Kassandra on-chain accounts via
 * `@kassandra/sdk`. NO React, NO hooks — a UI layer (FA3) wraps these in query
 * hooks for loading/error/empty states.
 *
 * Enumeration strategy (there is no `getProgramAccounts` helper in the SDK — the
 * runner had one in Rust; the app implements it here):
 *
 *   - every Pod account starts with `account_type: u8` at {@link ACCOUNT_TYPE_OFFSET}
 *     (offset 0), so we filter by a `memcmp` on that single tag byte (the PRIMARY
 *     type filter) plus a `{ dataSize }` guard (each account type has a distinct
 *     pinned ABI size — see {@link ACCOUNT_SIZES});
 *   - the CHILD accounts (`Fact`/`Proposer`/`AiClaim`/`Market`) all carry their
 *     parent `oracle` pubkey at byte offset {@link CHILD_ORACLE_OFFSET} (8) — the
 *     first real field after the 8-byte header — so a second `memcmp` on that
 *     offset scopes an enumeration to one oracle (mirrors the runner's
 *     `offset_of!(Fact, oracle) == 8`).
 *
 * Robustness: a single malformed/type-confused account (wrong tag or size) is
 * SKIPPED (the SDK decoder throws, we catch + drop it), never crashing the whole
 * enumeration. A missing oracle throws a typed {@link OracleNotFoundError} the
 * caller can render as a not-found state; a missing market is simply `undefined`.
 */
import type { Connection, GetProgramAccountsFilter } from "@solana/web3.js";
import { Address } from "@solana/web3.js";
import {
  ACCOUNT_SIZES,
  ACCOUNT_TYPE_OFFSET,
  AccountType,
  KASSANDRA_PROGRAM_ID,
  decodeAiClaim,
  decodeFact,
  decodeMarket,
  decodeOracle,
  decodeProposer,
  type AiClaim,
  type Fact,
  type Market,
  type Oracle,
  type Proposer,
} from "@kassandra/sdk";
import { fetchOracleAccounts, fetchOracleDetailAccounts } from "./indexer";
import type { IndexedChildAccount } from "./indexer";

/** Byte offset of the parent `oracle` pubkey in every child account (right after the 8-byte header). */
export const CHILD_ORACLE_OFFSET = 8;

/** One enumerated + decoded oracle. */
export interface OracleSummary {
  /** Base58 oracle PDA. */
  pubkey: string;
  oracle: Oracle;
}

/** An oracle plus all of its decoded children — the detail-view payload. */
export interface OracleDetail {
  pubkey: string;
  oracle: Oracle;
  facts: { pubkey: string; fact: Fact }[];
  proposers: { pubkey: string; proposer: Proposer }[];
  aiClaims: { pubkey: string; aiClaim: AiClaim }[];
  /** The first challenge market for this oracle, if any exists (else `undefined`). */
  market?: { pubkey: string; market: Market };
}

/** Thrown by {@link fetchOracleDetail} when the oracle account is absent or the wrong type. */
export class OracleNotFoundError extends Error {
  readonly pubkey: string;
  constructor(pubkey: string) {
    super(`Oracle account ${pubkey} not found (or not a Kassandra Oracle).`);
    this.name = "OracleNotFoundError";
    this.pubkey = pubkey;
  }
}

// --- base58 (single-byte tag encoding for the memcmp filter) ----------------
// web3.js does not export a byte-array base58 encoder; the oracle-field memcmp
// reuses an address's own base58 string, but the 1-byte account_type tag needs
// its own encode. This is the standard Bitcoin base58 alphabet.
const B58_ALPHABET = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

/** Base58-encode raw bytes (used for the single account_type tag byte). */
export function base58Encode(bytes: Uint8Array): string {
  let zeros = 0;
  while (zeros < bytes.length && bytes[zeros] === 0) zeros++;
  const digits: number[] = [];
  for (let i = zeros; i < bytes.length; i++) {
    let carry = bytes[i];
    for (let j = 0; j < digits.length; j++) {
      carry += digits[j] << 8;
      digits[j] = carry % 58;
      carry = (carry / 58) | 0;
    }
    while (carry > 0) {
      digits.push(carry % 58);
      carry = (carry / 58) | 0;
    }
  }
  let out = "1".repeat(zeros);
  for (let i = digits.length - 1; i >= 0; i--) out += B58_ALPHABET[digits[i]];
  return out.length > 0 ? out : "1";
}

/** The base58-encoded single-byte account_type tag for a given {@link AccountType}. */
function tagBytes(type: AccountType): string {
  return base58Encode(Uint8Array.of(type));
}

/** Filter selecting exactly one account type: its tag byte (primary) + pinned ABI size. */
function typeFilters(type: AccountType, size: number): GetProgramAccountsFilter[] {
  return [
    { memcmp: { offset: ACCOUNT_TYPE_OFFSET, bytes: tagBytes(type) } },
    { dataSize: size },
  ];
}

/**
 * Enumerate every account of one type via `getProgramAccounts` (type filters +
 * any `extraFilters`, e.g. a child's parent-oracle memcmp), decoding each with
 * `decode`. Accounts that fail the decoder's tag/size guard are SKIPPED (not
 * thrown) so one bad blob never sinks the whole list. RPC errors propagate to
 * the caller (a query hook renders the error state).
 */
async function enumerate<T>(
  connection: Connection,
  type: AccountType,
  size: number,
  decode: (data: Uint8Array) => T,
  extraFilters: GetProgramAccountsFilter[] = [],
): Promise<{ pubkey: string; value: T }[]> {
  const accounts = await connection.getProgramAccounts(KASSANDRA_PROGRAM_ID, {
    filters: [...typeFilters(type, size), ...extraFilters],
  });
  const out: { pubkey: string; value: T }[] = [];
  for (const { pubkey, account } of accounts) {
    try {
      out.push({ pubkey: pubkey.toString(), value: decode(account.data) });
    } catch {
      // Malformed / type-confused account — skip it, keep the rest.
    }
  }
  return out;
}

/**
 * Enumerate + decode every {@link Oracle} the program owns, sorted by `deadline`
 * descending (soonest-expiring / most-recent first). Undecodable accounts are
 * skipped. Never throws on a bad account; RPC failures reject (caller handles).
 */
export async function fetchOracles(connection: Connection): Promise<OracleSummary[]> {
  // Prefer the indexer's account mirror (an indexed Postgres read, kept fresh by a
  // gpa snapshot + programSubscribe) over a slow client-side getProgramAccounts.
  // Fall back to the direct scan when the indexer is absent or has nothing yet.
  const indexed = await fetchOracleAccounts();
  const summaries: OracleSummary[] =
    indexed && indexed.length > 0
      ? indexed.flatMap((a) => {
          try {
            return [{ pubkey: a.pubkey, oracle: decodeOracle(a.data) }];
          } catch {
            return []; // malformed/foreign row — skip, keep the rest
          }
        })
      : (await enumerate(connection, AccountType.Oracle, ACCOUNT_SIZES.Oracle, decodeOracle)).map(
          ({ pubkey, value }) => ({ pubkey, oracle: value }),
        );
  return summaries.sort((a, b) =>
    b.oracle.deadline > a.oracle.deadline ? 1 : b.oracle.deadline < a.oracle.deadline ? -1 : 0,
  );
}

/**
 * Fetch one oracle plus all of its decoded children (facts, proposers, AI
 * claims, and the first challenge market if any). Each child set is enumerated
 * by `account_type` + a `memcmp` on the child's parent-`oracle` pubkey at
 * {@link CHILD_ORACLE_OFFSET}. Throws {@link OracleNotFoundError} if the oracle
 * account is absent or the wrong type; a missing market yields `market: undefined`.
 */
/**
 * Assemble an {@link OracleDetail} from the indexer's account mirror (the oracle +
 * its children, tagged by `accountType`). Returns `null` when the oracle account
 * isn't in the set (indexer lagging / oracle absent) so the caller falls back to
 * direct reads. Each decoder re-checks its own tag+size; a bad row is skipped.
 */
function assembleDetailFromIndexed(
  oraclePubkey: string,
  accounts: IndexedChildAccount[],
): OracleDetail | null {
  const oracleAcct = accounts.find(
    (a) => a.accountType === AccountType.Oracle && a.pubkey === oraclePubkey,
  );
  if (!oracleAcct) return null;
  let oracle: Oracle;
  try {
    oracle = decodeOracle(oracleAcct.data);
  } catch {
    return null;
  }
  const facts: OracleDetail["facts"] = [];
  const proposers: OracleDetail["proposers"] = [];
  const aiClaims: OracleDetail["aiClaims"] = [];
  let market: OracleDetail["market"];
  for (const a of accounts) {
    try {
      switch (a.accountType) {
        case AccountType.Fact:
          facts.push({ pubkey: a.pubkey, fact: decodeFact(a.data) });
          break;
        case AccountType.Proposer:
          proposers.push({ pubkey: a.pubkey, proposer: decodeProposer(a.data) });
          break;
        case AccountType.AiClaim:
          aiClaims.push({ pubkey: a.pubkey, aiClaim: decodeAiClaim(a.data) });
          break;
        case AccountType.Market:
          if (!market) market = { pubkey: a.pubkey, market: decodeMarket(a.data) };
          break;
      }
    } catch {
      // malformed/foreign child — skip
    }
  }
  return { pubkey: oraclePubkey, oracle, facts, proposers, aiClaims, market };
}

export async function fetchOracleDetail(
  connection: Connection,
  oraclePubkey: string,
): Promise<OracleDetail> {
  // Prefer the indexer's account mirror; fall back to direct reads when it's
  // absent, lagging, or doesn't yet have this oracle.
  const indexedChildren = await fetchOracleDetailAccounts(oraclePubkey);
  if (indexedChildren && indexedChildren.length > 0) {
    const detail = assembleDetailFromIndexed(oraclePubkey, indexedChildren);
    if (detail) return detail;
  }

  const info = await connection.getAccountInfo(new Address(oraclePubkey));
  if (!info || info.data.length === 0) throw new OracleNotFoundError(oraclePubkey);
  let oracle: Oracle;
  try {
    oracle = decodeOracle(info.data);
  } catch {
    throw new OracleNotFoundError(oraclePubkey);
  }

  // Every child (Fact/Proposer/AiClaim/Market) stores its parent oracle at the SAME
  // offset 8, so ONE getProgramAccounts scoped by that memcmp returns them all — then
  // partition by the account_type tag byte. This collapses what used to be FOUR
  // full-program `getProgramAccounts` scans (the expensive op) into a single scan.
  const children = await connection.getProgramAccounts(KASSANDRA_PROGRAM_ID, {
    filters: [{ memcmp: { offset: CHILD_ORACLE_OFFSET, bytes: oraclePubkey } }],
  });

  const facts: OracleDetail["facts"] = [];
  const proposers: OracleDetail["proposers"] = [];
  const aiClaims: OracleDetail["aiClaims"] = [];
  let market: OracleDetail["market"];
  for (const { pubkey, account } of children) {
    const data = account.data;
    const key = pubkey.toString();
    try {
      // Each decoder re-checks its own tag + size, so a partition slip can't
      // mis-decode; a malformed/foreign child is skipped, keeping the rest.
      switch (data[ACCOUNT_TYPE_OFFSET]) {
        case AccountType.Fact:
          facts.push({ pubkey: key, fact: decodeFact(data) });
          break;
        case AccountType.Proposer:
          proposers.push({ pubkey: key, proposer: decodeProposer(data) });
          break;
        case AccountType.AiClaim:
          aiClaims.push({ pubkey: key, aiClaim: decodeAiClaim(data) });
          break;
        case AccountType.Market:
          if (!market) market = { pubkey: key, market: decodeMarket(data) };
          break;
      }
    } catch {
      // Malformed / type-confused child — skip it, keep the rest.
    }
  }

  return { pubkey: oraclePubkey, oracle, facts, proposers, aiClaims, market };
}
