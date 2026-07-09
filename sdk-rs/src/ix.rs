//! Instruction builders — one per [`Ix`] variant, returning a client
//! [`Instruction`]. Account order, signer/writable flags, and payload byte
//! layouts are the program's wire contract (see the processors in
//! `programs/kassandra/src/processor/`). Every oracle-PDA-signing instruction
//! needs the oracle `nonce`, which the Oracle struct does not store — callers
//! must carry it alongside the oracle pubkey.

use kassandra_program::instruction::Ix;
use solana_instruction::{AccountMeta, Instruction};
use solana_pubkey::Pubkey;

use crate::{SYSTEM_PROGRAM_ID, TOKEN_PROGRAM_ID};

#[inline]
fn build(program_id: &Pubkey, accounts: Vec<AccountMeta>, data: Vec<u8>) -> Instruction {
    Instruction {
        program_id: *program_id,
        accounts,
        data,
    }
}

// ===================================================================== Ix 0
/// `SubmitFact` (Ix 0) — post a candidate fact with a KASS stake.
#[allow(clippy::too_many_arguments)]
pub fn submit_fact(
    program_id: &Pubkey,
    oracle: Pubkey,
    fact: Pubkey,
    submitter: Pubkey,
    submitter_kass: Pubkey,
    stake_vault: Pubkey,
    content_hash: &[u8; 32],
    stake: u64,
    uri: &[u8],
) -> Instruction {
    let mut data = Vec::with_capacity(1 + 42 + uri.len());
    data.push(Ix::SubmitFact as u8);
    data.extend_from_slice(content_hash);
    data.extend_from_slice(&stake.to_le_bytes());
    data.extend_from_slice(&(uri.len() as u16).to_le_bytes());
    data.extend_from_slice(uri);
    build(
        program_id,
        vec![
            AccountMeta::new(oracle, false),
            AccountMeta::new(fact, false),
            AccountMeta::new(submitter, true),
            AccountMeta::new(submitter_kass, false),
            AccountMeta::new(stake_vault, false),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
        ],
        data,
    )
}

// ===================================================================== Ix 1
/// `VoteFact` (Ix 1) — approve (`kind = 0`) or mark-duplicate (`kind = 1`) a fact.
#[allow(clippy::too_many_arguments)]
pub fn vote_fact(
    program_id: &Pubkey,
    oracle: Pubkey,
    fact: Pubkey,
    fact_vote: Pubkey,
    voter: Pubkey,
    voter_kass: Pubkey,
    stake_vault: Pubkey,
    kind: u8,
    stake: u64,
) -> Instruction {
    let mut data = Vec::with_capacity(1 + 9);
    data.push(Ix::VoteFact as u8);
    data.push(kind);
    data.extend_from_slice(&stake.to_le_bytes());
    build(
        program_id,
        vec![
            AccountMeta::new(oracle, false),
            AccountMeta::new(fact, false),
            AccountMeta::new(fact_vote, false),
            AccountMeta::new(voter, true),
            AccountMeta::new(voter_kass, false),
            AccountMeta::new(stake_vault, false),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
        ],
        data,
    )
}

// ===================================================================== Ix 2
/// `FinalizeFacts` (Ix 2) — incrementally settle the fact-voting phase. `tail`
/// is a non-empty writable subset of Facts (normal) or Proposers (no-facts
/// dead-end). Head burns via the oracle PDA (needs `nonce`).
pub fn finalize_facts(
    program_id: &Pubkey,
    oracle: Pubkey,
    kass_mint: Pubkey,
    stake_vault: Pubkey,
    nonce: u64,
    tail: &[Pubkey],
) -> Instruction {
    let mut data = Vec::with_capacity(1 + 8);
    data.push(Ix::FinalizeFacts as u8);
    data.extend_from_slice(&nonce.to_le_bytes());
    let mut accounts = Vec::with_capacity(4 + tail.len());
    accounts.push(AccountMeta::new(oracle, false));
    accounts.push(AccountMeta::new(kass_mint, false));
    accounts.push(AccountMeta::new(stake_vault, false));
    accounts.push(AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false));
    for k in tail {
        accounts.push(AccountMeta::new(*k, false));
    }
    build(program_id, accounts, data)
}

