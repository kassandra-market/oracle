//! Top-level instruction dispatch.
//!
//! [`process`] decodes the leading discriminant byte (see
//! [`crate::instruction::Ix`]) and routes to the matching processor. Each arm
//! is currently a [`KassandraError::NotImplemented`] placeholder; later tasks
//! replace them one at a time with real processors living in
//! `processor/<name>.rs`.

use pinocchio::{
    account::AccountView as AccountInfo, address::Address as Pubkey, error::ProgramError,
    ProgramResult,
};

use crate::instruction::Ix;

pub mod advance_phase;
pub mod claims;
pub mod close_ai_claim;
pub mod close_market;
pub mod create_oracle;
pub mod finalize_ai_claims;
pub mod finalize_facts;
pub mod finalize_oracle;
pub mod finalize_proposals;
pub mod guards;
pub mod init_protocol;
pub mod kass_price;
pub mod open_challenge;
pub mod propose;
pub mod resolve_deadend;
pub mod set_config;
pub mod set_governance;
pub mod settle_challenge;
pub mod submit_ai_claim;
pub mod submit_fact;
pub mod sweep_oracle;
pub mod vote_fact;
pub mod write_oracle_meta;

pub fn process(program_id: &Pubkey, accounts: &mut [AccountInfo], data: &[u8]) -> ProgramResult {
    // First byte = discriminant; the rest is the per-instruction payload.
    let (&disc, payload) = data
        .split_first()
        .ok_or(ProgramError::InvalidInstructionData)?;
    let ix = Ix::from_u8(disc).ok_or(ProgramError::InvalidInstructionData)?;

    match ix {
        Ix::SubmitFact => submit_fact::process(program_id, accounts, payload),
        Ix::VoteFact => vote_fact::process(program_id, accounts, payload),
        Ix::AdvancePhase => advance_phase::process(program_id, accounts, payload),
        Ix::FinalizeFacts => finalize_facts::process(program_id, accounts, payload),
        Ix::SubmitAiClaim => submit_ai_claim::process(program_id, accounts, payload),
        Ix::FinalizeAiClaims => finalize_ai_claims::process(program_id, accounts, payload),
        Ix::OpenChallenge => open_challenge::process(program_id, accounts, payload),
        Ix::SettleChallenge => settle_challenge::process(program_id, accounts, payload),
        Ix::FinalizeOracle => finalize_oracle::process(program_id, accounts, payload),
        Ix::InitProtocol => init_protocol::process(program_id, accounts, payload),
        Ix::CreateOracle => create_oracle::process(program_id, accounts, payload),
        Ix::Propose => propose::process(program_id, accounts, payload),
        Ix::FinalizeProposals => finalize_proposals::process(program_id, accounts, payload),
        Ix::SetGovernance => set_governance::process(program_id, accounts, payload),
        Ix::SetConfig => set_config::process(program_id, accounts, payload),
        Ix::ResolveDeadend => resolve_deadend::process(program_id, accounts, payload),
        Ix::KassPrice => kass_price::process(program_id, accounts, payload),
        Ix::ClaimProposer => claims::claim_proposer(program_id, accounts, payload),
        Ix::ClaimFact => claims::claim_fact(program_id, accounts, payload),
        Ix::ClaimFactVote => claims::claim_fact_vote(program_id, accounts, payload),
        Ix::CloseAiClaim => close_ai_claim::process(program_id, accounts, payload),
        Ix::CloseMarket => close_market::process(program_id, accounts, payload),
        Ix::SweepOracle => sweep_oracle::process(program_id, accounts, payload),
        Ix::WriteOracleMeta => write_oracle_meta::process(program_id, accounts, payload),
    }
}
