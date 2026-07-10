//! The Kassandra instruction decoder for Carbon: filter to the program, read the
//! `Ix` discriminant, and carry the accounts + raw data.
//!
//! The program id and the instruction discriminants are REUSED from the Rust SDK
//! (`kassandra_oracles_sdk`, whose single source of truth is the on-chain
//! `kassandra_program`), not re-declared here — so a change to the wire contract
//! (a renamed or renumbered instruction) propagates automatically.

use carbon_core::instruction::InstructionDecoder;
use kassandra_oracles_sdk::Ix;
use solana_instruction::Instruction;
use solana_pubkey::Pubkey;

/// Every `Ix` variant, listed once. Both the discriminant (`*ix as u8`) and the
/// snake_case name (from the variant's own `Debug`) are derived from the SDK enum
/// — there is no hand-written number→name table to drift from the program.
const IX_VARIANTS: [Ix; 24] = [
    Ix::SubmitFact,
    Ix::VoteFact,
    Ix::FinalizeFacts,
    Ix::SubmitAiClaim,
    Ix::OpenChallenge,
    Ix::SettleChallenge,
    Ix::FinalizeOracle,
    Ix::AdvancePhase,
    Ix::FinalizeAiClaims,
    Ix::InitProtocol,
    Ix::CreateOracle,
    Ix::Propose,
    Ix::FinalizeProposals,
    Ix::SetGovernance,
    Ix::SetConfig,
    Ix::ResolveDeadend,
    Ix::KassPrice,
    Ix::ClaimProposer,
    Ix::ClaimFact,
    Ix::ClaimFactVote,
    Ix::CloseAiClaim,
    Ix::CloseMarket,
    Ix::SweepOracle,
    Ix::WriteOracleMeta,
];

/// The Kassandra program id (from the SDK), as the `solana_pubkey::Pubkey` type
/// Carbon's `Instruction` uses.
pub fn program_id() -> Pubkey {
    Pubkey::new_from_array(kassandra_oracles_sdk::PROGRAM_ID.to_bytes())
}

/// The program id as a base58 string (for the API's `/status`).
pub fn program_id_str() -> String {
    program_id().to_string()
}

fn to_snake(pascal: &str) -> String {
    let mut out = String::with_capacity(pascal.len() + 4);
    for (i, c) in pascal.char_indices() {
        if c.is_ascii_uppercase() && i > 0 {
            out.push('_');
        }
        out.push(c.to_ascii_lowercase());
    }
    out
}

/// Human name for a discriminant (e.g. `11 → "propose"`), derived from the SDK
/// `Ix` enum; `"unknown"` for an unrecognized byte.
pub fn ix_name(discriminant: u8) -> String {
    IX_VARIANTS
        .iter()
        .find(|ix| **ix as u8 == discriminant)
        .map(|ix| to_snake(&format!("{ix:?}")))
        .unwrap_or_else(|| "unknown".to_string())
}

/// A decoded Kassandra instruction.
#[derive(Debug)]
pub struct KassandraIx {
    pub discriminant: u8,
    pub name: String,
    pub accounts: Vec<String>,
    pub data: Vec<u8>,
}

pub struct KassandraDecoder {
    pub program_id: Pubkey,
}

impl<'a> InstructionDecoder<'a> for KassandraDecoder {
    type InstructionType = KassandraIx;

    fn decode_instruction(&self, instruction: &'a Instruction) -> Option<Self::InstructionType> {
        if instruction.program_id != self.program_id {
            return None;
        }
        let discriminant = *instruction.data.first()?;
        Some(KassandraIx {
            discriminant,
            name: ix_name(discriminant),
            accounts: instruction
                .accounts
                .iter()
                .map(|a| a.pubkey.to_string())
                .collect(),
            data: instruction.data.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_instruction::AccountMeta;

    #[test]
    fn maps_discriminants_to_names_from_the_sdk_enum() {
        // Discriminants come from the SDK `Ix`, so these bind to the real values.
        assert_eq!(ix_name(Ix::Propose as u8), "propose");
        assert_eq!(ix_name(Ix::OpenChallenge as u8), "open_challenge");
        assert_eq!(ix_name(Ix::SweepOracle as u8), "sweep_oracle");
        assert_eq!(ix_name(Ix::SubmitAiClaim as u8), "submit_ai_claim");
        assert_eq!(ix_name(200), "unknown");
    }

    #[test]
    fn program_id_matches_the_sdk() {
        assert_eq!(
            program_id().to_bytes(),
            kassandra_oracles_sdk::PROGRAM_ID.to_bytes()
        );
    }

    #[test]
    fn decodes_only_kassandra_instructions() {
        let dec = KassandraDecoder {
            program_id: program_id(),
        };
        let oracle = Pubkey::new_unique();

        let ours = Instruction {
            program_id: program_id(),
            accounts: vec![AccountMeta::new(oracle, false)],
            data: vec![Ix::Propose as u8, 0xAB, 0xCD],
        };
        let decoded = dec.decode_instruction(&ours).expect("kassandra ix decodes");
        assert_eq!(decoded.discriminant, Ix::Propose as u8);
        assert_eq!(decoded.name, "propose");
        assert_eq!(decoded.accounts, vec![oracle.to_string()]);

        let other = Instruction {
            program_id: Pubkey::new_unique(),
            accounts: vec![],
            data: vec![Ix::Propose as u8],
        };
        assert!(dec.decode_instruction(&other).is_none());
    }
}
