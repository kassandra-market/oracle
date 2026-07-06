/**
 * Parity guard — the drift detector.
 *
 * The values on the left are HARDCODED copies of the program's pinned source of
 * truth (`instruction.rs`, `state.rs`, `error.rs`, `tests/state_layout.rs`).
 * If the SDK constants ever drift from these, this test fails. To change a
 * pinned value you must edit it in BOTH the program and here — a deliberate,
 * visible break, mirroring the program's own `state_layout.rs` philosophy.
 */
import { describe, expect, it } from "vitest";

import {
  ACCOUNT_SIZES,
  AccountType,
  Ix,
  MARKET_PROGRAM_ID,
  MarketError,
  MarketStatus,
  decodeError,
} from "../src/constants.js";
import {
  AMM_ACCOUNT_DISCRIMINATOR,
  DISC as METADAO_DISC,
} from "../src/metadao/constants.js";
import { resolveQuestion, splitTokens } from "../src/metadao/vault.js";
import { addLiquidity } from "../src/metadao/amm.js";

describe("parity guard: Ix discriminants (instruction.rs 0..=10)", () => {
  // Pinned from programs/kassandra-market/src/instruction.rs.
  const PINNED: Record<string, number> = {
    InitConfig: 0,
    UpdateConfig: 1,
    CreateMarket: 2,
    Contribute: 3,
    Cancel: 4,
    Refund: 5,
    Activate: 6,
    ClaimLp: 7,
    ResolveMarket: 8,
    CollectFee: 9,
    CloseMarket: 10,
  };

  it("matches every Ix by name and value", () => {
    for (const [name, value] of Object.entries(PINNED)) {
      expect(Ix[name as keyof typeof Ix], `Ix.${name}`).toBe(value);
    }
  });

  it("has exactly 11 instructions (0..=10), no more, no fewer", () => {
    const numericValues = Object.values(Ix).filter((v) => typeof v === "number");
    expect(numericValues.sort((a, b) => (a as number) - (b as number))).toEqual(
      Array.from({ length: 11 }, (_, i) => i),
    );
  });
});

describe("parity guard: AccountType (state.rs 0..=3)", () => {
  const PINNED: Record<string, number> = {
    Uninitialized: 0,
    Config: 1,
    Market: 2,
    Contribution: 3,
  };

  it("matches every AccountType by name and value", () => {
    for (const [name, value] of Object.entries(PINNED)) {
      expect(AccountType[name as keyof typeof AccountType], `AccountType.${name}`).toBe(value);
    }
  });
});

describe("parity guard: MarketStatus (state.rs 0..=4)", () => {
  const PINNED: Record<string, number> = {
    Funding: 0,
    Active: 1,
    Resolved: 2,
    Void: 3,
    Cancelled: 4,
  };

  it("matches every MarketStatus by name and value", () => {
    for (const [name, value] of Object.entries(PINNED)) {
      expect(MarketStatus[name as keyof typeof MarketStatus], `MarketStatus.${name}`).toBe(value);
    }
  });
});

describe("parity guard: account sizes (tests/state_layout.rs)", () => {
  // Pinned absolute on-chain ABI sizes from `account_sizes_are_stable`.
  const PINNED = {
    Config: 120,
    Market: 400,
    Contribution: 88,
  } as const;

  it("matches every pinned account size", () => {
    expect(ACCOUNT_SIZES).toEqual(PINNED);
  });
});

describe("parity guard: MarketError (error.rs 0..=21)", () => {
  // Pinned from programs/kassandra-market/src/error.rs.
  const PINNED: Record<string, number> = {
    InvalidAccount: 0,
    Unauthorized: 1,
    AlreadyInitialized: 2,
    InvalidSplit: 3,
    ZeroAmount: 4,
    NotFunding: 5,
    OracleNotTerminal: 6,
    AlreadyFunded: 7,
    AlreadyClaimed: 8,
    NotCancelled: 9,
    OracleResolved: 10,
    NotBinary: 11,
    WrongMint: 12,
    NotFunded: 13,
    PoolNotEmpty: 14,
    NotActive: 15,
    AlreadySettled: 16,
    InvalidFee: 17,
    FeeNotCollected: 18,
    InvalidOutcome: 19,
    ContributionsOpen: 20,
    NotSettled: 21,
  };

  it("matches every MarketError by name and value", () => {
    for (const [name, value] of Object.entries(PINNED)) {
      expect(MarketError[name as keyof typeof MarketError], `MarketError.${name}`).toBe(value);
    }
  });

  it("has exactly 22 errors (0..=21)", () => {
    const numericValues = Object.values(MarketError).filter((v) => typeof v === "number");
    expect(numericValues.length).toBe(22);
  });

  it("decodeError round-trips known codes", () => {
    expect(decodeError(MarketError.OracleNotTerminal)).toBe(MarketError.OracleNotTerminal);
    expect(decodeError(MarketError.AlreadySettled)).toBe(MarketError.AlreadySettled);
  });

  it("decodeError returns null for unknown codes", () => {
    expect(decodeError(9999)).toBeNull();
  });
});

