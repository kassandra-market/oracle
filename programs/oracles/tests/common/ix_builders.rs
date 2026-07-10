use super::*;

/// Hand-build a futarchy `Dao` account blob with a `PoolState::Spot` embedded
/// spot `Pool` whose `TwapOracle` carries the given fields at the F0-documented
/// fixed offsets (mirrors `tests/kass_price.rs`). Used to give `open_challenge`
/// a deterministic `kass_price`.
pub fn build_dao_blob(
    aggregator: u128,
    last_updated: i64,
    created_at: i64,
    start_delay: u32,
) -> Vec<u8> {
    let mut data = vec![0u8; md6::DAO_SPOT_TWAP_MIN_LEN];
    data[0..8].copy_from_slice(&md6::DAO_ACCOUNT_DISCRIMINATOR);
    data[md6::DAO_POOLSTATE_TAG_OFFSET] = 0; // PoolState::Spot
    data[md6::DAO_SPOT_AGGREGATOR_OFFSET..md6::DAO_SPOT_AGGREGATOR_OFFSET + 16]
        .copy_from_slice(&aggregator.to_le_bytes());
    data[md6::DAO_SPOT_LAST_UPDATED_TS_OFFSET..md6::DAO_SPOT_LAST_UPDATED_TS_OFFSET + 8]
        .copy_from_slice(&last_updated.to_le_bytes());
    data[md6::DAO_SPOT_CREATED_AT_TS_OFFSET..md6::DAO_SPOT_CREATED_AT_TS_OFFSET + 8]
        .copy_from_slice(&created_at.to_le_bytes());
    data[md6::DAO_SPOT_START_DELAY_SECONDS_OFFSET..md6::DAO_SPOT_START_DELAY_SECONDS_OFFSET + 4]
        .copy_from_slice(&start_delay.to_le_bytes());
    data
}

// ---------------------------------------------------------------------------
// Shared dispute-core instruction builders.
//
// These are the raw-encoding builders for the dispute-core instructions
// (submit_fact / advance_phase / vote_fact / submit_ai_claim /
// finalize_ai_claims). They were previously copy-pasted, byte-for-byte, into
// ~8 separate integration-test files; hoisted here so every test shares one
// definition. Kept as an INDEPENDENT hand-encoding (not a wrapper over the Rust
// SDK) so the tests double as a cross-check that the on-chain layout matches.
// ---------------------------------------------------------------------------

/// Encode a `submit_fact` payload: `disc ++ content_hash[32] ++ stake_le[8] ++
/// uri_len_le[2] ++ uri`.
pub fn submit_fact_payload(content_hash: &[u8; 32], stake: u64, uri: &[u8]) -> Vec<u8> {
    let mut data = Vec::with_capacity(1 + 32 + 8 + 2 + uri.len());
    data.push(Ix::SubmitFact as u8);
    data.extend_from_slice(content_hash);
    data.extend_from_slice(&stake.to_le_bytes());
    data.extend_from_slice(&(uri.len() as u16).to_le_bytes());
    data.extend_from_slice(uri);
    data
}

/// Build a `submit_fact` instruction with the locked-in account order.
pub fn submit_fact_ix(
    ctx: &TestCtx,
    oracle: Pubkey,
    fact: Pubkey,
    submitter: Pubkey,
    submitter_kass: Pubkey,
    vault: Pubkey,
    data: Vec<u8>,
) -> Instruction {
    Instruction {
        program_id: ctx.program_id,
        accounts: vec![
            AccountMeta::new(oracle, false),
            AccountMeta::new(fact, false),
            AccountMeta::new(submitter, true),
            AccountMeta::new(submitter_kass, false),
            AccountMeta::new(vault, false),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            AccountMeta::new_readonly(system_program::id(), false),
        ],
        data,
    }
}

/// Build an `advance_phase` instruction (single oracle account).
pub fn advance_phase_ix(ctx: &TestCtx, oracle: Pubkey) -> Instruction {
    Instruction {
        program_id: ctx.program_id,
        accounts: vec![AccountMeta::new(oracle, false)],
        data: vec![Ix::AdvancePhase as u8],
    }
}

/// Encode a `vote_fact` payload: `disc ++ kind[1] ++ stake_le[8]`.
pub fn vote_payload(kind: u8, stake: u64) -> Vec<u8> {
    let mut data = Vec::with_capacity(1 + 1 + 8);
    data.push(Ix::VoteFact as u8);
    data.push(kind);
    data.extend_from_slice(&stake.to_le_bytes());
    data
}

/// Build a `vote_fact` instruction with the locked-in account order.
#[allow(clippy::too_many_arguments)]
pub fn vote_fact_ix(
    ctx: &TestCtx,
    oracle: Pubkey,
    fact: Pubkey,
    fact_vote: Pubkey,
    voter: Pubkey,
    voter_kass: Pubkey,
    vault: Pubkey,
    data: Vec<u8>,
) -> Instruction {
    Instruction {
        program_id: ctx.program_id,
        accounts: vec![
            AccountMeta::new(oracle, false),
            AccountMeta::new(fact, false),
            AccountMeta::new(fact_vote, false),
            AccountMeta::new(voter, true),
            AccountMeta::new(voter_kass, false),
            AccountMeta::new(vault, false),
            AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
            AccountMeta::new_readonly(system_program::id(), false),
        ],
        data,
    }
}

/// Encode a `submit_ai_claim` payload with fixed test hashes (model 0xAA,
/// params 0xBB, io 0xCC) + the chosen `option`.
pub fn submit_ai_payload(option: u8) -> Vec<u8> {
    let mut data = Vec::with_capacity(1 + 32 + 32 + 32 + 1);
    data.push(Ix::SubmitAiClaim as u8);
    data.extend_from_slice(&[0xAA; 32]); // model_id
    data.extend_from_slice(&[0xBB; 32]); // params_hash
    data.extend_from_slice(&[0xCC; 32]); // io_hash
    data.push(option);
    data
}

/// Build a `submit_ai_claim` instruction with the locked-in account order.
pub fn submit_ai_claim_ix(
    ctx: &TestCtx,
    oracle: Pubkey,
    proposer: Pubkey,
    claim: Pubkey,
    authority: Pubkey,
    data: Vec<u8>,
) -> Instruction {
    Instruction {
        program_id: ctx.program_id,
        accounts: vec![
            AccountMeta::new(oracle, false),
            AccountMeta::new(proposer, false),
            AccountMeta::new(claim, false),
            AccountMeta::new(authority, true),
            AccountMeta::new_readonly(system_program::id(), false),
        ],
        data,
    }
}

/// Build a `finalize_ai_claims` instruction (oracle + the proposer-set tail).
pub fn finalize_ai_claims_ix(ctx: &TestCtx, oracle: Pubkey, tail: &[Pubkey]) -> Instruction {
    let mut accounts = Vec::with_capacity(1 + tail.len());
    accounts.push(AccountMeta::new(oracle, false));
    for k in tail {
        accounts.push(AccountMeta::new(*k, false));
    }
    Instruction {
        program_id: ctx.program_id,
        accounts,
        data: vec![Ix::FinalizeAiClaims as u8],
    }
}
