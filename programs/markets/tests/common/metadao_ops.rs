//! MetaDAO composition + the activate/claim/resolve/collect cranks and the
//! client-side split/swap/redeem helpers that drive the composed market.

use super::{MetaDaoRefs, TestCtx};
use litesvm::types::TransactionResult;
use solana_sdk::{
    pubkey::Pubkey,
    signature::{Keypair, Signer},
};

impl TestCtx {
    /// Compose the MetaDAO market for `market`/`oracle` (the client precondition
    /// for `activate`), using the sdks/oracles/rust builders: `initialize_question`
    /// (oracle-authority = the MARKET PDA, question_id = the kassandra oracle
    /// address bytes, num_outcomes = 2), `initialize_conditional_vault`
    /// (underlying = `kass_mint`, creating cYES/cNO mints idx 0/1), and
    /// `create_amm` (base = cYES, quote = cNO, balanced 1e12 initial observation).
    /// Each is its own compute-budgeted transaction. Returns all derived addresses.
    pub fn compose_metadao_market(
        &mut self,
        market: Pubkey,
        oracle: Pubkey,
        kass_mint: Pubkey,
    ) -> MetaDaoRefs {
        use kassandra_markets_sdk::metadao as md;
        let payer = self.payer.pubkey();
        let question_id = oracle.to_bytes();

        // (1) initialize_question — oracle-authority == the MARKET PDA.
        let (question, _) = md::question(&question_id, &market, 2);
        let ix_q = md::initialize_question(&payer, &market, &question_id, 2);
        self.send_many(&[ix_q], &[]).expect("initialize_question");

        // (2) initialize_conditional_vault — underlying == kass_mint.
        let (vault, _) = md::vault(&question, &kass_mint);
        let vault_underlying_ata = md::ata(&vault, &kass_mint);
        let (yes_mint, _) = md::conditional_token_mint(&vault, 0);
        let (no_mint, _) = md::conditional_token_mint(&vault, 1);
        let ix_v = md::initialize_conditional_vault(&payer, &question, &kass_mint, 2);
        self.send_many(&[ix_v], &[])
            .expect("initialize_conditional_vault");

        // (3) create_amm — base = cYES, quote = cNO, balanced (price 1.0).
        let (amm, _) = md::amm(&yes_mint, &no_mint);
        let (lp_mint, _) = md::amm_lp_mint(&amm);
        let amm_vault_base = md::ata(&amm, &yes_mint);
        let amm_vault_quote = md::ata(&amm, &no_mint);
        let max_change: u128 = (u64::MAX as u128) * 1_000_000_000_000;
        let ix_a = md::create_amm(
            &payer,
            &yes_mint,
            &no_mint,
            1_000_000_000_000,
            max_change,
            0,
        );
        self.send_many(&[ix_a], &[]).expect("create_amm");

        MetaDaoRefs {
            question,
            vault,
            vault_underlying_ata,
            yes_mint,
            no_mint,
            amm,
            lp_mint,
            amm_vault_base,
            amm_vault_quote,
        }
    }

    /// Send an `Activate` instruction (fee-payer signs and pays rent for the
    /// three market-owned token accounts). Returns the LiteSVM result.
    #[allow(clippy::result_large_err)]
    pub fn activate(&mut self, oracle: Pubkey, kass_mint: Pubkey) -> TransactionResult {
        self.activate_at(oracle, kass_mint, 0)
    }

    /// Send an `Activate` instruction for the `outcome_index` sub-market. Returns
    /// the LiteSVM result.
    #[allow(clippy::result_large_err)]
    pub fn activate_at(
        &mut self,
        oracle: Pubkey,
        kass_mint: Pubkey,
        outcome_index: u8,
    ) -> TransactionResult {
        let ix = kassandra_markets_sdk::ix::activate(
            &self.payer.pubkey(),
            &oracle,
            &kass_mint,
            outcome_index,
        );
        self.send_many(&[ix], &[])
    }

    /// Send a `ClaimLp` instruction (permissionless). Derives the `lp_vault` and
    /// the `contribution` PDA from `market` + `contributor`, distributing the
    /// pro-rata LP to `contributor_lp_ata`. Returns the LiteSVM result.
    #[allow(clippy::result_large_err)]
    pub fn claim_lp(
        &mut self,
        market: Pubkey,
        contributor: Pubkey,
        contributor_lp_ata: Pubkey,
    ) -> litesvm::types::TransactionResult {
        let (lp_vault, _) = kassandra_markets_sdk::pda::lp_vault(&market);
        let (contribution, _) = kassandra_markets_sdk::pda::contribution(&market, &contributor);
        let ix = kassandra_markets_sdk::ix::claim_lp(
            &market,
            &lp_vault,
            &contribution,
            &contributor_lp_ata,
            &contributor,
        );
        self.send(ix, &[])
    }

    /// Attack variant of `claim_lp` with an explicit `Contribution` account:
    /// derives the `lp_vault` from `market` but pairs it with an arbitrary
    /// `contribution` PDA (e.g. one belonging to a DIFFERENT market), to prove the
    /// `contribution.market != market` cross-market guard fires. Returns the result.
    #[allow(clippy::result_large_err)]
    pub fn claim_lp_with_contribution(
        &mut self,
        market: Pubkey,
        contribution: Pubkey,
        dest_ata: Pubkey,
    ) -> litesvm::types::TransactionResult {
        let (lp_vault, _) = kassandra_markets_sdk::pda::lp_vault(&market);
        // The cross-market guard fires before the contributor binding is checked, so
        // the placeholder contributor (`dest`) is never validated.
        let ix = kassandra_markets_sdk::ix::claim_lp(
            &market,
            &lp_vault,
            &contribution,
            &dest_ata,
            &dest_ata,
        );
        self.send(ix, &[])
    }