describe("parity guard: program id", () => {
  it("is the pinned kassandra-market program id", () => {
    expect(MARKET_PROGRAM_ID.toString()).toBe("FEGNHWAB7kc7VC9CCwbvVPsv4Jykz2r2WQ758V4xCT9S");
  });
});

// ── MetaDAO wire-format parity (sdk-rs/src/metadao.rs) ────────────────────────

/** Hex-encode a byte array for byte-exact comparison against the Rust pins. */
function hex(bytes: Uint8Array): string {
  return Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
}

describe("parity guard: MetaDAO instruction discriminators (sdk-rs/src/metadao.rs)", () => {
  // Hardcoded copies of the Anchor discriminators pinned in sdk-rs/src/metadao.rs.
  // crankThatTwap is NOT in sdk-rs (never composed Rust-side); pinned from the
  // binary-validated sibling AMM SDK.
  const PINNED: Record<string, string> = {
    initializeQuestion: "f5976abc582c41d4",
    initializeConditionalVault: "2558fad436dae3af",
    splitTokens: "4fc374008cb049b3",
    mergeTokens: "e259fb79e182b40e",
    redeemTokens: "f662862998217845",
    resolveQuestion: "3420e0b3b40800f6",
    createAmm: "f25b15aa05447d40",
    addLiquidity: "b59d59438fb63448",
    swap: "f8c69e91e17587c8",
    crankThatTwap: "dc6419f9005cc3c1",
  };

  for (const [name, expected] of Object.entries(PINNED)) {
    it(`DISC.${name} == ${expected}`, () => {
      expect(hex(METADAO_DISC[name as keyof typeof METADAO_DISC])).toBe(expected);
    });
  }

  it("Amm account discriminator == 8ff5c8114ad6c487", () => {
    expect(hex(AMM_ACCOUNT_DISCRIMINATOR)).toBe("8ff5c8114ad6c487");
  });
});

describe("parity guard: MetaDAO arg encoders (byte-exact vs sdk-rs)", () => {
  it("resolveQuestion([1,0]) data = disc ++ 02000000 ++ 01000000 ++ 00000000", async () => {
    const ix = await resolveQuestion({
      question: MARKET_PROGRAM_ID,
      oracle: MARKET_PROGRAM_ID,
      payoutNumerators: [1, 0],
    });
    expect(hex(ix.data)).toBe("3420e0b3b40800f6" + "02000000" + "01000000" + "00000000");
  });

  it("splitTokens(amount) data = disc ++ u64le(amount)", async () => {
    const ix = await splitTokens({
      question: MARKET_PROGRAM_ID,
      vault: MARKET_PROGRAM_ID,
      vaultUnderlyingAta: MARKET_PROGRAM_ID,
      authority: MARKET_PROGRAM_ID,
      userUnderlyingAta: MARKET_PROGRAM_ID,
      conditionalMints: [MARKET_PROGRAM_ID, MARKET_PROGRAM_ID],
      userConditionalAtas: [MARKET_PROGRAM_ID, MARKET_PROGRAM_ID],
      amount: 1_000_000n,
    });
    // 1_000_000 = 0x0f4240 → LE u64 = 40420f0000000000
    expect(hex(ix.data)).toBe("4fc374008cb049b3" + "40420f0000000000");
  });

  it("addLiquidity(q,b,l) data = disc ++ u64le(q) ++ u64le(b) ++ u64le(l)", async () => {
    const ix = await addLiquidity({
      payer: MARKET_PROGRAM_ID,
      baseMint: MARKET_PROGRAM_ID,
      quoteMint: MARKET_PROGRAM_ID,
      quoteAmount: 1n,
      maxBaseAmount: 2n,
      minLpTokens: 3n,
    });
    expect(hex(ix.data)).toBe(
      "b59d59438fb63448" +
        "0100000000000000" +
        "0200000000000000" +
        "0300000000000000",
    );
  });
});
