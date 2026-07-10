//! Zero-copy decoders over the shared on-chain account structs.
//!
//! The layout structs live in the program crate ([`kassandra_oracles_program::state`])
//! and are `bytemuck`-castable, so decoding is a checked reference cast — no
//! parsing, no drift. Each decoder validates the length/alignment and returns a
//! borrow into the caller's buffer.

use bytemuck::PodCastError;

pub use kassandra_oracles_program::state::{
    AccountType, AiClaim, Fact, FactVote, Market, Oracle, Phase, Proposer, Protocol,
    CLAIM_OPTION_NONE, VOTE_APPROVE, VOTE_DUPLICATE,
};

/// Decode a byte buffer as a reference to `T` (zero-copy). Requires the buffer to
/// be aligned to `T` and exactly `size_of::<T>()` bytes.
pub fn decode<T: bytemuck::Pod>(data: &[u8]) -> Result<&T, PodCastError> {
    bytemuck::try_from_bytes::<T>(data)
}

/// Decode a byte buffer as an owned `T` by copying — no alignment requirement, so
/// this suits RPC-fetched buffers (a `Vec<u8>` may be unaligned to `T`). Fails if
/// the length does not equal `size_of::<T>()`.
pub fn read<T: bytemuck::Pod>(data: &[u8]) -> Result<T, PodCastError> {
    bytemuck::try_pod_read_unaligned::<T>(data)
}

macro_rules! decoder {
    ($name:ident, $ty:ty, $doc:literal) => {
        #[doc = $doc]
        pub fn $name(data: &[u8]) -> Result<&$ty, PodCastError> {
            decode::<$ty>(data)
        }
    };
}

decoder!(
    decode_protocol,
    Protocol,
    "Decode a `Protocol` singleton account."
);
decoder!(decode_oracle, Oracle, "Decode an `Oracle` account.");
decoder!(decode_proposer, Proposer, "Decode a `Proposer` account.");
decoder!(decode_fact, Fact, "Decode a `Fact` account.");
decoder!(decode_fact_vote, FactVote, "Decode a `FactVote` account.");
decoder!(decode_ai_claim, AiClaim, "Decode an `AiClaim` account.");
decoder!(decode_market, Market, "Decode a `Market` account.");

/// The parsed `oracle_meta` account — the plaintext subject + option labels + the
/// off-chain-JSON `uri`/`uri_hash`. NOT a Pod struct (variable length), so it is
/// parsed from the length-prefixed layout written by `write_oracle_meta`:
/// `account_type u8 ++ bump u8 ++ oracle[32] ++ subject_len u16 ++ subject ++
/// options_count u8 ++ [option_len u16 ++ option]* ++ uri_len u16 ++ uri ++
/// uri_hash[32]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OracleMeta {
    pub oracle: [u8; 32],
    pub subject: String,
    pub options: Vec<String>,
    pub uri: String,
    pub uri_hash: [u8; 32],
}

/// Parse an `oracle_meta` account buffer. Returns `None` on any malformed/short
/// input (never panics), so an RPC-fetched buffer is safe to feed in directly.
pub fn decode_oracle_meta(data: &[u8]) -> Option<OracleMeta> {
    if data.first().copied()? != AccountType::OracleMeta.as_u8() || data.len() < 34 {
        return None;
    }
    let oracle: [u8; 32] = data[2..34].try_into().ok()?;
    let mut off = 34usize;
    let read_u16 = |d: &[u8], off: &mut usize| -> Option<usize> {
        let b = d.get(*off..*off + 2)?;
        *off += 2;
        Some(u16::from_le_bytes(b.try_into().unwrap()) as usize)
    };
    let read_str = |d: &[u8], off: &mut usize, len: usize| -> Option<String> {
        let s = d.get(*off..*off + len)?;
        *off += len;
        String::from_utf8(s.to_vec()).ok()
    };
    let subject_len = read_u16(data, &mut off)?;
    let subject = read_str(data, &mut off, subject_len)?;
    let options_count = *data.get(off)?;
    off += 1;
    let mut options = Vec::with_capacity(options_count as usize);
    for _ in 0..options_count {
        let ol = read_u16(data, &mut off)?;
        options.push(read_str(data, &mut off, ol)?);
    }
    let uri_len = read_u16(data, &mut off)?;
    let uri = read_str(data, &mut off, uri_len)?;
    let uri_hash: [u8; 32] = data.get(off..off + 32)?.try_into().ok()?;
    Some(OracleMeta {
        oracle,
        subject,
        options,
        uri,
        uri_hash,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oracle_meta_round_trip() {
        // Hand-encode the account body, then decode it back.
        let oracle = [9u8; 32];
        let mut d = vec![AccountType::OracleMeta.as_u8(), 254];
        d.extend_from_slice(&oracle);
        let subject = "Who wins?";
        d.extend_from_slice(&(subject.len() as u16).to_le_bytes());
        d.extend_from_slice(subject.as_bytes());
        let options = ["Yes", "No", "Draw"];
        d.push(options.len() as u8);
        for o in options {
            d.extend_from_slice(&(o.len() as u16).to_le_bytes());
            d.extend_from_slice(o.as_bytes());
        }
        let uri = "https://x/m.json";
        d.extend_from_slice(&(uri.len() as u16).to_le_bytes());
        d.extend_from_slice(uri.as_bytes());
        let uri_hash = [7u8; 32];
        d.extend_from_slice(&uri_hash);

        let m = decode_oracle_meta(&d).expect("decodes");
        assert_eq!(m.oracle, oracle);
        assert_eq!(m.subject, subject);
        assert_eq!(m.options, options);
        assert_eq!(m.uri, uri);
        assert_eq!(m.uri_hash, uri_hash);

        // Truncated / wrong-tag inputs decode to None (never panic).
        assert!(decode_oracle_meta(&d[..20]).is_none());
        let mut bad = d.clone();
        bad[0] = 1;
        assert!(decode_oracle_meta(&bad).is_none());
    }
}