// ===================================================================== Ix 3
/// `SubmitAiClaim` (Ix 3) — resubmit a value + AI-claim metadata over the agreed
/// facts. Assembles the 97-byte payload from its components.
#[allow(clippy::too_many_arguments)]
pub fn submit_ai_claim(
    program_id: &Pubkey,
    oracle: Pubkey,
    proposer: Pubkey,
    ai_claim: Pubkey,
    authority: Pubkey,
    model_id: &[u8; 32],
    params_hash: &[u8; 32],
    io_hash: &[u8; 32],
    option: u8,
) -> Instruction {
    let mut payload = [0u8; 97];
    payload[0..32].copy_from_slice(model_id);
    payload[32..64].copy_from_slice(params_hash);
    payload[64..96].copy_from_slice(io_hash);
    payload[96] = option;
    submit_ai_claim_raw(program_id, oracle, proposer, ai_claim, authority, &payload)
}

/// `SubmitAiClaim` (Ix 3) from a pre-computed 97-byte payload
/// (`model_id[32] ++ params_hash[32] ++ io_hash[32] ++ option[1]`). Used by the
/// runner, which passes the exact bytes it emitted as metadata so the submitted
/// claim can never diverge from the emitted claim.
pub fn submit_ai_claim_raw(
    program_id: &Pubkey,
    oracle: Pubkey,
    proposer: Pubkey,
    ai_claim: Pubkey,
    authority: Pubkey,
    payload: &[u8; 97],
) -> Instruction {
    let mut data = Vec::with_capacity(1 + 97);
    data.push(Ix::SubmitAiClaim as u8);
    data.extend_from_slice(payload);
    build(
        program_id,
        vec![
            AccountMeta::new(oracle, false),
            AccountMeta::new(proposer, false),
            AccountMeta::new(ai_claim, false),
            AccountMeta::new(authority, true),
            AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
        ],
        data,
    )
}

// ===================================================================== Ix 4
/// The 25 accounts `OpenChallenge` (Ix 4) requires, in wire order. The MetaDAO
/// slots (question, conditional vaults, AMMs, conditional-token mints, event
/// authority) are composed by the challenger beforehand and passed in.
#[derive(Clone, Copy, Debug)]
pub struct OpenChallengeAccounts {
    pub oracle: Pubkey,
    pub ai_claim: Pubkey,
    pub proposer: Pubkey,
    pub market: Pubkey,
    pub challenger: Pubkey,
    pub question: Pubkey,
    pub kass_vault: Pubkey,
    pub usdc_vault: Pubkey,
    pub pass_amm: Pubkey,
    pub fail_amm: Pubkey,
    pub stake_vault: Pubkey,
    pub kass_vault_underlying: Pubkey,
    pub pass_kass_mint: Pubkey,
    pub fail_kass_mint: Pubkey,
    pub oracle_pass_kass: Pubkey,
    pub oracle_fail_kass: Pubkey,
    pub cv_program: Pubkey,
    pub cv_event_authority: Pubkey,
    pub protocol: Pubkey,
    pub kass_dao: Pubkey,
    pub usdc_mint: Pubkey,
    pub challenger_usdc_src: Pubkey,
    pub challenger_usdc_vault: Pubkey,
}

