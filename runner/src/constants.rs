//! On-chain protocol constants reused by the runner.
//!
//! These are RE-EXPORTED / DERIVED through the Rust SDK's account re-exports
//! (`kassandra_oracles_sdk::accounts`, whose source of truth is the on-chain program)
//! rather than mirrored, so there is no risk of the runner drifting from the
//! source of truth. The compile-time assertions below pin the `submit_ai_claim`
//! payload layout to the actual [`AiClaim`] account struct: if the program's
//! field order or widths ever change, this crate fails to build.

use core::mem::offset_of;
use kassandra_oracles_sdk::accounts::AiClaim;

/// `Proposer.claim_option` sentinel: no AI claim submitted yet. Re-exported
/// through the SDK so the runner and chain agree on the 0xFF "none" value.
pub use kassandra_oracles_sdk::accounts::CLAIM_OPTION_NONE;

/// Width of `model_id` in the `submit_ai_claim` payload (bytes).
pub const MODEL_ID_LEN: usize = 32;
/// Width of `params_hash` in the `submit_ai_claim` payload (bytes).
pub const PARAMS_HASH_LEN: usize = 32;
/// Width of `io_hash` in the `submit_ai_claim` payload (bytes).
pub const IO_HASH_LEN: usize = 32;
/// Width of the categorical `option` in the payload (bytes).
pub const OPTION_LEN: usize = 1;

/// Exact `submit_ai_claim` instruction payload length (after the 1-byte
/// discriminant): `model_id[32] ++ params_hash[32] ++ io_hash[32] ++ option[1]`.
/// The processor (`submit_ai_claim.rs`) requires EXACTLY this many bytes.
pub const SUBMIT_AI_CLAIM_PAYLOAD_LEN: usize =
    MODEL_ID_LEN + PARAMS_HASH_LEN + IO_HASH_LEN + OPTION_LEN;

// --- compile-time parity with the on-chain AiClaim layout -------------------
// The payload fields are a contiguous slice of the AiClaim struct
// (model_id, params_hash, io_hash, option). Tie our widths to the actual
// struct field offsets so a layout change in the program breaks this build
// rather than silently producing a wrong payload.
const _: () = assert!(SUBMIT_AI_CLAIM_PAYLOAD_LEN == 97);
const _: () =
    assert!(offset_of!(AiClaim, params_hash) - offset_of!(AiClaim, model_id) == MODEL_ID_LEN);
const _: () =
    assert!(offset_of!(AiClaim, io_hash) - offset_of!(AiClaim, params_hash) == PARAMS_HASH_LEN);
const _: () = assert!(offset_of!(AiClaim, option) - offset_of!(AiClaim, io_hash) == IO_HASH_LEN);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claim_option_none_is_0xff() {
        // The program's documented sentinel (state.rs).
        assert_eq!(CLAIM_OPTION_NONE, 0xFF);
    }

    #[test]
    fn payload_len_is_97() {
        assert_eq!(SUBMIT_AI_CLAIM_PAYLOAD_LEN, 97);
    }

    #[test]
    fn payload_field_widths_match_aiclaim() {
        assert_eq!(
            offset_of!(AiClaim, params_hash) - offset_of!(AiClaim, model_id),
            MODEL_ID_LEN
        );
        assert_eq!(
            offset_of!(AiClaim, io_hash) - offset_of!(AiClaim, params_hash),
            PARAMS_HASH_LEN
        );
        assert_eq!(
            offset_of!(AiClaim, option) - offset_of!(AiClaim, io_hash),
            IO_HASH_LEN
        );
    }
}
