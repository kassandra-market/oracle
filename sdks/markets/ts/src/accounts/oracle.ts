/**
 * Minimal reader for the external Kassandra `Oracle` account.
 *
 * kassandra-market only needs three fields off the oracle to drive its lifecycle
 * (is it binary? has it resolved? which option won?), so rather than depend on
 * the full `@kassandra-market/oracles` this reads those bytes directly at the offsets pinned
 * in `../kassandra/programs/kassandra/src/state.rs` +
 * `../kassandra/programs/kassandra/tests/state_layout.rs`:
 *
 *   - `options_count: u8` @160  (after the 8-byte header + 4 pubkeys + 3 i64s)
 *   - `phase: u8`         @161
 *   - `resolved_option: u8` @197  (pinned; valid ONLY when `phase == Resolved`)
 *
 * The oracle account is `Oracle::LEN == 360` bytes; we require at least enough
 * bytes to cover `resolved_option` but do NOT pin the exact size (the oracle
 * struct grows independently of this market program).
 */
import { readU8, view } from "./common.js";

/** Byte offset of `Oracle.options_count`. */
export const ORACLE_OPTIONS_COUNT_OFFSET = 160;
/** Byte offset of `Oracle.phase`. */
export const ORACLE_PHASE_OFFSET = 161;
/** Byte offset of `Oracle.resolved_option`. */
export const ORACLE_RESOLVED_OPTION_OFFSET = 197;

/**
 * Kassandra oracle dispute phase (`state.rs::Phase`), stored on-chain as a `u8`.
 * `Created` (0) is reserved/unused — live oracles start at `Proposal`.
 */
export enum Phase {
  Created = 0,
  Proposal = 1,
  FactProposal = 2,
  FactVoting = 3,
  AiClaim = 4,
  Challenge = 5,
  FinalRecompute = 6,
  Resolved = 7,
  InvalidDeadend = 8,
}

/** The three oracle fields kassandra-market reads. */
export interface MarketOracle {
  /** Number of categorical options (binary markets require 2). */
  optionsCount: number;
  /** Current dispute phase. */
  phase: Phase;
  /**
   * Winning categorical option. CONTRACT: meaningful ONLY when
   * `phase == Resolved`; on `InvalidDeadend` it is the `0xFF` sentinel and on
   * any non-terminal phase it is its zeroed default — use
   * {@link resolvedOptionOrNull}, never this field raw.
   */
  resolvedOption: number;
}

/** Read the three oracle fields kassandra-market needs from raw account bytes. */
export function decodeMarketOracle(data: Uint8Array): MarketOracle {
  if (data.length <= ORACLE_RESOLVED_OPTION_OFFSET) {
    throw new Error(
      `Oracle: too short — need > ${ORACLE_RESOLVED_OPTION_OFFSET} bytes, got ${data.length}.`,
    );
  }
  const dv = view(data);
  return {
    optionsCount: readU8(dv, ORACLE_OPTIONS_COUNT_OFFSET),
    phase: readU8(dv, ORACLE_PHASE_OFFSET) as Phase,
    resolvedOption: readU8(dv, ORACLE_RESOLVED_OPTION_OFFSET),
  };
}

/**
 * True when the oracle has reached a terminal phase — `Resolved` (a winning
 * option was stamped) or `InvalidDeadend` (tie / no survivors). `cancel` requires
 * a terminal oracle; `resolve_market` requires specifically `Resolved`.
 */
export function isTerminal(phase: Phase): boolean {
  return phase === Phase.Resolved || phase === Phase.InvalidDeadend;
}

/**
 * The winning option, or `null` unless the oracle is `Resolved`. Guards against
 * reading the `0xFF` `InvalidDeadend` sentinel (or a pre-finalize zero) as a real
 * outcome.
 */
export function resolvedOptionOrNull(oracle: MarketOracle): number | null {
  return oracle.phase === Phase.Resolved ? oracle.resolvedOption : null;
}
