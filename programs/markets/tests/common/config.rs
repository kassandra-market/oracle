//! `Config`-singleton instruction convenience: init/update and fee accessors.

use super::TestCtx;
use litesvm::types::TransactionResult;
use solana_sdk::{
    pubkey::Pubkey,
    signature::{Keypair, Signer},
};

impl TestCtx {
    /// `init_config` signed by an arbitrary `signer` (the ix payer). Used by the
    /// front-run negative test: when `signer` is NOT the program's upgrade
    /// authority the processor must reject with `NotUpgradeAuthority`. The harness
    /// `payer` remains the fee payer; `signer` co-signs.
    #[allow(clippy::result_large_err)]
    pub fn init_config_signed_by(
        &mut self,
        signer: &Keypair,
        authority: Pubkey,
        kass_mint: Pubkey,
        min_liquidity: u64,
        fee_bps: u16,
        fee_destination: Pubkey,
    ) -> TransactionResult {
        let ix = kassandra_markets_sdk::ix::init_config(
            &signer.pubkey(),
            &kass_mint,
            &authority,
            min_liquidity,
            fee_bps,
            &fee_destination,
        );
        self.send(ix, &[signer])
    }

    /// Send an `InitConfig` instruction creating the `Config` singleton. Returns
    /// the config PDA plus the result so tests can assert success / rejection.
    ///
    /// Threads a default protocol fee (100 bps) and a freshly fabricated KASS
    /// `fee_destination` (a token account on `kass_mint` owned by `authority`), so
    /// existing tests need not care about the fee config. Use
    /// [`TestCtx::init_config_full`] to control the fee args.
    #[allow(clippy::result_large_err)]
    pub fn init_config(
        &mut self,
        authority: Pubkey,
        kass_mint: Pubkey,
        min_liquidity: u64,
    ) -> (Pubkey, TransactionResult) {
        let fee_destination = self.create_token_account(kass_mint, authority, 0);
        self.init_config_full(authority, kass_mint, min_liquidity, 100, fee_destination)
    }

    /// Full `InitConfig` with explicit `fee_bps` + `fee_destination` (for the
    /// fee-validation tests). Returns the config PDA plus the result.
    #[allow(clippy::result_large_err)]
    pub fn init_config_full(
        &mut self,
        authority: Pubkey,
        kass_mint: Pubkey,
        min_liquidity: u64,
        fee_bps: u16,
        fee_destination: Pubkey,
    ) -> (Pubkey, TransactionResult) {
        let (config, _) = kassandra_markets_sdk::pda::config();
        let ix = kassandra_markets_sdk::ix::init_config(
            &self.payer.pubkey(),
            &kass_mint,
            &authority,
            min_liquidity,
            fee_bps,
            &fee_destination,
        );
        let res = self.send(ix, &[]);
        (config, res)
    }

    /// Read the `Config` singleton's `fee_destination` (a KASS token account).
    pub fn config_fee_destination(&self) -> Pubkey {
        use kassandra_markets_program::state::Config;
        let (config, _) = kassandra_markets_sdk::pda::config();
        Pubkey::new_from_array(self.read_pod::<Config>(config).fee_destination.to_bytes())
    }

    /// Send an `UpdateConfig` instruction. The `authority` signs as an extra
    /// signer (the payer remains fee-payer). Threads a default fee (100 bps) and a
    /// freshly fabricated KASS `fee_destination` on `kass_mint`; use
    /// [`TestCtx::update_config_full`] to control the fee args.
    #[allow(clippy::result_large_err)]
    pub fn update_config(
        &mut self,
        authority: &solana_sdk::signature::Keypair,
        kass_mint: Pubkey,
        min_liquidity: u64,
    ) -> litesvm::types::TransactionResult {
        let fee_destination = self.create_token_account(kass_mint, authority.pubkey(), 0);
        self.update_config_full(authority, min_liquidity, 100, fee_destination)
    }

    /// Full `UpdateConfig` with explicit `fee_bps` + `fee_destination`.
    #[allow(clippy::result_large_err)]
    pub fn update_config_full(
        &mut self,
        authority: &solana_sdk::signature::Keypair,
        min_liquidity: u64,
        fee_bps: u16,
        fee_destination: Pubkey,
    ) -> litesvm::types::TransactionResult {
        let ix = kassandra_markets_sdk::ix::update_config(
            &authority.pubkey(),
            min_liquidity,
            fee_bps,
            &fee_destination,
        );
        self.send(ix, &[authority])
    }
}
