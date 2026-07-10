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
  KASSANDRA_PROGRAM_ID,
  KassandraError,
  Phase,
  decodeError,
} from "../src/constants.js";

describe("parity guard: Ix discriminants (instruction.rs 0..=23)", () => {
  // Pinned from programs/oracles/src/instruction.rs.
  const PINNED: Record<string, number> = {
    SubmitFact: 0,
    VoteFact: 1,
    FinalizeFacts: 2,
    SubmitAiClaim: 3,
    OpenChallenge: 4,
    SettleChallenge: 5,
    FinalizeOracle: 6,
    AdvancePhase: 7,
    FinalizeAiClaims: 8,
    InitProtocol: 9,
    CreateOracle: 10,
    Propose: 11,
    FinalizeProposals: 12,
    SetGovernance: 13,
    SetConfig: 14,
    ResolveDeadend: 15,
    KassPrice: 16,
    ClaimProposer: 17,
    ClaimFact: 18,
    ClaimFactVote: 19,
    CloseAiClaim: 20,
    CloseMarket: 21,
    SweepOracle: 22,
  };

  it("matches every Ix by name and value", () => {
    for (const [name, value] of Object.entries(PINNED)) {
      expect(Ix[name as keyof typeof Ix], `Ix.${name}`).toBe(value);
    }
  });

  it("has exactly 24 instructions (0..=23), no more, no fewer", () => {
    const numericValues = Object.values(Ix).filter((v) => typeof v === "number");
    expect(numericValues.sort((a, b) => (a as number) - (b as number))).toEqual(
      Array.from({ length: 24 }, (_, i) => i),
    );
  });
});

describe("parity guard: AccountType (state.rs 0..=7)", () => {
  const PINNED: Record<string, number> = {
    Uninitialized: 0,
    Oracle: 1,
    Proposer: 2,
    Fact: 3,
    FactVote: 4,
    AiClaim: 5,
    Market: 6,
    Protocol: 7,
  };

  it("matches every AccountType by name and value", () => {
    for (const [name, value] of Object.entries(PINNED)) {
      expect(AccountType[name as keyof typeof AccountType], `AccountType.${name}`).toBe(value);
    }
  });
});

describe("parity guard: Phase (state.rs 0..=8)", () => {
  it("matches every Phase by name and value", () => {
    const PINNED: Record<string, number> = {
      Created: 0,
      Proposal: 1,
      FactProposal: 2,
      FactVoting: 3,
      AiClaim: 4,
      Challenge: 5,
      FinalRecompute: 6,
      Resolved: 7,
      InvalidDeadend: 8,
    };
    for (const [name, value] of Object.entries(PINNED)) {
      expect(Phase[name as keyof typeof Phase], `Phase.${name}`).toBe(value);
    }
  });
});

describe("parity guard: account sizes (tests/state_layout.rs)", () => {
  // Pinned absolute on-chain ABI sizes from `account_sizes_are_stable`.
  const PINNED = {
    Protocol: 392,
    Oracle: 368,
    Proposer: 96,
    Fact: 336,
    FactVote: 88,
    AiClaim: 208,
    Market: 416,
  } as const;

  it("matches every pinned account size", () => {
    expect(ACCOUNT_SIZES).toEqual(PINNED);
  });
});

describe("parity guard: KassandraError (error.rs 0..=36)", () => {
  // Pinned from programs/oracles/src/error.rs.
  const PINNED: Record<string, number> = {
    NotImplemented: 0,
    WrongPhase: 1,
    WindowClosed: 2,
    WindowNotElapsed: 3,
    Unauthorized: 4,
    InvalidAccount: 5,
    DuplicateFact: 6,
    ZeroStake: 7,
    DuplicateVote: 8,
    IncompleteFactSet: 9,
    AlreadySettled: 10,
    NoDisputeBond: 11,
    DuplicateClaim: 12,
    InvalidOption: 13,
    AlreadyChallenged: 14,
    TwapWindowOpen: 15,
    ChallengesOutstanding: 16,
    AlreadyInitialized: 17,
    InvalidDeadline: 18,
    InvalidOptionsCount: 19,
    DeadlineNotReached: 20,
    ProposalWindowClosed: 21,
    TooManyProposers: 22,
    DuplicateProposer: 23,
    NoProposals: 24,
    GovernanceAlreadySet: 25,
    InvalidConfig: 26,
    VotersOutstanding: 27,
    BadMintAuthority: 28,
    MarketNotSettled: 29,
    EscrowNotEmpty: 30,
    InvalidFutarchyDao: 31,
    DaoAuthorityMismatch: 32,
    SweepGraceNotElapsed: 33,
    GovernanceNotSet: 34,
    InvalidTreasury: 35,
    BelowMinStake: 36,
  };

  it("matches every KassandraError by name and value", () => {
    for (const [name, value] of Object.entries(PINNED)) {
      expect(KassandraError[name as keyof typeof KassandraError], `KassandraError.${name}`).toBe(value);
    }
  });

  it("has exactly 37 errors (0..=36)", () => {
    const numericValues = Object.values(KassandraError).filter((v) => typeof v === "number");
    expect(numericValues.length).toBe(37);
  });

  it("decodeError maps a custom code to its name + a non-empty message", () => {
    const decoded = decodeError(KassandraError.TwapWindowOpen);
    expect(decoded.name).toBe("TwapWindowOpen");
    expect(decoded.message.length).toBeGreaterThan(0);
  });

  it("decodeError handles unknown codes gracefully", () => {
    const decoded = decodeError(9999);
    expect(decoded.name).toBe("Unknown");
  });
});

describe("parity guard: program id", () => {
  it("is the pinned Kassandra program id", () => {
    expect(KASSANDRA_PROGRAM_ID.toString()).toBe("KassVxvXUEPr5apSr2MqiGva4VFtJXyYLLDFS3f83nY");
  });
});