/// `OpenChallenge` (Ix 4) — open a MetaDAO decision market against an AI claim.
/// Payload is the oracle `nonce` (split signer). Escrow size is computed on-chain.
pub fn open_challenge(program_id: &Pubkey, a: &OpenChallengeAccounts, nonce: u64) -> Instruction {
    let mut data = Vec::with_capacity(1 + 8);
    data.push(Ix::OpenChallenge as u8);
    data.extend_from_slice(&nonce.to_le_bytes());
    build(
        program_id,
        vec![
            AccountMeta::new(a.oracle, false),
            AccountMeta::new(a.ai_claim, false),
            AccountMeta::new(a.proposer, false),
            AccountMeta::new(a.market, false),
            AccountMeta::new(a.challenger, true),
            AccountMeta::new_readonly(a.question, false),
            AccountMeta::new(a.kass_vault, false),
            AccountMeta::new_readonly(a.usdc_vault, false),
            AccountMeta::new_readonly(a.pass_amm, false),
            AccountMeta::new_readonly(a.fail_amm, false),
            AccountMeta::new(a.stake_vault, false),
            AccountMeta::new(a.kass_vault_underlying, false),
            AccountMeta::new(a.pass_kass_mint, false),
            AccountMeta::new(a.fail_kass_mint, false),
            AccountMeta::new(a.oracle_pass_kass, false),
            AccountMeta::new(a.oracle_fail_kass, false),
            AccountMeta::new_readonly(a.cv_program, false),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
            AccountMeta::new_readonly(a.cv_event_authority, false),
            AccountMeta::new_readonly(a.protocol, false),
            AccountMeta::new_readonly(a.kass_dao, false),
            AccountMeta::new_readonly(a.usdc_mint, false),
            AccountMeta::new(a.challenger_usdc_src, false),
            AccountMeta::new(a.challenger_usdc_vault, false),
        ],
        data,
    )
}

// ===================================================================== Ix 5
/// The 21 accounts `SettleChallenge` (Ix 5) requires, in wire order.
#[derive(Clone, Copy, Debug)]
pub struct SettleChallengeAccounts {
    pub oracle: Pubkey,
    pub market: Pubkey,
    pub ai_claim: Pubkey,
    pub proposer: Pubkey,
    pub question: Pubkey,
    pub pass_amm: Pubkey,
    pub fail_amm: Pubkey,
    pub cv_program: Pubkey,
    pub cv_event_authority: Pubkey,
    pub stake_vault: Pubkey,
    pub kass_vault: Pubkey,
    pub kass_vault_underlying: Pubkey,
    pub pass_kass_mint: Pubkey,
    pub fail_kass_mint: Pubkey,
    pub oracle_pass_kass: Pubkey,
    pub oracle_fail_kass: Pubkey,
    pub challenger_usdc_vault: Pubkey,
    pub proposer_usdc: Pubkey,
    pub challenger_usdc_dest: Pubkey,
    pub challenger_kass: Pubkey,
}

/// `SettleChallenge` (Ix 5) — read the market TWAP, apply the verdict, resolve
/// the question, redeem, and route directional fees. Payload is the oracle `nonce`.
pub fn settle_challenge(
    program_id: &Pubkey,
    a: &SettleChallengeAccounts,
    nonce: u64,
) -> Instruction {
    let mut data = Vec::with_capacity(1 + 8);
    data.push(Ix::SettleChallenge as u8);
    data.extend_from_slice(&nonce.to_le_bytes());
    build(
        program_id,
        vec![
            AccountMeta::new(a.oracle, false),
            AccountMeta::new(a.market, false),
            AccountMeta::new_readonly(a.ai_claim, false),
            AccountMeta::new(a.proposer, false),
            AccountMeta::new(a.question, false),
            AccountMeta::new_readonly(a.pass_amm, false),
            AccountMeta::new_readonly(a.fail_amm, false),
            AccountMeta::new_readonly(a.cv_program, false),
            AccountMeta::new_readonly(a.cv_event_authority, false),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            AccountMeta::new(a.stake_vault, false),
            AccountMeta::new(a.kass_vault, false),
            AccountMeta::new(a.kass_vault_underlying, false),
            AccountMeta::new(a.pass_kass_mint, false),
            AccountMeta::new(a.fail_kass_mint, false),
            AccountMeta::new(a.oracle_pass_kass, false),
            AccountMeta::new(a.oracle_fail_kass, false),
            AccountMeta::new(a.challenger_usdc_vault, false),
            AccountMeta::new(a.proposer_usdc, false),
            AccountMeta::new(a.challenger_usdc_dest, false),
            AccountMeta::new(a.challenger_kass, false),
        ],
        data,
    )
}

