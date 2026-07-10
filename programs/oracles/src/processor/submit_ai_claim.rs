//! `submit_ai_claim`: a locked-in proposer reruns the pinned model over the
//! agreed facts and submits a categorical claim with reproducibility metadata.
//!
//! Creates a per-proposer [`AiClaim`] PDA and records the proposer's chosen
//! `claim_option`. No token CPI: bonds were escrowed at dispute start and no
//! stake moves here. A claim that differs from the proposer's `original_option`
//! marks them `flipped` (penalized later in `finalize_ai_claims`).
//!
//! # AiClaim PDA seeds (CONTRACT)
//! `[b"claim", oracle_pubkey, proposer_pubkey]`, program = [`crate::ID`].
//!
//! # Instruction payload (after the 1-byte discriminant)
//! `model_id: [u8; 32]` ++ `params_hash: [u8; 32]` ++ `io_hash: [u8; 32]` ++
//! `option: u8`. Exact length (97 bytes) required; trailing bytes are rejected.
//!
//! # Accounts
//! 0. oracle           — writable, owned by this program
//! 1. proposer PDA     — writable, owned by this program (the submitter's)
//! 2. ai_claim PDA     — writable, uninitialized (created here)
//! 3. authority        — signer, writable; must equal `proposer.authority` and
//!    funds the AiClaim rent
//! 4. system program

use bytemuck::Zeroable;
use pinocchio::{
    account::AccountView as AccountInfo, address::Address as Pubkey, cpi::Seed,
    error::ProgramError, ProgramResult,
};

use crate::{
    clock::{now, require_before_end, require_phase},
    error::KassandraError,
    processor::guards::{assert_key, assert_signer, create_pda, load_oracle, load_proposer},
    rent::minimum_rent,
    state::{AccountType, AiClaim, Oracle, Phase},
};

/// Exact payload length: model_id[32] ++ params_hash[32] ++ io_hash[32] ++ option[1].
const PAYLOAD_LEN: usize = 32 + 32 + 32 + 1;

/// Parsed `submit_ai_claim` payload borrowing from the instruction data.
struct Args<'a> {
    model_id: &'a [u8; 32],
    params_hash: &'a [u8; 32],
    io_hash: &'a [u8; 32],
    option: u8,
}

impl<'a> Args<'a> {
    fn parse(payload: &'a [u8]) -> Result<Self, ProgramError> {
        // Exact length — no short reads, no trailing bytes.
        if payload.len() != PAYLOAD_LEN {
            return Err(ProgramError::InvalidInstructionData);
        }
        let model_id: &[u8; 32] = payload[0..32].try_into().unwrap();
        let params_hash: &[u8; 32] = payload[32..64].try_into().unwrap();
        let io_hash: &[u8; 32] = payload[64..96].try_into().unwrap();
        let option = payload[96];
        Ok(Self {
            model_id,
            params_hash,
            io_hash,
            option,
        })
    }
}

pub fn process(program_id: &Pubkey, accounts: &mut [AccountInfo], payload: &[u8]) -> ProgramResult {
    let args = Args::parse(payload)?;

    let [oracle_ai, proposer_ai, claim_ai, authority_ai, system_prog_ai, ..] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };

    // --- account validation -------------------------------------------------
    assert_signer(authority_ai)?;
    assert_key(system_prog_ai, &pinocchio_system::ID)?;

    // Owner + size + account_type check, then owned copies for later mutation.
    let oracle: Oracle = load_oracle(oracle_ai, program_id)?;
    let mut proposer = load_proposer(proposer_ai, program_id)?;

    // --- phase / window gates (phase-first convention) ----------------------
    // A wrong-phase / closed-window tx must surface WrongPhase / WindowClosed,
    // not a semantic error, so these come before the binding checks below.
    require_phase(&oracle, Phase::AiClaim)?;
    require_before_end(&oracle, now()?)?;

    // The proposer must belong to this oracle and be controlled by the signer.
    if proposer.oracle != *oracle_ai.address() {
        return Err(KassandraError::InvalidAccount.into());
    }
    if proposer.authority != *authority_ai.address() {
        return Err(KassandraError::Unauthorized.into());
    }
    // A disqualified proposer has no standing to submit a claim.
    if proposer.is_disqualified() {
        return Err(KassandraError::Unauthorized.into());
    }

    // The claimed option must index a real categorical option.
    if args.option >= oracle.options_count {
        return Err(KassandraError::InvalidOption.into());
    }

    // --- ai_claim PDA derivation + duplicate rejection ----------------------
    let (expected_claim, bump) = Pubkey::find_program_address(
        &[
            b"claim",
            oracle_ai.address().as_ref(),
            proposer_ai.address().as_ref(),
        ],
        program_id,
    );
    assert_key(claim_ai, &expected_claim)?;
    // An already-funded PDA means this proposer already submitted a claim.
    if claim_ai.lamports() != 0 || !claim_ai.is_data_empty() {
        return Err(KassandraError::DuplicateClaim.into());
    }

    // --- create the AiClaim account (program-signed) ------------------------
    let rent = minimum_rent(AiClaim::LEN)?;
    let bump_seed = [bump];
    let signer_seeds = [
        Seed::from(b"claim".as_ref()),
        Seed::from(oracle_ai.address().as_ref()),
        Seed::from(proposer_ai.address().as_ref()),
        Seed::from(&bump_seed),
    ];
    create_pda(
        authority_ai,
        claim_ai,
        &signer_seeds,
        rent,
        AiClaim::LEN,
        program_id,
    )?;

    // --- initialize the AiClaim ---------------------------------------------
    let mut claim = AiClaim::zeroed();
    claim.account_type = AccountType::AiClaim.as_u8();
    claim.oracle = *oracle_ai.address();
    claim.proposer = *proposer_ai.address();
    claim.model_id = *args.model_id;
    claim.params_hash = *args.params_hash;
    claim.io_hash = *args.io_hash;
    claim.option = args.option;
    claim.challenged = 0;
    claim.bump = bump;
    // Record the proposer's human authority (== proposer.authority, asserted
    // above) so close_ai_claim can reclaim rent to it without the Proposer.
    claim.authority = *authority_ai.address();
    {
        let mut data = claim_ai.try_borrow_mut()?;
        data.copy_from_slice(bytemuck::bytes_of(&claim));
    }

    // --- record the claim on the proposer -----------------------------------
    proposer.claim_option = args.option;
    if args.option != proposer.original_option {
        proposer.flipped = 1;
    }
    {
        let mut data = proposer_ai.try_borrow_mut()?;
        data[..crate::state::Proposer::LEN].copy_from_slice(bytemuck::bytes_of(&proposer));
    }

    Ok(())
}
