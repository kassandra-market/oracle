//! Zero-copy decoders over the shared on-chain account structs.
//!
//! The layout structs live in the program crate ([`kassandra_program::state`])
//! and are `bytemuck`-castable, so decoding is a checked reference cast — no
//! parsing, no drift. Each decoder validates the length/alignment and returns a
//! borrow into the caller's buffer.

use bytemuck::PodCastError;

pub use kassandra_program::state::{
    AccountType, AiClaim, Fact, FactVote, Market, Oracle, Phase, Proposer, Protocol,
};

/// Decode a byte buffer (e.g. account data from RPC) as a reference to `T`.
/// Returns an error if the length or alignment does not match `T`.
pub fn decode<T: bytemuck::Pod>(data: &[u8]) -> Result<&T, PodCastError> {
    bytemuck::try_from_bytes::<T>(data)
}

macro_rules! decoder {
    ($name:ident, $ty:ty, $doc:literal) => {
        #[doc = $doc]
        pub fn $name(data: &[u8]) -> Result<&$ty, PodCastError> {
            decode::<$ty>(data)
        }
    };
}

decoder!(decode_protocol, Protocol, "Decode a `Protocol` singleton account.");
decoder!(decode_oracle, Oracle, "Decode an `Oracle` account.");
decoder!(decode_proposer, Proposer, "Decode a `Proposer` account.");
decoder!(decode_fact, Fact, "Decode a `Fact` account.");
decoder!(decode_fact_vote, FactVote, "Decode a `FactVote` account.");
decoder!(decode_ai_claim, AiClaim, "Decode an `AiClaim` account.");
decoder!(decode_market, Market, "Decode a `Market` account.");