// ===================================================================== Ix 6
/// `FinalizeOracle` (Ix 6) — compute the final plurality. `tail` must be EXACTLY
/// `oracle.proposer_count` read-only Proposer accounts (one-shot).
pub fn finalize_oracle(
    program_id: &Pubkey,
    oracle: Pubkey,
    kass_mint: Pubkey,
    stake_vault: Pubkey,
    nonce: u64,
    tail: &[Pubkey],
) -> Instruction {
    let mut data = Vec::with_capacity(1 + 8);
    data.push(Ix::FinalizeOracle as u8);
    data.extend_from_slice(&nonce.to_le_bytes());
    let mut accounts = Vec::with_capacity(4 + tail.len());
    accounts.push(AccountMeta::new(oracle, false));
    accounts.push(AccountMeta::new(kass_mint, false));
    accounts.push(AccountMeta::new(stake_vault, false));
    accounts.push(AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false));
    for k in tail {
        accounts.push(AccountMeta::new_readonly(*k, false));
    }
    build(program_id, accounts, data)
}

// ===================================================================== Ix 7
/// `AdvancePhase` (Ix 7) — permissionless `FactProposal -> FactVoting` freeze.
pub fn advance_phase(program_id: &Pubkey, oracle: Pubkey) -> Instruction {
    build(
        program_id,
        vec![AccountMeta::new(oracle, false)],
        vec![Ix::AdvancePhase as u8],
    )
}

// ===================================================================== Ix 8
/// `FinalizeAiClaims` (Ix 8) — incrementally settle the AI-claim round. `tail`
/// is a non-empty writable subset of this oracle's Proposers.
pub fn finalize_ai_claims(program_id: &Pubkey, oracle: Pubkey, tail: &[Pubkey]) -> Instruction {
    let mut accounts = Vec::with_capacity(1 + tail.len());
    accounts.push(AccountMeta::new(oracle, false));
    for k in tail {
        accounts.push(AccountMeta::new(*k, false));
    }
    build(program_id, accounts, vec![Ix::FinalizeAiClaims as u8])
}

// ===================================================================== Ix 9
/// `InitProtocol` (Ix 9) — create the `[b"protocol"]` singleton.
pub fn init_protocol(
    program_id: &Pubkey,
    protocol: Pubkey,
    admin: Pubkey,
    kass_mint: Pubkey,
    usdc_mint: Pubkey,
) -> Instruction {
    build(
        program_id,
        vec![
            AccountMeta::new(protocol, false),
            AccountMeta::new(admin, true),
            AccountMeta::new_readonly(kass_mint, false),
            AccountMeta::new_readonly(usdc_mint, false),
            AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
        ],
        vec![Ix::InitProtocol as u8],
    )
}

// ===================================================================== Ix 10
/// `CreateOracle` (Ix 10). Derives the protocol, stake-vault, and mint-authority
/// PDAs internally. Payload order is nonce, options_count, deadline, twap_window
/// (NOT the account order). The subject now lives on-chain in `oracle_meta`.
#[allow(clippy::too_many_arguments)]
pub fn create_oracle(
    program_id: &Pubkey,
    nonce: u64,
    options_count: u8,
    deadline: i64,
    twap_window: i64,
    oracle: Pubkey,
    kass_mint: Pubkey,
    usdc_mint: Pubkey,
    creator: Pubkey,
    creator_kass: Pubkey,
) -> Instruction {
    let (protocol, _) = crate::pda::protocol(program_id);
    let (stake_vault, _) = crate::pda::stake_vault(program_id, &oracle);
    let (mint_authority, _) = crate::pda::mint_authority(program_id);

    let mut data = Vec::with_capacity(1 + 25);
    data.push(Ix::CreateOracle as u8);
    data.extend_from_slice(&nonce.to_le_bytes());
    data.push(options_count);
    data.extend_from_slice(&deadline.to_le_bytes());
    data.extend_from_slice(&twap_window.to_le_bytes());

    build(
        program_id,
        vec![
            AccountMeta::new(protocol, false),
            AccountMeta::new(oracle, false),
            AccountMeta::new(stake_vault, false),
            AccountMeta::new(creator, true),
            AccountMeta::new(kass_mint, false),
            AccountMeta::new_readonly(usdc_mint, false),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
            AccountMeta::new(creator_kass, false),
            AccountMeta::new_readonly(mint_authority, false),
        ],
        data,
    )
}

