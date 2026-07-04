//! The Kassandra instruction decoder for Carbon: filter to the program, read the
//! `Ix` discriminant, and carry the accounts + raw data.

use std::str::FromStr;

use carbon_core::instruction::InstructionDecoder;
use solana_instruction::Instruction;
use solana_pubkey::Pubkey;

/// The Kassandra program id.
pub const PROGRAM_ID_STR: &str = "KassVxvXUEPr5apSr2MqiGva4VFtJXyYLLDFS3f83nY";

pub fn program_id() -> Pubkey {
    Pubkey::from_str(PROGRAM_ID_STR).expect("valid program id")
}

/// discriminant → instruction name (mirrors `kassandra_program::instruction::Ix`).
pub fn ix_name(discriminant: u8) -> &'static str {
    match discriminant {
        0 => "submit_fact",
        1 => "vote_fact",
        2 => "finalize_facts",
        3 => "submit_ai_claim",
        4 => "open_challenge",
        5 => "settle_challenge",
        6 => "finalize_oracle",
        7 => "advance_phase",
        8 => "finalize_ai_claims",
        9 => "init_protocol",
        10 => "create_oracle",
        11 => "propose",
        12 => "finalize_proposals",
        13 => "set_governance",
        14 => "set_config",
        15 => "resolve_deadend",
        16 => "kass_price",
        17 => "claim_proposer",
        18 => "claim_fact",
        19 => "claim_fact_vote",
        20 => "close_ai_claim",
        21 => "close_market",
        22 => "sweep_oracle",
        _ => "unknown",
    }
}

/// A decoded Kassandra instruction.
#[derive(Debug)]
pub struct KassandraIx {
    pub discriminant: u8,
    pub name: &'static str,
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
            accounts: instruction.accounts.iter().map(|a| a.pubkey.to_string()).collect(),
            data: instruction.data.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_instruction::AccountMeta;

    #[test]
    fn maps_discriminants_to_names() {
        assert_eq!(ix_name(11), "propose");
        assert_eq!(ix_name(4), "open_challenge");
        assert_eq!(ix_name(22), "sweep_oracle");
        assert_eq!(ix_name(200), "unknown");
    }

    #[test]
    fn decodes_only_kassandra_instructions() {
        let dec = KassandraDecoder { program_id: program_id() };
        let oracle = Pubkey::new_unique();

        // A propose (disc 11) instruction for the Kassandra program.
        let ours = Instruction {
            program_id: program_id(),
            accounts: vec![AccountMeta::new(oracle, false)],
            data: vec![11, 0xAB, 0xCD],
        };
        let decoded = dec.decode_instruction(&ours).expect("kassandra ix decodes");
        assert_eq!(decoded.discriminant, 11);
        assert_eq!(decoded.name, "propose");
        assert_eq!(decoded.accounts, vec![oracle.to_string()]);

        // A different program's instruction is skipped.
        let other = Instruction {
            program_id: Pubkey::new_unique(),
            accounts: vec![],
            data: vec![11],
        };
        assert!(dec.decode_instruction(&other).is_none());
    }
}
