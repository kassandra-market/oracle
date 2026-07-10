mod common;
use common::*;

#[test]
fn seed_disputed_oracle_has_two_conflicting_proposers() {
    let mut ctx = TestCtx::new();
    let oracle = ctx.seed_disputed_oracle(&[
        ProposerSpec {
            option: 0,
            bond: 1_000,
        },
        ProposerSpec {
            option: 1,
            bond: 1_000,
        },
    ]);
    let acc = ctx.oracle(oracle);
    assert_eq!(
        acc.phase,
        kassandra_oracles_program::state::Phase::FactProposal as u8
    );
    assert_eq!(acc.proposer_count, 2);
    assert_eq!(acc.total_oracle_stake, 2_000);
}
