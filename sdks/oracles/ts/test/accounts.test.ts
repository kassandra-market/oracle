/**
 * D2 — Pod account decoder tests.
 *
 * For each of the 7 account types we build a synthetic buffer of the EXACT
 * pinned size, write KNOWN values at the EXACT little-endian offsets (mirroring
 * `programs/oracles/src/state.rs` + the pins in `tests/state_layout.rs`),
 * decode it, and assert every field round-trips (esp. pubkeys, u64 bigints, the
 * LE encoding, and the Fact uri sliced by uri_len). We also assert each decoder
 * REJECTS a wrong account_type tag and a wrong length.
 *
 * The STRONGEST check — a litesvm test that creates a REAL `Protocol` account
 * via `init_protocol`, fetches the raw bytes, and decodes them with the SDK —
 * lives in accounts-litesvm.test.ts. Shared fixtures live in ./helpers/accounts.ts.
 */
import { describe, expect, it } from "vitest";

import { AccountType, ACCOUNT_SIZES, Phase } from "../src/constants.js";
import {
  decodeAiClaim,
  decodeFact,
  decodeFactVote,
  decodeMarket,
  decodeOracle,
  decodeProposer,
  decodeProtocol,
  VoteKind,
} from "../src/accounts/index.js";
import { Buf, key32, key32Addr } from "./helpers/accounts.js";

