use core::mem::{offset_of, size_of};
use kassandra_oracles_program::state::*;

#[test]
fn account_sizes_are_stable() {
    // size_of and LEN must agree (LEN is defined as size_of).
    assert_eq!(size_of::<Oracle>(), Oracle::LEN);
    assert_eq!(size_of::<Proposer>(), Proposer::LEN);
    assert_eq!(size_of::<Fact>(), Fact::LEN);
    assert_eq!(size_of::<FactVote>(), FactVote::LEN);
    assert_eq!(size_of::<AiClaim>(), AiClaim::LEN);
    assert_eq!(size_of::<Market>(), Market::LEN);
    assert_eq!(size_of::<Protocol>(), Protocol::LEN);

    // Absolute pinned on-chain ABI sizes. Changing a struct's layout must
    // be a deliberate, visible break of these constants. Each carries an
    // 8-byte header (account_type: u8 + _pad_hdr: [u8;7]) at offset 0.
    // Oracle 360→368 and Protocol 368→392 grew by the bootstrapping stake-floor
    // fields (Oracle.min_stake; Protocol.stake_floor_ema_threshold/cap/max).
    assert_eq!(Oracle::LEN, 368);
    assert_eq!(Proposer::LEN, 96);
    assert_eq!(Fact::LEN, 336);
    assert_eq!(FactVote::LEN, 88);
    assert_eq!(AiClaim::LEN, 208);
    assert_eq!(Market::LEN, 416);
    assert_eq!(Protocol::LEN, 392);
}