    /// Attack variant of `claim_lp`: derives the `Contribution` PDA from the
    /// recorded `contributor` but sends the LP to an arbitrary `dest_ata`. Used
    /// to prove a cranker cannot redirect a contributor's LP (wrong owner) or
    /// point at a non-LP-mint account.
    #[allow(clippy::result_large_err)]
    pub fn claim_lp_to(
        &mut self,
        market: Pubkey,
        contributor: Pubkey,
        dest_ata: Pubkey,
    ) -> litesvm::types::TransactionResult {
        let (lp_vault, _) = kassandra_markets_sdk::pda::lp_vault(&market);
        let (contribution, _) = kassandra_markets_sdk::pda::contribution(&market, &contributor);
        // Recorded contributor is `contributor`; the wrong-dest guard fires before
        // the contributor binding is checked, so pass the real contributor here.
        let ix = kassandra_markets_sdk::ix::claim_lp(
            &market,
            &lp_vault,
            &contribution,
            &dest_ata,
            &contributor,
        );
        self.send(ix, &[])
    }

    /// Send a `ResolveMarket` instruction (permissionless). Derives the
    /// conditional_vault event-authority; a 1.4M-CU budget is prepended for the
    /// `resolve_question` CPI. Returns the LiteSVM result.
    #[allow(clippy::result_large_err)]
    pub fn resolve_market(
        &mut self,
        market: Pubkey,
        oracle: Pubkey,
        question: Pubkey,
    ) -> TransactionResult {
        use kassandra_markets_sdk::metadao as md;
        let (cv_event_auth, _) = md::event_authority(&md::CONDITIONAL_VAULT_ID);
        let ix =
            kassandra_markets_sdk::ix::resolve_market(&market, &oracle, &question, &cv_event_auth);
        self.send_many(&[ix], &[])
    }

    /// Send a `CollectFee` instruction (permissionless crank). Derives every
    /// account from `oracle` + `kass_mint` + the given `fee_destination`; a 1.4M-CU
    /// budget is prepended for the remove_liquidity → redeem → transfer CPIs.
    #[allow(clippy::result_large_err)]
    pub fn collect_fee(
        &mut self,
        oracle: Pubkey,
        kass_mint: Pubkey,
        fee_destination: Pubkey,
    ) -> TransactionResult {
        let ix = kassandra_markets_sdk::ix::collect_fee(&oracle, &kass_mint, &fee_destination, 0);
        self.send_many(&[ix], &[])
    }

    /// Client `amm::swap`: `user` swaps `input` of one conditional leg for the
    /// other (fee accrues to the pool, growing the LP position's value). `user`
    /// owns `user_cyes`/`user_cno`. Returns the LiteSVM result.
    #[allow(clippy::result_large_err, clippy::too_many_arguments)]
    pub fn user_swap(
        &mut self,
        user: &Keypair,
        refs: &MetaDaoRefs,
        user_cyes: Pubkey,
        user_cno: Pubkey,
        swap_type: kassandra_markets_sdk::metadao::SwapType,
        input_amount: u64,
        min_out: u64,
    ) -> TransactionResult {
        use kassandra_markets_sdk::metadao as md;
        let ix = md::swap(
            &user.pubkey(),
            &refs.yes_mint,
            &refs.no_mint,
            &user_cyes,
            &user_cno,
            swap_type,
            input_amount,
            min_out,
        );
        self.send_many(&[ix], &[user])
    }

    /// Client `split_tokens`: `user` splits `amount` KASS out of `user_kass_ata`
    /// into the vault, receiving `amount` of BOTH cYES and cNO into
    /// `user_cyes`/`user_cno`. Returns the LiteSVM result.
    #[allow(clippy::result_large_err, clippy::too_many_arguments)]
    pub fn user_split(
        &mut self,
        user: &Keypair,
        refs: &MetaDaoRefs,
        user_kass_ata: Pubkey,
        user_cyes: Pubkey,
        user_cno: Pubkey,
        amount: u64,
    ) -> TransactionResult {
        use kassandra_markets_sdk::metadao as md;
        let ix = md::split_tokens(
            &user.pubkey(),
            &refs.question,
            &refs.vault,
            &refs.vault_underlying_ata,
            &user_kass_ata,
            &refs.yes_mint,
            &refs.no_mint,
            &user_cyes,
            &user_cno,
            amount,
        );
        self.send_many(&[ix], &[user])
    }

    /// Client `redeem_tokens`: `user` burns their full cYES/cNO balances and
    /// receives the resolved payout underlying into `user_kass_ata`. Returns the
    /// LiteSVM result.
    #[allow(clippy::result_large_err)]
    pub fn redeem(
        &mut self,
        user: &Keypair,
        refs: &MetaDaoRefs,
        user_kass_ata: Pubkey,
        user_cyes: Pubkey,
        user_cno: Pubkey,
    ) -> TransactionResult {
        use kassandra_markets_sdk::metadao as md;
        let ix = md::redeem_tokens(
            &user.pubkey(),
            &refs.question,
            &refs.vault,
            &refs.vault_underlying_ata,
            &user_kass_ata,
            &refs.yes_mint,
            &refs.no_mint,
            &user_cyes,
            &user_cno,
        );
        self.send_many(&[ix], &[user])
    }
}