// ===================================================================== Ix 11
/// `Propose` (Ix 11) — register a categorical `option` with a KASS `bond`.
#[allow(clippy::too_many_arguments)]
pub fn propose(
    program_id: &Pubkey,
    oracle: Pubkey,
    proposer: Pubkey,
    authority: Pubkey,
    authority_kass: Pubkey,
    stake_vault: Pubkey,
    option: u8,
    bond: u64,
) -> Instruction {
    let mut data = Vec::with_capacity(1 + 9);
    data.push(Ix::Propose as u8);
    data.push(option);
    data.extend_from_slice(&bond.to_le_bytes());
    build(
        program_id,
        vec![
            AccountMeta::new(oracle, false),
            AccountMeta::new(proposer, false),
            AccountMeta::new(authority, true),
            AccountMeta::new(authority_kass, false),
            AccountMeta::new(stake_vault, false),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
        ],
        data,
    )
}

// ===================================================================== Ix 12
/// `FinalizeProposals` (Ix 12) — close the proposal window. `tail` must be
/// EXACTLY `oracle.proposer_count` read-only Proposer accounts.
pub fn finalize_proposals(program_id: &Pubkey, oracle: Pubkey, tail: &[Pubkey]) -> Instruction {
    let mut accounts = Vec::with_capacity(1 + tail.len());
    accounts.push(AccountMeta::new(oracle, false));
    for p in tail {
        accounts.push(AccountMeta::new_readonly(*p, false));
    }
    build(program_id, accounts, vec![Ix::FinalizeProposals as u8])
}

// ===================================================================== Ix 13
/// `SetGovernance` (Ix 13) — record `dao_authority` + `kass_dao`. The `kass_dao`
/// account must equal the payload `kass_dao`.
pub fn set_governance(
    program_id: &Pubkey,
    protocol: Pubkey,
    authority: Pubkey,
    dao_authority: Pubkey,
    kass_dao: Pubkey,
) -> Instruction {
    let mut data = Vec::with_capacity(1 + 64);
    data.push(Ix::SetGovernance as u8);
    data.extend_from_slice(&dao_authority.to_bytes());
    data.extend_from_slice(&kass_dao.to_bytes());
    build(
        program_id,
        vec![
            AccountMeta::new(protocol, false),
            AccountMeta::new_readonly(authority, true),
            AccountMeta::new_readonly(kass_dao, false),
        ],
        data,
    )
}

// ===================================================================== Ix 14
/// `SetConfig` (Ix 14) — overwrite the governable params. Payload = the 200-byte
/// packed [`crate::ConfigParams`].
pub fn set_config(
    program_id: &Pubkey,
    protocol: Pubkey,
    dao_authority: Pubkey,
    params: &crate::ConfigParams,
) -> Instruction {
    let mut data = Vec::with_capacity(1 + 200);
    data.push(Ix::SetConfig as u8);
    data.extend_from_slice(&params.to_payload());
    build(
        program_id,
        vec![
            AccountMeta::new(protocol, false),
            AccountMeta::new_readonly(dao_authority, true),
        ],
        data,
    )
}

// ===================================================================== Ix 15
/// `ResolveDeadend` (Ix 15) — DAO-gated resolution of a dead-ended oracle.
pub fn resolve_deadend(
    program_id: &Pubkey,
    protocol: Pubkey,
    oracle: Pubkey,
    dao_authority: Pubkey,
    option: u8,
) -> Instruction {
    build(
        program_id,
        vec![
            AccountMeta::new_readonly(protocol, false),
            AccountMeta::new(oracle, false),
            AccountMeta::new_readonly(dao_authority, true),
        ],
        vec![Ix::ResolveDeadend as u8, option],
    )
}