#[test]
fn field_offsets_are_pinned() {
    // The account_type discriminator is the very first byte of every struct.
    assert_eq!(offset_of!(Oracle, account_type), 0);
    assert_eq!(offset_of!(Proposer, account_type), 0);
    assert_eq!(offset_of!(Fact, account_type), 0);
    assert_eq!(offset_of!(FactVote, account_type), 0);
    assert_eq!(offset_of!(AiClaim, account_type), 0);
    assert_eq!(offset_of!(Market, account_type), 0);

    // Lock a few key field offsets per struct so reordering/resizing breaks.
    // (All shifted +8 by the header relative to the pre-tag layout.)
    assert_eq!(offset_of!(Oracle, proposer_count), 162);
    assert_eq!(offset_of!(Oracle, surviving_count), 164);
    assert_eq!(offset_of!(Oracle, total_oracle_stake), 168);
    assert_eq!(offset_of!(Oracle, bond_pool), 176);
    assert_eq!(offset_of!(Oracle, dispute_bond_total), 184);
    assert_eq!(offset_of!(Oracle, settled_count), 192);
    assert_eq!(offset_of!(Oracle, ai_finalized_count), 194);
    // bump @196; resolved_option absorbs the former _pad1[1] @197.
    assert_eq!(offset_of!(Oracle, resolved_option), 197);
    assert_eq!(offset_of!(Oracle, open_challenge_count), 198);
    // The former `prompt_hash` [u8;32] @200 was removed; `threshold_num` (8-aligned)
    // now packs directly after `open_challenge_count`, shifting this block -32.
    // F2 governable-param snapshot block (8-byte aligned).
    assert_eq!(offset_of!(Oracle, threshold_num), 200);
    assert_eq!(offset_of!(Oracle, threshold_den), 208);
    assert_eq!(offset_of!(Oracle, market_threshold_num), 216);
    assert_eq!(offset_of!(Oracle, market_threshold_den), 224);
    assert_eq!(offset_of!(Oracle, flip_slash_num), 232);
    assert_eq!(offset_of!(Oracle, flip_slash_den), 240);
    assert_eq!(offset_of!(Oracle, phase_window), 248);
    assert_eq!(offset_of!(Oracle, proposal_window), 256);
    assert_eq!(offset_of!(Oracle, fact_vote_slash_num), 264);
    assert_eq!(offset_of!(Oracle, fact_vote_slash_den), 272);
    assert_eq!(offset_of!(Oracle, reward_proposer_weight), 280);
    assert_eq!(offset_of!(Oracle, reward_fact_weight), 288);
    // C1 challenge-fee config snapshot block.
    assert_eq!(offset_of!(Oracle, challenge_fail_usdc_fee_num), 296);
    assert_eq!(offset_of!(Oracle, challenge_fail_usdc_fee_den), 304);
    assert_eq!(offset_of!(Oracle, challenge_success_kass_fee_num), 312);
    assert_eq!(offset_of!(Oracle, challenge_success_kass_fee_den), 320);
    // S1 settlement resolution totals, packed after the C1 challenge-fee block.
    assert_eq!(offset_of!(Oracle, total_correct_proposer_stake), 328);
    assert_eq!(offset_of!(Oracle, total_approved_fact_stake), 336);
    assert_eq!(offset_of!(Oracle, reward_pool), 344);
    // S3 emission minted at creation, packed after the S1 totals.
    assert_eq!(offset_of!(Oracle, reward_emission), 352);
    // Bootstrapping stake floor, appended after reward_emission.
    assert_eq!(offset_of!(Oracle, min_stake), 360);

    assert_eq!(offset_of!(Proposer, bond), 72);
    assert_eq!(offset_of!(Proposer, ai_finalized), 86);
    assert_eq!(offset_of!(Proposer, slashed_amount), 88);

    assert_eq!(offset_of!(Fact, uri), 136);

    assert_eq!(offset_of!(FactVote, stake), 72);

    assert_eq!(offset_of!(AiClaim, io_hash), 136);
    // S4: `authority` appended at offset 176 (clean ABI addition; LEN 176 → 208).
    assert_eq!(offset_of!(AiClaim, authority), 176);

    // Market: 9 pubkeys packed after the 8-byte header, then the i64/u64 tail.
    assert_eq!(offset_of!(Market, oracle), 8);
    assert_eq!(offset_of!(Market, ai_claim), 40);
    assert_eq!(offset_of!(Market, question), 136);
    assert_eq!(offset_of!(Market, kass_vault), 168);
    assert_eq!(offset_of!(Market, usdc_vault), 200);
    assert_eq!(offset_of!(Market, pass_amm), 232);
    assert_eq!(offset_of!(Market, fail_amm), 264);
    assert_eq!(offset_of!(Market, oracle_pass_kass), 296);
    assert_eq!(offset_of!(Market, oracle_fail_kass), 328);
    assert_eq!(offset_of!(Market, challenger_usdc_vault), 360);
    assert_eq!(offset_of!(Market, twap_end), 392);
    assert_eq!(offset_of!(Market, challenger_usdc), 400);
    assert_eq!(offset_of!(Market, settled), 408);

    // Protocol: 3 pubkeys packed after the 8-byte header, then the fee-EMA tail,
    // then (F1) the governance flag + DAO linkage + governable monetary params.
    assert_eq!(offset_of!(Protocol, account_type), 0);
    assert_eq!(offset_of!(Protocol, admin), 8);
    assert_eq!(offset_of!(Protocol, kass_mint), 40);
    assert_eq!(offset_of!(Protocol, usdc_mint), 72);
    assert_eq!(offset_of!(Protocol, fee_ema), 104);
    assert_eq!(offset_of!(Protocol, last_creation_unix), 112);
    assert_eq!(offset_of!(Protocol, bump), 120);
    assert_eq!(offset_of!(Protocol, governance_set), 121);
    // _pad[6] @122 fills to the 8-byte boundary before the Pubkey pair.
    assert_eq!(offset_of!(Protocol, dao_authority), 128);
    assert_eq!(offset_of!(Protocol, kass_dao), 160);
    assert_eq!(offset_of!(Protocol, emission_num), 192);
    assert_eq!(offset_of!(Protocol, emission_den), 200);
    assert_eq!(offset_of!(Protocol, total_supply_cap), 208);
    assert_eq!(offset_of!(Protocol, fee_ema_halflife), 216);
    assert_eq!(offset_of!(Protocol, fee_per_ema_unit), 224);
    assert_eq!(offset_of!(Protocol, fee_ema_increment), 232);
    // F2 governable behavioral params (mutable source; snapshotted onto Oracle).
    assert_eq!(offset_of!(Protocol, threshold_num), 240);
    assert_eq!(offset_of!(Protocol, threshold_den), 248);
    assert_eq!(offset_of!(Protocol, market_threshold_num), 256);
    assert_eq!(offset_of!(Protocol, market_threshold_den), 264);
    assert_eq!(offset_of!(Protocol, flip_slash_num), 272);
    assert_eq!(offset_of!(Protocol, flip_slash_den), 280);
    assert_eq!(offset_of!(Protocol, phase_window), 288);
    assert_eq!(offset_of!(Protocol, proposal_window), 296);
    assert_eq!(offset_of!(Protocol, fact_vote_slash_num), 304);
    assert_eq!(offset_of!(Protocol, fact_vote_slash_den), 312);
    assert_eq!(offset_of!(Protocol, reward_proposer_weight), 320);
    assert_eq!(offset_of!(Protocol, reward_fact_weight), 328);
    // C1 challenge-fee config (mutable source; snapshotted onto Oracle).
    assert_eq!(offset_of!(Protocol, challenge_fail_usdc_fee_num), 336);
    assert_eq!(offset_of!(Protocol, challenge_fail_usdc_fee_den), 344);
    assert_eq!(offset_of!(Protocol, challenge_success_kass_fee_num), 352);
    assert_eq!(offset_of!(Protocol, challenge_success_kass_fee_den), 360);
    // Bootstrapping stake-floor curve, appended after the challenge fees.
    assert_eq!(offset_of!(Protocol, stake_floor_ema_threshold), 368);
    assert_eq!(offset_of!(Protocol, stake_floor_ema_cap), 376);
    assert_eq!(offset_of!(Protocol, stake_floor_max), 384);
}

#[test]
fn phase_discriminants_and_roundtrip() {
    assert_eq!(Phase::Created as u8, 0);
    assert_eq!(Phase::InvalidDeadend as u8, 8);

    assert!(Phase::from_u8(9).is_none());

    for v in [
        Phase::Created,
        Phase::Proposal,
        Phase::FactProposal,
        Phase::FactVoting,
        Phase::AiClaim,
        Phase::Challenge,
        Phase::FinalRecompute,
        Phase::Resolved,
        Phase::InvalidDeadend,
    ] {
        assert_eq!(Phase::from_u8(v as u8), Some(v));
        assert_eq!(v.as_u8(), v as u8);
    }
}
