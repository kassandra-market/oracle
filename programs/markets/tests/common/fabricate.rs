//! Low-level account fabrication: SPL mints/token accounts and Kassandra oracle
//! / market state stamped directly into the SVM.

use super::{kass_oracle_bytes, kass_oracle_owner, TestCtx};
use solana_sdk::{account::Account, pubkey::Pubkey, signature::Signer};
use spl_token::{
    solana_program::{program_option::COption, program_pack::Pack},
    state::{Account as TokenAccount, AccountState, Mint},
    ID as TOKEN_PROGRAM_ID,
};

impl TestCtx {
    /// Fabricate an initialized SPL mint with the given decimals, authority =
    /// the payer, supply 0. Returns its address.
    pub fn create_mint(&mut self, decimals: u8) -> Pubkey {
        let mint = Pubkey::new_unique();
        let state = Mint {
            mint_authority: COption::Some(self.payer.pubkey()),
            supply: 0,
            decimals,
            is_initialized: true,
            freeze_authority: COption::None,
        };
        let mut data = vec![0u8; Mint::LEN];
        state.pack_into_slice(&mut data);
        let lamports = self.svm.minimum_balance_for_rent_exemption(Mint::LEN);
        self.svm
            .set_account(
                mint,
                Account {
                    lamports,
                    data,
                    owner: TOKEN_PROGRAM_ID,
                    executable: false,
                    rent_epoch: 0,
                },
            )
            .unwrap();
        mint
    }

    /// Fabricate an initialized SPL token account on `mint` owned by `owner`
    /// holding `amount` base units. Returns its address.
    pub fn create_token_account(&mut self, mint: Pubkey, owner: Pubkey, amount: u64) -> Pubkey {
        let addr = Pubkey::new_unique();
        let state = TokenAccount {
            mint,
            owner,
            amount,
            delegate: COption::None,
            state: AccountState::Initialized,
            is_native: COption::None,
            delegated_amount: 0,
            close_authority: COption::None,
        };
        let mut data = vec![0u8; TokenAccount::LEN];
        state.pack_into_slice(&mut data);
        let lamports = self
            .svm
            .minimum_balance_for_rent_exemption(TokenAccount::LEN);
        self.svm
            .set_account(
                addr,
                Account {
                    lamports,
                    data,
                    owner: TOKEN_PROGRAM_ID,
                    executable: false,
                    rent_epoch: 0,
                },
            )
            .unwrap();
        addr
    }

    /// Overwrite an existing SPL token account's `amount` in place (preserving
    /// mint/owner/state) — used to simulate a dust donation into a program-owned
    /// token account (e.g. an escrow PDA a griefer transfers into).
    pub fn set_token_amount(&mut self, addr: Pubkey, amount: u64) {
        let acc = self
            .svm
            .get_account(&addr)
            .unwrap_or_else(|| panic!("token account {addr} not found"));
        let mut state = TokenAccount::unpack(&acc.data).expect("not a token account");
        state.amount = amount;
        let mut data = vec![0u8; TokenAccount::LEN];
        state.pack_into_slice(&mut data);
        self.svm
            .set_account(
                addr,
                Account {
                    lamports: acc.lamports,
                    data,
                    owner: TOKEN_PROGRAM_ID,
                    executable: false,
                    rent_epoch: 0,
                },
            )
            .unwrap();
    }

    /// Read the token balance (base units) of an SPL token account.
    pub fn token_balance(&self, key: Pubkey) -> u64 {
        let acc = self
            .svm
            .get_account(&key)
            .unwrap_or_else(|| panic!("token account {key} not found"));
        TokenAccount::unpack(&acc.data)
            .expect("not a token account")
            .amount
    }

    /// Fabricate a minimal Kassandra `Oracle` account (owned by the Kassandra
    /// program) carrying the given `options_count` and `phase`, so market tests
    /// can point `create_market` at a real-looking oracle. Returns its address.
    pub fn seed_kass_oracle(&mut self, options_count: u8, phase: u8) -> Pubkey {
        let addr = Pubkey::new_unique();
        let data = kass_oracle_bytes(options_count, phase, 0);
        let lamports = self.svm.minimum_balance_for_rent_exemption(data.len());
        self.svm
            .set_account(
                addr,
                Account {
                    lamports,
                    data,
                    owner: kass_oracle_owner(),
                    executable: false,
                    rent_epoch: 0,
                },
            )
            .unwrap();
        addr
    }

    /// Fabricate a Market account at its canonical PDA with an arbitrary status,
    /// for testing status guards without going through the full lifecycle.
    pub fn seed_market_with_status(
        &mut self,
        oracle: Pubkey,
        kass_mint: Pubkey,
        escrow: Pubkey,
        status: u8,
    ) -> Pubkey {
        use bytemuck::Zeroable;
        use kassandra_markets_program::state::{AccountType, Market};
        let (market, bump) = kassandra_markets_sdk::pda::market(&oracle, 0);
        let mut m = Market::zeroed();
        m.account_type = AccountType::Market.as_u8();
        m.oracle = oracle.to_bytes().into();

        m.kass_mint = kass_mint.to_bytes().into();

        m.escrow_vault = escrow.to_bytes().into();

        m.status = status;
        m.bump = bump;
        let data = bytemuck::bytes_of(&m).to_vec();
        let lamports = self.svm.minimum_balance_for_rent_exemption(data.len());
        self.svm
            .set_account(
                market,
                Account {
                    lamports,
                    data,
                    owner: self.program_id,
                    executable: false,
                    rent_epoch: 0,
                },
            )
            .unwrap();
        market
    }

    /// Rewrite an existing fabricated Kassandra oracle account to a new phase
    /// (keeps options_count = 2). Lets a test move an oracle to a terminal phase
    /// after a market has been created against it.
    pub fn set_oracle_phase(&mut self, oracle: Pubkey, phase: u8) {
        let data = kass_oracle_bytes(2, phase, 0);
        let lamports = self.svm.minimum_balance_for_rent_exemption(data.len());
        self.svm
            .set_account(
                oracle,
                solana_sdk::account::Account {
                    lamports,
                    data,
                    owner: kass_oracle_owner(),
                    executable: false,
                    rent_epoch: 0,
                },
            )
            .unwrap();
    }

    /// Rewrite a fabricated Kassandra oracle to the terminal `Resolved` phase with
    /// the given `resolved_option` (options_count = 2). Lets a resolve test move an
    /// activated market's oracle to a winning binary outcome.
    pub fn set_oracle_resolved(&mut self, oracle: Pubkey, resolved_option: u8) {
        self.set_oracle_resolved_full(oracle, 2, resolved_option);
    }

    /// Rewrite a fabricated Kassandra oracle to the terminal `Resolved` phase with
    /// an explicit `options_count` + winning `resolved_option`. Lets a categorical
    /// resolve test move a 3-option oracle to a chosen outcome.
    pub fn set_oracle_resolved_full(
        &mut self,
        oracle: Pubkey,
        options_count: u8,
        resolved_option: u8,
    ) {
        let data = kass_oracle_bytes(
            options_count,
            kassandra_markets_program::kass_oracle::PHASE_RESOLVED,
            resolved_option,
        );
        let lamports = self.svm.minimum_balance_for_rent_exemption(data.len());
        self.svm
            .set_account(
                oracle,
                solana_sdk::account::Account {
                    lamports,
                    data,
                    owner: kass_oracle_owner(),
                    executable: false,
                    rent_epoch: 0,
                },
            )
            .unwrap();
    }
}