// ===================================================================== Ix 16
/// `KassPrice` (Ix 16) — read the governance-anchored KASS/USDC spot TWAP.
pub fn kass_price(program_id: &Pubkey, protocol: Pubkey, kass_dao: Pubkey) -> Instruction {
    build(
        program_id,
        vec![
            AccountMeta::new_readonly(protocol, false),
            AccountMeta::new_readonly(kass_dao, false),
        ],
        vec![Ix::KassPrice as u8],
    )
}

// ===================================================================== Ix 17
/// `ClaimProposer` (Ix 17) — claim-and-close one proposer after the oracle is terminal.
pub fn claim_proposer(
    program_id: &Pubkey,
    oracle: Pubkey,
    nonce: u64,
    proposer: Pubkey,
    dest_kass: Pubkey,
    stake_vault: Pubkey,
    rent_recipient: Pubkey,
) -> Instruction {
    let mut data = Vec::with_capacity(1 + 8);
    data.push(Ix::ClaimProposer as u8);
    data.extend_from_slice(&nonce.to_le_bytes());
    build(
        program_id,
        vec![
            AccountMeta::new_readonly(oracle, false),
            AccountMeta::new(proposer, false),
            AccountMeta::new(dest_kass, false),
            AccountMeta::new(stake_vault, false),
            AccountMeta::new(rent_recipient, false),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
        ],
        data,
    )
}

// ===================================================================== Ix 18
/// `ClaimFact` (Ix 18) — claim-and-close one fact submitter.
pub fn claim_fact(
    program_id: &Pubkey,
    oracle: Pubkey,
    nonce: u64,
    fact: Pubkey,
    dest_kass: Pubkey,
    stake_vault: Pubkey,
    rent_recipient: Pubkey,
) -> Instruction {
    let mut data = Vec::with_capacity(1 + 8);
    data.push(Ix::ClaimFact as u8);
    data.extend_from_slice(&nonce.to_le_bytes());
    build(
        program_id,
        vec![
            AccountMeta::new_readonly(oracle, false),
            AccountMeta::new(fact, false),
            AccountMeta::new(dest_kass, false),
            AccountMeta::new(stake_vault, false),
            AccountMeta::new(rent_recipient, false),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
        ],
        data,
    )
}

// ===================================================================== Ix 19
/// `ClaimFactVote` (Ix 19) — claim-and-close one fact vote. `fact` (index 2) is
/// writable: its running voter-stake total is decremented (the fact is NOT closed).
#[allow(clippy::too_many_arguments)]
pub fn claim_fact_vote(
    program_id: &Pubkey,
    oracle: Pubkey,
    nonce: u64,
    fact_vote: Pubkey,
    fact: Pubkey,
    dest_kass: Pubkey,
    stake_vault: Pubkey,
    rent_recipient: Pubkey,
) -> Instruction {
    let mut data = Vec::with_capacity(1 + 8);
    data.push(Ix::ClaimFactVote as u8);
    data.extend_from_slice(&nonce.to_le_bytes());
    build(
        program_id,
        vec![
            AccountMeta::new_readonly(oracle, false),
            AccountMeta::new(fact_vote, false),
            AccountMeta::new(fact, false),
            AccountMeta::new(dest_kass, false),
            AccountMeta::new(stake_vault, false),
            AccountMeta::new(rent_recipient, false),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
        ],
        data,
    )
}

// ===================================================================== Ix 20
/// `CloseAiClaim` (Ix 20) — rent-reclaim close of an `AiClaim`. Empty payload.
pub fn close_ai_claim(
    program_id: &Pubkey,
    oracle: Pubkey,
    ai_claim: Pubkey,
    rent_recipient: Pubkey,
) -> Instruction {
    build(
        program_id,
        vec![
            AccountMeta::new_readonly(oracle, false),
            AccountMeta::new(ai_claim, false),
            AccountMeta::new(rent_recipient, false),
        ],
        vec![Ix::CloseAiClaim as u8],
    )
}

