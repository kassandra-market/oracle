//! Hand-written Carbon `AccountDecoder` for the kassandra-market program.
//!
//! Dispatch is first-byte (`account_type` tag @0): Config=1, Market=2,
//! Contribution=3. The tag is field 0 of the Pod struct, so we decode the WHOLE
//! buffer with `bytemuck::pod_read_unaligned` (NOT `split_first`). The layouts are
//! REUSED from `kassandra_market_program::state` — one source of truth.

use carbon_core::account::{AccountDecoder, DecodedAccount};
use kassandra_market_program::state::{AccountType, Config, Contribution, Market};
use solana_pubkey::Pubkey;

/// A decoded kassandra-market account. (The Pod layouts don't derive `Debug`,
/// so neither does this wrapper.) The variants differ in size but are short-lived
/// per-account decode values, so the size delta is fine.
#[derive(Clone)]
#[allow(clippy::large_enum_variant)]
// The Config/Market payloads are carried for symmetry + validation (the decode
// proves tag+length); the persist path only needs the variant tag, and reads
// decode the raw bytes separately — so their inner value is intentionally unread.
#[allow(dead_code)]
pub enum KassandraAccount {
    Config(Config),
    Market(Market),
    Contribution(Contribution),
}

pub struct KassandraAccountDecoder {
    pub program_id: Pubkey,
}

impl<'a> AccountDecoder<'a> for KassandraAccountDecoder {
    type AccountType = KassandraAccount;

    fn decode_account(
        &self,
        account: &'a solana_account::Account,
    ) -> Option<DecodedAccount<Self::AccountType>> {
        // Only our program's accounts.
        if account.owner.to_bytes() != self.program_id.to_bytes() {
            return None;
        }

        let data = account.data.as_slice();
        let tag = *data.first()?;

        // Tag @0 == AccountType; guard exact length then decode the whole Pod struct
        // (the tag is field 0). `pod_read_unaligned` (like the on-chain loader) reads
        // a copy with NO alignment requirement — `account.data` is a `Vec<u8>`
        // (1-byte aligned), so `try_from_bytes`'s 8-byte-alignment check could
        // otherwise spuriously drop a valid account.
        let decoded = if tag == AccountType::Config.as_u8() {
            (data.len() == Config::LEN)
                .then(|| bytemuck::pod_read_unaligned::<Config>(&data[..Config::LEN]))
                .map(KassandraAccount::Config)?
        } else if tag == AccountType::Market.as_u8() {
            (data.len() == Market::LEN)
                .then(|| bytemuck::pod_read_unaligned::<Market>(&data[..Market::LEN]))
                .map(KassandraAccount::Market)?
        } else if tag == AccountType::Contribution.as_u8() {
            (data.len() == Contribution::LEN)
                .then(|| bytemuck::pod_read_unaligned::<Contribution>(&data[..Contribution::LEN]))
                .map(KassandraAccount::Contribution)?
        } else {
            return None;
        };

        Some(DecodedAccount {
            lamports: account.lamports,
            data: decoded,
            owner: account.owner,
            executable: account.executable,
            rent_epoch: account.rent_epoch,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytemuck::Zeroable;
    use solana_account::Account;

    fn account(owner: Pubkey, data: Vec<u8>) -> Account {
        Account {
            lamports: 42,
            data,
            owner,
            executable: false,
            rent_epoch: 0,
        }
    }

    fn program() -> Pubkey {
        crate::market::default_program_id()
    }

    #[test]
    fn decodes_config() {
        let mut cfg = Config::zeroed();
        cfg.account_type = AccountType::Config.as_u8();
        cfg.min_liquidity = 1_000_000;
        cfg.fee_bps = 250;
        let acc = account(program(), bytemuck::bytes_of(&cfg).to_vec());
        let dec = KassandraAccountDecoder {
            program_id: program(),
        };
        match dec.decode_account(&acc).expect("decodes").data {
            KassandraAccount::Config(c) => {
                assert_eq!(c.min_liquidity, 1_000_000);
                assert_eq!(c.fee_bps, 250);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn decodes_market() {
        let mut m = Market::zeroed();
        m.account_type = AccountType::Market.as_u8();
        m.total_contributed = 777;
        m.status = 4; // Cancelled
        let acc = account(program(), bytemuck::bytes_of(&m).to_vec());
        let dec = KassandraAccountDecoder {
            program_id: program(),
        };
        match dec.decode_account(&acc).expect("decodes").data {
            KassandraAccount::Market(mm) => {
                assert_eq!(mm.total_contributed, 777);
                assert_eq!(mm.status, 4);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn decodes_contribution() {
        let mut c = Contribution::zeroed();
        c.account_type = AccountType::Contribution.as_u8();
        c.amount = 555;
        let acc = account(program(), bytemuck::bytes_of(&c).to_vec());
        let dec = KassandraAccountDecoder {
            program_id: program(),
        };
        match dec.decode_account(&acc).expect("decodes").data {
            KassandraAccount::Contribution(cc) => assert_eq!(cc.amount, 555),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn rejects_wrong_owner() {
        let mut cfg = Config::zeroed();
        cfg.account_type = AccountType::Config.as_u8();
        let acc = account(Pubkey::new_unique(), bytemuck::bytes_of(&cfg).to_vec());
        let dec = KassandraAccountDecoder {
            program_id: program(),
        };
        assert!(dec.decode_account(&acc).is_none());
    }

    #[test]
    fn rejects_wrong_size() {
        // Config tag but a Market-sized buffer -> length guard rejects.
        let mut buf = vec![0u8; Market::LEN];
        buf[0] = AccountType::Config.as_u8();
        let acc = account(program(), buf);
        let dec = KassandraAccountDecoder {
            program_id: program(),
        };
        assert!(dec.decode_account(&acc).is_none());
    }

    #[test]
    fn rejects_unknown_tag() {
        let mut buf = vec![0u8; Config::LEN];
        buf[0] = 9;
        let acc = account(program(), buf);
        let dec = KassandraAccountDecoder {
            program_id: program(),
        };
        assert!(dec.decode_account(&acc).is_none());
    }

    #[test]
    fn rejects_empty() {
        let acc = account(program(), vec![]);
        let dec = KassandraAccountDecoder {
            program_id: program(),
        };
        assert!(dec.decode_account(&acc).is_none());
    }
}
