//! Program-derived address helpers.
//!
//! These seed conventions are part of the program's public contract; downstream
//! code MUST derive with exactly these seeds. Each function returns
//! `(address, bump)` like [`solana_pubkey::Pubkey::find_program_address`].

use kassandra_oracles_program::config::MINT_AUTHORITY_SEED;
use solana_pubkey::Pubkey;

use crate::{ATA_PROGRAM_ID, TOKEN_PROGRAM_ID};

/// Oracle PDA — seeds `[b"oracle", nonce_le]`.
pub fn oracle(program_id: &Pubkey, nonce: u64) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"oracle", &nonce.to_le_bytes()], program_id)
}

/// Oracle-metadata PDA — seeds `[b"oracle_meta", oracle]`. Holds the plaintext
/// subject + option labels + uri/uri_hash written by `write_oracle_meta`.
pub fn oracle_meta(program_id: &Pubkey, oracle: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"oracle_meta", oracle.as_ref()], program_id)
}

/// Protocol singleton PDA — seeds `[b"protocol"]`.
pub fn protocol(program_id: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"protocol"], program_id)
}

/// KASS mint-authority PDA — seeds `[MINT_AUTHORITY_SEED]`. Handed to the KASS
/// mint so the program's emission `MintTo` can program-sign.
pub fn mint_authority(program_id: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[MINT_AUTHORITY_SEED], program_id)
}

/// Stake-vault PDA for an oracle — seeds `[b"vault", oracle]`. An SPL token
/// account on the KASS mint whose authority is the oracle PDA.
pub fn stake_vault(program_id: &Pubkey, oracle: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"vault", oracle.as_ref()], program_id)
}

/// Challenger USDC escrow-vault PDA for a market — seeds `[b"challenge_usdc", market]`.
pub fn challenge_usdc_vault(program_id: &Pubkey, market: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"challenge_usdc", market.as_ref()], program_id)
}

/// Proposer PDA — seeds `[b"proposer", oracle, authority]`.
pub fn proposer(program_id: &Pubkey, oracle: &Pubkey, authority: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[b"proposer", oracle.as_ref(), authority.as_ref()],
        program_id,
    )
}

/// Fact PDA — seeds `[b"fact", oracle, content_hash]`.
pub fn fact(program_id: &Pubkey, oracle: &Pubkey, content_hash: &[u8; 32]) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[b"fact", oracle.as_ref(), content_hash.as_ref()],
        program_id,
    )
}

/// FactVote PDA — seeds `[b"vote", fact, voter]`.
pub fn vote(program_id: &Pubkey, fact: &Pubkey, voter: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"vote", fact.as_ref(), voter.as_ref()], program_id)
}

/// AiClaim PDA — seeds `[b"claim", oracle, proposer]`.
pub fn ai_claim(program_id: &Pubkey, oracle: &Pubkey, proposer: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"claim", oracle.as_ref(), proposer.as_ref()], program_id)
}

/// The canonical KASS associated-token-account of `owner` — where the DAO
/// treasury lives. Derived under the ATA program from `[owner, token_program, mint]`.
pub fn kass_ata(owner: &Pubkey, kass_mint: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[
            owner.as_ref(),
            TOKEN_PROGRAM_ID.as_ref(),
            kass_mint.as_ref(),
        ],
        &ATA_PROGRAM_ID,
    )
    .0
}