describe("Pod account decoders — synthetic buffers at pinned offsets", () => {
  it("decodes Protocol (368) with every field", () => {
    const b = new Buf(ACCOUNT_SIZES.Protocol, AccountType.Protocol)
      .raw(8, key32(1)) // admin
      .raw(40, key32(2)) // kass_mint
      .raw(72, key32(3)) // usdc_mint
      .u64(104, 1234n) // fee_ema
      .i64(112, 1_700_000_000n) // last_creation_unix
      .u8(120, 251) // bump
      .u8(121, 1) // governance_set
      .raw(128, key32(4)) // dao_authority
      .raw(160, key32(5)) // kass_dao
      .u64(192, 7n) // emission_num
      .u64(200, 100n) // emission_den
      .u64(208, 999n) // total_supply_cap
      .i64(216, 86400n) // fee_ema_halflife
      .u64(224, 11n) // fee_per_ema_unit
      .u64(232, 22n) // fee_ema_increment
      .u64(240, 2n) // threshold_num
      .u64(248, 3n) // threshold_den
      .u64(256, 1n) // market_threshold_num
      .u64(264, 10n) // market_threshold_den
      .u64(272, 1n) // flip_slash_num
      .u64(280, 2n) // flip_slash_den
      .i64(288, 3600n) // phase_window
      .i64(296, 3601n) // proposal_window
      .u64(304, 1n) // fact_vote_slash_num
      .u64(312, 2n) // fact_vote_slash_den
      .u64(320, 2n) // reward_proposer_weight
      .u64(328, 1n) // reward_fact_weight
      .u64(336, 1n) // challenge_fail_usdc_fee_num
      .u64(344, 100n) // challenge_fail_usdc_fee_den
      .u64(352, 1n) // challenge_success_kass_fee_num
      .u64(360, 100n); // challenge_success_kass_fee_den

    const p = decodeProtocol(b.bytes);
    expect(p.accountType).toBe(AccountType.Protocol);
    expect(p.admin.toString()).toBe(key32Addr(1));
    expect(p.kassMint.toString()).toBe(key32Addr(2));
    expect(p.usdcMint.toString()).toBe(key32Addr(3));
    expect(p.feeEma).toBe(1234n);
    expect(p.lastCreationUnix).toBe(1_700_000_000n);
    expect(p.bump).toBe(251);
    expect(p.governanceSet).toBe(true);
    expect(p.daoAuthority.toString()).toBe(key32Addr(4));
    expect(p.kassDao.toString()).toBe(key32Addr(5));
    expect(p.emissionNum).toBe(7n);
    expect(p.emissionDen).toBe(100n);
    expect(p.totalSupplyCap).toBe(999n);
    expect(p.feeEmaHalflife).toBe(86400n);
    expect(p.feePerEmaUnit).toBe(11n);
    expect(p.feeEmaIncrement).toBe(22n);
    expect(p.thresholdNum).toBe(2n);
    expect(p.thresholdDen).toBe(3n);
    expect(p.marketThresholdNum).toBe(1n);
    expect(p.marketThresholdDen).toBe(10n);
    expect(p.flipSlashNum).toBe(1n);
    expect(p.flipSlashDen).toBe(2n);
    expect(p.phaseWindow).toBe(3600n);
    expect(p.proposalWindow).toBe(3601n);
    expect(p.factVoteSlashNum).toBe(1n);
    expect(p.factVoteSlashDen).toBe(2n);
    expect(p.rewardProposerWeight).toBe(2n);
    expect(p.rewardFactWeight).toBe(1n);
    expect(p.challengeFailUsdcFeeNum).toBe(1n);
    expect(p.challengeFailUsdcFeeDen).toBe(100n);
    expect(p.challengeSuccessKassFeeNum).toBe(1n);
    expect(p.challengeSuccessKassFeeDen).toBe(100n);
  });

  it("decodes Oracle (360) with every field, incl. Phase enum + 0xFF resolved_option", () => {
    const b = new Buf(ACCOUNT_SIZES.Oracle, AccountType.Oracle)
      .raw(8, key32(10)) // creator
      .raw(40, key32(11)) // kass_mint
      .raw(72, key32(12)) // usdc_mint
      .raw(104, key32(13)) // stake_vault
      .i64(136, 1_800_000_000n) // deadline
      .i64(144, 1_800_003_600n) // phase_ends_at
      .i64(152, 900n) // twap_window
      .u8(160, 4) // options_count
      .u8(161, Phase.Challenge) // phase = 5
      .u16(162, 7) // proposer_count
      .u16(164, 5) // surviving_count
      .u16(166, 9) // fact_count
      .u64(168, 5_000_000n) // total_oracle_stake
      .u64(176, 2_000_000n) // bond_pool
      .u64(184, 6_000_000n) // dispute_bond_total
      .u16(192, 3) // settled_count
      .u16(194, 2) // ai_finalized_count
      .u8(196, 249) // bump
      .u8(197, 0xff) // resolved_option (dead-end sentinel)
      .u16(198, 1) // open_challenge_count
      // (former prompt_hash @200 removed — governable block shifted -32.)
      .u64(200, 2n) // threshold_num
      .u64(208, 3n) // threshold_den
      .u64(216, 1n) // market_threshold_num
      .u64(224, 10n) // market_threshold_den
      .u64(232, 1n) // flip_slash_num
      .u64(240, 2n) // flip_slash_den
      .i64(248, 3600n) // phase_window
      .i64(256, 3601n) // proposal_window
      .u64(264, 1n) // fact_vote_slash_num
      .u64(272, 2n) // fact_vote_slash_den
      .u64(280, 2n) // reward_proposer_weight
      .u64(288, 1n) // reward_fact_weight
      .u64(296, 1n) // challenge_fail_usdc_fee_num
      .u64(304, 100n) // challenge_fail_usdc_fee_den
      .u64(312, 1n) // challenge_success_kass_fee_num
      .u64(320, 100n) // challenge_success_kass_fee_den
      .u64(328, 3_000_000n) // total_correct_proposer_stake
      .u64(336, 4_000_000n) // total_approved_fact_stake
      .u64(344, 7_000_000n) // reward_pool
      .u64(352, 1_500_000n); // reward_emission

    const o = decodeOracle(b.bytes);
    expect(o.accountType).toBe(AccountType.Oracle);
    expect(o.creator.toString()).toBe(key32Addr(10));
    expect(o.kassMint.toString()).toBe(key32Addr(11));
    expect(o.usdcMint.toString()).toBe(key32Addr(12));
    expect(o.stakeVault.toString()).toBe(key32Addr(13));
    expect(o.deadline).toBe(1_800_000_000n);
    expect(o.phaseEndsAt).toBe(1_800_003_600n);
    expect(o.twapWindow).toBe(900n);
    expect(o.optionsCount).toBe(4);
    expect(o.phaseRaw).toBe(5);
    expect(o.phase).toBe(Phase.Challenge);
    expect(o.proposerCount).toBe(7);
    expect(o.survivingCount).toBe(5);
    expect(o.factCount).toBe(9);
    expect(o.totalOracleStake).toBe(5_000_000n);
    expect(o.bondPool).toBe(2_000_000n);
    expect(o.disputeBondTotal).toBe(6_000_000n);
    expect(o.settledCount).toBe(3);
    expect(o.aiFinalizedCount).toBe(2);
    expect(o.bump).toBe(249);
    expect(o.resolvedOption).toBe(0xff);
    expect(o.openChallengeCount).toBe(1);
    expect(o.thresholdNum).toBe(2n);
    expect(o.thresholdDen).toBe(3n);
    expect(o.marketThresholdNum).toBe(1n);
    expect(o.marketThresholdDen).toBe(10n);
    expect(o.flipSlashNum).toBe(1n);
    expect(o.flipSlashDen).toBe(2n);
    expect(o.phaseWindow).toBe(3600n);
    expect(o.proposalWindow).toBe(3601n);
    expect(o.factVoteSlashNum).toBe(1n);
    expect(o.factVoteSlashDen).toBe(2n);
    expect(o.rewardProposerWeight).toBe(2n);
    expect(o.rewardFactWeight).toBe(1n);
    expect(o.challengeFailUsdcFeeNum).toBe(1n);
    expect(o.challengeFailUsdcFeeDen).toBe(100n);
    expect(o.challengeSuccessKassFeeNum).toBe(1n);
    expect(o.challengeSuccessKassFeeDen).toBe(100n);
    expect(o.totalCorrectProposerStake).toBe(3_000_000n);
    expect(o.totalApprovedFactStake).toBe(4_000_000n);
    expect(o.rewardPool).toBe(7_000_000n);
    expect(o.rewardEmission).toBe(1_500_000n);
  });

  it("decodes Proposer (96) with every field + bool flags", () => {
    const b = new Buf(ACCOUNT_SIZES.Proposer, AccountType.Proposer)
      .raw(8, key32(20)) // oracle
      .raw(40, key32(21)) // authority
      .u64(72, 50_000n) // bond
      .u8(80, 2) // original_option
      .u8(81, 0xff) // claim_option (CLAIM_OPTION_NONE)
      .u8(82, 1) // disqualified
      .u8(83, 0) // slashed
      .u8(84, 1) // flipped
      .u8(85, 247) // bump
      .u8(86, 1) // ai_finalized
      .u64(88, 25_000n); // slashed_amount

    const p = decodeProposer(b.bytes);
    expect(p.accountType).toBe(AccountType.Proposer);
    expect(p.oracle.toString()).toBe(key32Addr(20));
    expect(p.authority.toString()).toBe(key32Addr(21));
    expect(p.bond).toBe(50_000n);
    expect(p.originalOption).toBe(2);
    expect(p.claimOption).toBe(0xff);
    expect(p.disqualified).toBe(true);
    expect(p.slashed).toBe(false);
    expect(p.flipped).toBe(true);
    expect(p.bump).toBe(247);
    expect(p.aiFinalized).toBe(true);
    expect(p.slashedAmount).toBe(25_000n);
  });

  it("decodes Fact (336) incl. uri sliced by uri_len", () => {
    const uriStr = "ipfs://bafy-some-fact-uri";
    const uriBytes = new TextEncoder().encode(uriStr);
    const b = new Buf(ACCOUNT_SIZES.Fact, AccountType.Fact)
      .raw(8, key32(30)) // oracle
      .raw(40, key32(31)) // proposer
      .raw(72, key32(32)) // content_hash
      .u64(104, 1_000n) // stake
      .u64(112, 2_000n) // approve_stake
      .u64(120, 500n) // duplicate_stake
      .u16(128, uriBytes.length) // uri_len
      .u8(130, 1) // agreed
      .u8(131, 0) // duplicate
      .u8(132, 1) // settled
      .u8(133, 240) // bump
      .raw(136, uriBytes); // uri (+ trailing zeros to 200)

    const f = decodeFact(b.bytes);
    expect(f.accountType).toBe(AccountType.Fact);
    expect(f.oracle.toString()).toBe(key32Addr(30));
    expect(f.proposer.toString()).toBe(key32Addr(31));
    expect(Array.from(f.contentHash)).toEqual(Array.from(key32(32)));
    expect(f.stake).toBe(1_000n);
    expect(f.approveStake).toBe(2_000n);
    expect(f.duplicateStake).toBe(500n);
    expect(f.uriLen).toBe(uriBytes.length);
    expect(f.agreed).toBe(true);
    expect(f.duplicate).toBe(false);
    expect(f.settled).toBe(true);
    expect(f.bump).toBe(240);
    expect(f.uri).toBe(uriStr); // sliced by uri_len — no trailing NULs
    expect(f.uriRaw.length).toBe(200);
  });

  it("decodes FactVote (88) incl. VoteKind enum", () => {
    const b = new Buf(ACCOUNT_SIZES.FactVote, AccountType.FactVote)
      .raw(8, key32(40)) // fact
      .raw(40, key32(41)) // voter
      .u64(72, 12_345n) // stake
      .u8(80, 1) // kind = duplicate
      .u8(81, 233); // bump

    const v = decodeFactVote(b.bytes);
    expect(v.accountType).toBe(AccountType.FactVote);
    expect(v.fact.toString()).toBe(key32Addr(40));
    expect(v.voter.toString()).toBe(key32Addr(41));
    expect(v.stake).toBe(12_345n);
    expect(v.kindRaw).toBe(1);
    expect(v.kind).toBe(VoteKind.Duplicate);
    expect(v.bump).toBe(233);
  });

  it("decodes AiClaim (208) incl. appended authority @176", () => {
    const b = new Buf(ACCOUNT_SIZES.AiClaim, AccountType.AiClaim)
      .raw(8, key32(50)) // oracle
      .raw(40, key32(51)) // proposer
      .raw(72, key32(52)) // model_id
      .raw(104, key32(53)) // params_hash
      .raw(136, key32(54)) // io_hash
      .u8(168, 3) // option
      .u8(169, 1) // challenged
      .u8(170, 231) // bump
      .raw(176, key32(55)); // authority

    const c = decodeAiClaim(b.bytes);
    expect(c.accountType).toBe(AccountType.AiClaim);
    expect(c.oracle.toString()).toBe(key32Addr(50));
    expect(c.proposer.toString()).toBe(key32Addr(51));
    expect(Array.from(c.modelId)).toEqual(Array.from(key32(52)));
    expect(Array.from(c.paramsHash)).toEqual(Array.from(key32(53)));
    expect(Array.from(c.ioHash)).toEqual(Array.from(key32(54)));
    expect(c.option).toBe(3);
    expect(c.challenged).toBe(true);
    expect(c.bump).toBe(231);
    expect(c.authority.toString()).toBe(key32Addr(55));
  });

  it("decodes Market (416) with all 12 pubkeys + tail", () => {
    const b = new Buf(ACCOUNT_SIZES.Market, AccountType.Market)
      .raw(8, key32(60)) // oracle
      .raw(40, key32(61)) // ai_claim
      .raw(72, key32(62)) // proposer
      .raw(104, key32(63)) // challenger
      .raw(136, key32(64)) // question
      .raw(168, key32(65)) // kass_vault
      .raw(200, key32(66)) // usdc_vault
      .raw(232, key32(67)) // pass_amm
      .raw(264, key32(68)) // fail_amm
      .raw(296, key32(69)) // oracle_pass_kass
      .raw(328, key32(70)) // oracle_fail_kass
      .raw(360, key32(71)) // challenger_usdc_vault
      .i64(392, 1_900_000_000n) // twap_end
      .u64(400, 8_888n) // challenger_usdc
      .u8(408, 1) // settled
      .u8(409, 229); // bump

    const m = decodeMarket(b.bytes);
    expect(m.accountType).toBe(AccountType.Market);
    expect(m.oracle.toString()).toBe(key32Addr(60));
    expect(m.aiClaim.toString()).toBe(key32Addr(61));
    expect(m.proposer.toString()).toBe(key32Addr(62));
    expect(m.challenger.toString()).toBe(key32Addr(63));
    expect(m.question.toString()).toBe(key32Addr(64));
    expect(m.kassVault.toString()).toBe(key32Addr(65));
    expect(m.usdcVault.toString()).toBe(key32Addr(66));
    expect(m.passAmm.toString()).toBe(key32Addr(67));
    expect(m.failAmm.toString()).toBe(key32Addr(68));
    expect(m.oraclePassKass.toString()).toBe(key32Addr(69));
    expect(m.oracleFailKass.toString()).toBe(key32Addr(70));
    expect(m.challengerUsdcVault.toString()).toBe(key32Addr(71));
    expect(m.twapEnd).toBe(1_900_000_000n);
    expect(m.challengerUsdc).toBe(8_888n);
    expect(m.settled).toBe(true);
    expect(m.bump).toBe(229);
  });
});

describe("Pod account decoders — type-confusion + length rejection", () => {
  it("rejects a wrong account_type tag", () => {
    const buf = new Uint8Array(ACCOUNT_SIZES.Protocol);
    buf[0] = AccountType.Oracle; // wrong tag for a Protocol-sized buffer
    expect(() => decodeProtocol(buf)).toThrow(/wrong account_type/);
  });

  it("rejects a wrong length", () => {
    const buf = new Uint8Array(ACCOUNT_SIZES.Oracle - 1);
    buf[0] = AccountType.Oracle;
    expect(() => decodeOracle(buf)).toThrow(/wrong account size/);
  });

  it("rejects an Oracle buffer fed to decodeFact (type confusion)", () => {
    const buf = new Uint8Array(ACCOUNT_SIZES.Fact);
    buf[0] = AccountType.Oracle;
    expect(() => decodeFact(buf)).toThrow(/wrong account_type/);
  });
});
