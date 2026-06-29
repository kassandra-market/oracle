use core::mem::{offset_of, size_of};
use kassandra_program::state::*;

#[test]
fn account_sizes_are_stable() {
    // size_of and LEN must agree (LEN is defined as size_of).
    assert_eq!(size_of::<Oracle>(), Oracle::LEN);
    assert_eq!(size_of::<Proposer>(), Proposer::LEN);
    assert_eq!(size_of::<Fact>(), Fact::LEN);
    assert_eq!(size_of::<FactVote>(), FactVote::LEN);
    assert_eq!(size_of::<AiClaim>(), AiClaim::LEN);

    // Absolute pinned on-chain ABI sizes. Changing a struct's layout must
    // be a deliberate, visible break of these constants. Each carries an
    // 8-byte header (account_type: u8 + _pad_hdr: [u8;7]) at offset 0.
    assert_eq!(Oracle::LEN, 232);
    assert_eq!(Proposer::LEN, 88);
    assert_eq!(Fact::LEN, 336);
    assert_eq!(FactVote::LEN, 88);
    assert_eq!(AiClaim::LEN, 176);
}

#[test]
fn field_offsets_are_pinned() {
    // The account_type discriminator is the very first byte of every struct.
    assert_eq!(offset_of!(Oracle, account_type), 0);
    assert_eq!(offset_of!(Proposer, account_type), 0);
    assert_eq!(offset_of!(Fact, account_type), 0);
    assert_eq!(offset_of!(FactVote, account_type), 0);
    assert_eq!(offset_of!(AiClaim, account_type), 0);

    // Lock a few key field offsets per struct so reordering/resizing breaks.
    // (All shifted +8 by the header relative to the pre-tag layout.)
    assert_eq!(offset_of!(Oracle, proposer_count), 162);
    assert_eq!(offset_of!(Oracle, surviving_count), 164);
    assert_eq!(offset_of!(Oracle, total_oracle_stake), 168);
    assert_eq!(offset_of!(Oracle, bond_pool), 176);
    assert_eq!(offset_of!(Oracle, dispute_bond_total), 184);
    assert_eq!(offset_of!(Oracle, settled_count), 192);
    assert_eq!(offset_of!(Oracle, prompt_hash), 200);

    assert_eq!(offset_of!(Proposer, bond), 72);

    assert_eq!(offset_of!(Fact, uri), 136);

    assert_eq!(offset_of!(FactVote, stake), 72);

    assert_eq!(offset_of!(AiClaim, io_hash), 136);
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
