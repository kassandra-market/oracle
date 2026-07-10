//! Market-lifecycle instruction convenience: create/contribute/cancel/refund/close.

use super::TestCtx;
use litesvm::types::TransactionResult;
use solana_sdk::{
    pubkey::Pubkey,
    signature::{Keypair, Signer},
};

impl TestCtx {
    /// Send a `CreateMarket` instruction for the binary (`outcome_index = 0`)
    /// sub-market. `creator` signs (and pays rent for the market/escrow/
    /// contribution PDAs). Returns the market PDA plus the result.
    #[allow(clippy::result_large_err)]
    pub fn create_market(
        &mut self,
        creator: &Keypair,
        oracle: Pubkey,
        kass_mint: Pubkey,
        creator_ata: Pubkey,
        seed: u64,
    ) -> (Pubkey, TransactionResult) {
        self.create_market_full(creator, oracle, kass_mint, creator_ata, seed, 0)
    }

    /// Full `CreateMarket` with an explicit `outcome_index` (the sub-market this
    /// binds to). Returns the sub-market PDA plus the result.
    #[allow(clippy::result_large_err, clippy::too_many_arguments)]
    pub fn create_market_full(
        &mut self,
        creator: &Keypair,
        oracle: Pubkey,
        kass_mint: Pubkey,
        creator_ata: Pubkey,
        seed: u64,
        outcome_index: u8,
    ) -> (Pubkey, TransactionResult) {
        let (market, _) = kassandra_markets_sdk::pda::market(&oracle, outcome_index);
        let ix = kassandra_markets_sdk::ix::create_market(
            &creator.pubkey(),
            &oracle,
            &kass_mint,
            &creator_ata,
            seed,
            outcome_index,
        );
        let res = self.send(ix, &[creator]);
        (market, res)
    }

    /// Send a `Contribute` instruction. `contributor` signs (and is the token
    /// authority for `contributor_ata`). Returns the LiteSVM result.
    #[allow(clippy::result_large_err)]
    pub fn contribute(
        &mut self,
        contributor: &solana_sdk::signature::Keypair,
        market: Pubkey,
        contributor_ata: Pubkey,
        amount: u64,
    ) -> litesvm::types::TransactionResult {
        let (escrow, _) = kassandra_markets_sdk::pda::escrow(&market);
        let ix = kassandra_markets_sdk::ix::contribute(
            &contributor.pubkey(),
            &market,
            &escrow,
            &contributor_ata,
            amount,
        );
        self.send(ix, &[contributor])
    }

    /// Send a `Cancel` instruction (permissionless). Returns the LiteSVM result.
    #[allow(clippy::result_large_err)]
    pub fn cancel(&mut self, market: Pubkey, oracle: Pubkey) -> litesvm::types::TransactionResult {
        let ix = kassandra_markets_sdk::ix::cancel(&market, &oracle);
        self.send(ix, &[])
    }

    /// Send a `Refund` instruction (permissionless). Derives the escrow and the
    /// contribution PDA from `market` + `contributor`, and refunds to
    /// `contributor_ata`. Returns the LiteSVM result.
    #[allow(clippy::result_large_err)]
    pub fn refund(
        &mut self,
        market: Pubkey,
        contributor: Pubkey,
        contributor_ata: Pubkey,
    ) -> litesvm::types::TransactionResult {
        let (escrow, _) = kassandra_markets_sdk::pda::escrow(&market);
        let (contribution, _) = kassandra_markets_sdk::pda::contribution(&market, &contributor);
        let ix = kassandra_markets_sdk::ix::refund(
            &market,
            &escrow,
            &contribution,
            &contributor_ata,
            &contributor,
        );
        self.send(ix, &[])
    }

    /// Send a `CloseMarket` (Ix 10) for the binary (`outcome_index = 0`) sub-market.
    /// Permissionless; reclaims all rent to `creator`. Returns the LiteSVM result.
    #[allow(clippy::result_large_err)]
    pub fn close_market(
        &mut self,
        oracle: Pubkey,
        creator: Pubkey,
    ) -> litesvm::types::TransactionResult {
        let ix = kassandra_markets_sdk::ix::close_market(&oracle, &creator, 0);
        self.send(ix, &[])
    }

    /// Attack variant of `refund`: derives the `Contribution` PDA from
    /// `contributor` (the recorded staker) but sends the tokens to an arbitrary
    /// `dest_ata`. Used to prove a cranker cannot redirect someone's refund.
    #[allow(clippy::result_large_err)]
    pub fn refund_to(
        &mut self,
        market: Pubkey,
        contributor: Pubkey,
        dest_ata: Pubkey,
    ) -> litesvm::types::TransactionResult {
        let (escrow, _) = kassandra_markets_sdk::pda::escrow(&market);
        let (contribution, _) = kassandra_markets_sdk::pda::contribution(&market, &contributor);
        // Recorded contributor is `contributor`; the wrong-dest guard fires before
        // the contributor binding is checked, so pass the real contributor here.
        let ix = kassandra_markets_sdk::ix::refund(
            &market,
            &escrow,
            &contribution,
            &dest_ata,
            &contributor,
        );
        self.send(ix, &[])
    }

    /// Attack variant of `refund` with an explicit `Contribution` account:
    /// derives the escrow from `market` but pairs it with an arbitrary
    /// `contribution` PDA (e.g. one belonging to a DIFFERENT market), to prove
    /// the `contribution.market != market` cross-market guard fires. Sends to
    /// `dest_ata`. Returns the LiteSVM result.
    #[allow(clippy::result_large_err)]
    pub fn refund_with_contribution(
        &mut self,
        market: Pubkey,
        contribution: Pubkey,
        dest_ata: Pubkey,
    ) -> litesvm::types::TransactionResult {
        let (escrow, _) = kassandra_markets_sdk::pda::escrow(&market);
        // The cross-market guard fires before the contributor binding is checked, so
        // the placeholder contributor (`dest_ata`) is never validated.
        let ix =
            kassandra_markets_sdk::ix::refund(&market, &escrow, &contribution, &dest_ata, &dest_ata);
        self.send(ix, &[])
    }
}