// ===================================================================== Ix 21
/// `CloseMarket` (Ix 21) — rent-reclaim close of a settled `Market` + its escrow.
pub fn close_market(
    program_id: &Pubkey,
    oracle: Pubkey,
    nonce: u64,
    market: Pubkey,
    challenger_usdc_vault: Pubkey,
    rent_recipient: Pubkey,
) -> Instruction {
    let mut data = Vec::with_capacity(1 + 8);
    data.push(Ix::CloseMarket as u8);
    data.extend_from_slice(&nonce.to_le_bytes());
    build(
        program_id,
        vec![
            AccountMeta::new_readonly(oracle, false),
            AccountMeta::new(market, false),
            AccountMeta::new(challenger_usdc_vault, false),
            AccountMeta::new(rent_recipient, false),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
        ],
        data,
    )
}

// ===================================================================== Ix 22
/// `SweepOracle` (Ix 22) — grace-gated dust sweep + terminal closure.
pub fn sweep_oracle(
    program_id: &Pubkey,
    oracle: Pubkey,
    nonce: u64,
    stake_vault: Pubkey,
    protocol: Pubkey,
    dao_treasury: Pubkey,
    creator: Pubkey,
    // The companion oracle_meta PDA, closed alongside the oracle (rent → creator).
    // `None` for an oracle that has no metadata (the close is skipped).
    oracle_meta: Option<Pubkey>,
) -> Instruction {
    let mut data = Vec::with_capacity(1 + 8);
    data.push(Ix::SweepOracle as u8);
    data.extend_from_slice(&nonce.to_le_bytes());
    let mut accounts = vec![
        AccountMeta::new(oracle, false),
        AccountMeta::new(stake_vault, false),
        AccountMeta::new_readonly(protocol, false),
        AccountMeta::new(dao_treasury, false),
        AccountMeta::new(creator, false),
        AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
    ];
    if let Some(meta) = oracle_meta {
        accounts.push(AccountMeta::new(meta, false));
    }
    build(program_id, accounts, data)
}

// ===================================================================== Ix 23
/// `WriteOracleMeta` (Ix 23). Writes the companion `[b"oracle_meta", oracle]`
/// PDA — the plaintext `subject` + option `labels` + `uri`/`uri_hash`. The body
/// is length-prefixed (`subject_len u16 ++ subject ++ options_count u8 ++
/// [option_len u16 ++ option]* ++ uri_len u16 ++ uri ++ uri_hash[32]`); the
/// account is sized to fit and is write-once, gated to the oracle's creator.
#[allow(clippy::too_many_arguments)]
pub fn write_oracle_meta(
    program_id: &Pubkey,
    oracle: Pubkey,
    creator: Pubkey,
    subject: &str,
    options: &[&str],
    uri: &str,
    uri_hash: &[u8; 32],
) -> Instruction {
    let (meta, _) = crate::pda::oracle_meta(program_id, &oracle);

    let mut data = Vec::new();
    data.push(Ix::WriteOracleMeta as u8);
    data.extend_from_slice(&(subject.len() as u16).to_le_bytes());
    data.extend_from_slice(subject.as_bytes());
    data.push(options.len() as u8);
    for o in options {
        data.extend_from_slice(&(o.len() as u16).to_le_bytes());
        data.extend_from_slice(o.as_bytes());
    }
    data.extend_from_slice(&(uri.len() as u16).to_le_bytes());
    data.extend_from_slice(uri.as_bytes());
    data.extend_from_slice(uri_hash);

    build(
        program_id,
        vec![
            AccountMeta::new(creator, true),
            AccountMeta::new_readonly(oracle, false),
            AccountMeta::new(meta, false),
            AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
        ],
        data,
    )
}
