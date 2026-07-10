use super::*;

impl TestCtx {
    /// Fold a creation-time `reward_emission` into an already-seeded TERMINAL
    /// `Resolved` oracle, mirroring the `create_oracle` mint + `finalize_oracle`
    /// fold the real flow would produce: physically add `amount` KASS to the
    /// stake vault (backed by mint supply), stamp `reward_emission`, AND add it to
    /// the distributable `reward_pool` (the S3 `reward_pool = bond_pool +
    /// reward_emission`). The S2 claims then read the emission-boosted pool, so a
    /// correct proposer / approved fact staker's reward reflects the emission.
    /// (Use ONLY on a `Resolved` seed; on `InvalidDeadend` the emission would have
    /// been burned back, so it must not sit in the vault.)
    pub fn fold_reward_emission(&mut self, oracle: Pubkey, amount: u64) {
        let mut o = self.oracle(oracle);
        o.reward_emission = amount;
        o.reward_pool += amount;
        let vault = Pubkey::new_from_array(o.stake_vault.to_bytes());
        self.set_program_account(oracle, bytemuck::bytes_of(&o).to_vec());
        self.add_token_balance(vault, amount);
        self.add_mint_supply(self.kass_mint, amount);
    }

    /// Seed a SETTLED-challenge disqualification on a proposer of an oracle seeded
    /// via [`TestCtx::seed_disputed_oracle`]: mark it disqualified + slashed with
    /// `slashed_amount = bond − kass_fee` (the bond_pool contribution), credit
    /// `bond_pool += slashed_amount`, decrement `surviving_count`, and remove the
    /// `kass_fee` KASS from the stake vault (modelling `settle_challenge`'s payout
    /// of `kass_fee` to the challenger). Mirrors the on-chain post-settle state so
    /// the deadend-after-settled-challenge conservation test starts from reality.
    pub fn seed_challenge_disqualify(&mut self, oracle: Pubkey, proposer: Pubkey, kass_fee: u64) {
        let mut p = self.proposer(proposer);
        let slashed_amount = p.bond - kass_fee;
        p.disqualified = 1;
        p.slashed = 1;
        p.slashed_amount = slashed_amount;
        self.set_program_account(proposer, bytemuck::bytes_of(&p).to_vec());

        let mut o = self.oracle(oracle);
        o.bond_pool += slashed_amount;
        o.surviving_count -= 1;
        let vault = Pubkey::new_from_array(o.stake_vault.to_bytes());
        self.set_program_account(oracle, bytemuck::bytes_of(&o).to_vec());
        // The kass_fee physically left the vault to the challenger at settle time.
        self.sub_token_balance(vault, kass_fee);
    }

    /// Build a `ClaimProposer` instruction (Ix 17). Account order:
    /// `[0] oracle(ro) [1] proposer(w) [2] dest_kass(w) [3] stake_vault(w)
    /// [4] rent_recipient(w) [5] token program`. Payload = `oracle_nonce` LE.
    pub fn claim_proposer_ix(
        &self,
        oracle: Pubkey,
        nonce: u64,
        proposer: Pubkey,
        dest_kass: Pubkey,
        stake_vault: Pubkey,
        rent_recipient: Pubkey,
    ) -> Instruction {
        kassandra_oracles_sdk::ix::claim_proposer(
            &self.program_id,
            oracle,
            nonce,
            proposer,
            dest_kass,
            stake_vault,
            rent_recipient,
        )
    }

    /// Build a `ClaimFact` instruction (Ix 18). Same account order as
    /// `claim_proposer_ix` with the `Fact` account at index 1.
    pub fn claim_fact_ix(
        &self,
        oracle: Pubkey,
        nonce: u64,
        fact: Pubkey,
        dest_kass: Pubkey,
        stake_vault: Pubkey,
        rent_recipient: Pubkey,
    ) -> Instruction {
        kassandra_oracles_sdk::ix::claim_fact(
            &self.program_id,
            oracle,
            nonce,
            fact,
            dest_kass,
            stake_vault,
            rent_recipient,
        )
    }

    /// Build a `ClaimFactVote` instruction (Ix 19). Account order:
    /// `[0] oracle(ro) [1] fact_vote(w) [2] fact(ro) [3] dest_kass(w)
    /// [4] stake_vault(w) [5] rent_recipient(w) [6] token program`.
    #[allow(clippy::too_many_arguments)]
    pub fn claim_fact_vote_ix(
        &self,
        oracle: Pubkey,
        nonce: u64,
        fact_vote: Pubkey,
        fact: Pubkey,
        dest_kass: Pubkey,
        stake_vault: Pubkey,
        rent_recipient: Pubkey,
    ) -> Instruction {
        kassandra_oracles_sdk::ix::claim_fact_vote(
            &self.program_id,
            oracle,
            nonce,
            fact_vote,
            fact,
            dest_kass,
            stake_vault,
            rent_recipient,
        )
    }

    /// Lamports balance of any account (0 if it does not exist), for asserting
    /// rent reclamation.
    pub fn lamports(&self, key: Pubkey) -> u64 {
        self.svm.get_account(&key).map(|a| a.lamports).unwrap_or(0)
    }

    /// Whether an account is closed (gone / zero-lamports / zero-length data).
    pub fn is_closed(&self, key: Pubkey) -> bool {
        match self.svm.get_account(&key) {
            None => true,
            Some(a) => a.lamports == 0 || a.data.is_empty(),
        }
    }

    // ----- S4: account-closure (close_ai_claim / close_market) helpers -------

    /// Fabricate an [`AiClaim`] bound to `oracle` + `proposer`, stamping
    /// `authority` (the rent recipient close_ai_claim reads). Returns its
    /// (random) address.
    pub fn seed_ai_claim(&mut self, oracle: Pubkey, proposer: Pubkey, authority: Pubkey) -> Pubkey {
        let mut c = AiClaim::zeroed();
        c.account_type = AccountType::AiClaim.as_u8();
        c.oracle = oracle.to_bytes().into();
        c.proposer = proposer.to_bytes().into();
        c.authority = authority.to_bytes().into();
        self.seed_program_account(bytemuck::bytes_of(&c).to_vec())
    }

    /// Fabricate a [`Market`] bound to `oracle`, recording `challenger`,
    /// `challenger_usdc_vault`, and `settled`. Returns its (random) address.
    pub fn seed_market(
        &mut self,
        oracle: Pubkey,
        challenger: Pubkey,
        challenger_usdc_vault: Pubkey,
        settled: bool,
    ) -> Pubkey {
        let mut m = Market::zeroed();
        m.account_type = AccountType::Market.as_u8();
        m.oracle = oracle.to_bytes().into();
        m.challenger = challenger.to_bytes().into();
        m.challenger_usdc_vault = challenger_usdc_vault.to_bytes().into();
        m.settled = settled as u8;
        self.seed_program_account(bytemuck::bytes_of(&m).to_vec())
    }

    /// Fabricate a USDC escrow token account holding `amount`, with its token
    /// authority set to `owner` (the oracle PDA in the close_market tests).
    pub fn seed_usdc_escrow(&mut self, owner: Pubkey, amount: u64) -> Pubkey {
        self.create_token_account(self.usdc_mint, owner, amount)
    }

    /// Build a `CloseAiClaim` instruction (Ix 20). Account order:
    /// `[0] oracle(ro) [1] ai_claim(w) [2] rent_recipient(w)`. Empty payload.
    pub fn close_ai_claim_ix(
        &self,
        oracle: Pubkey,
        ai_claim: Pubkey,
        rent_recipient: Pubkey,
    ) -> Instruction {
        kassandra_oracles_sdk::ix::close_ai_claim(&self.program_id, oracle, ai_claim, rent_recipient)
    }

    /// Build a `CloseMarket` instruction (Ix 21). Account order:
    /// `[0] oracle(ro) [1] market(w) [2] challenger_usdc_vault(w)
    /// [3] rent_recipient(w) [4] token program`. Payload = `oracle_nonce` LE.
    pub fn close_market_ix(
        &self,
        oracle: Pubkey,
        nonce: u64,
        market: Pubkey,
        challenger_usdc_vault: Pubkey,
        rent_recipient: Pubkey,
    ) -> Instruction {
        kassandra_oracles_sdk::ix::close_market(
            &self.program_id,
            oracle,
            nonce,
            market,
            challenger_usdc_vault,
            rent_recipient,
        )
    }

    // ----- SW1: sweep_oracle (dust → treasury + terminal closure) helpers -----

    /// Overwrite a seeded oracle's `creator` (the rent recipient the sweep /
    /// closes refund to). Lets sweep tests point rent at a fresh keypair distinct
    /// from the fee payer, so an exact lamport-delta assertion is not confounded
    /// by transaction fees.
    pub fn set_creator(&mut self, oracle: Pubkey, creator: Pubkey) {
        let mut o = self.oracle(oracle);
        o.creator = creator.to_bytes().into();
        self.set_program_account(oracle, bytemuck::bytes_of(&o).to_vec());
    }

    /// Add `amount` KASS to a seeded oracle's `stake_vault` (backed by mint
    /// supply, mirroring the harness philosophy), modelling the residual dust /
    /// unclaimed principal a terminal vault retains after (or without) claims.
    pub fn fund_vault(&mut self, oracle: Pubkey, amount: u64) {
        let vault = Pubkey::new_from_array(self.oracle(oracle).stake_vault.to_bytes());
        self.add_token_balance(vault, amount);
        self.add_mint_supply(self.kass_mint, amount);
    }

    /// Derive the canonical KASS associated-token-account of `owner`
    /// (`ATA(owner, kass_mint)` under the ATA program) — the address the DAO
    /// treasury lives at.
    pub fn kass_ata(&self, owner: Pubkey) -> Pubkey {
        Pubkey::find_program_address(
            &[
                owner.as_ref(),
                TOKEN_PROGRAM_ID.as_ref(),
                self.kass_mint.as_ref(),
            ],
            &ATA_PROGRAM_ID,
        )
        .0
    }

    /// Fabricate the DAO treasury: an empty KASS token account AT the canonical
    /// `ATA(owner, kass_mint)` address, owned (token authority) by `owner`.
    /// Returns the ATA address.
    pub fn seed_kass_treasury(&mut self, owner: Pubkey) -> Pubkey {
        let ata = self.kass_ata(owner);
        let state = TokenAccount {
            mint: self.kass_mint,
            owner,
            amount: 0,
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
                ata,
                Account {
                    lamports,
                    data,
                    owner: TOKEN_PROGRAM_ID,
                    executable: false,
                    rent_epoch: 0,
                },
            )
            .unwrap();
        ata
    }

    /// Build a `SweepOracle` instruction (Ix 22). Account order:
    /// `[0] oracle(w) [1] stake_vault(w) [2] protocol(ro) [3] dao_treasury(w)
    /// [4] creator(w) [5] token program`. Payload = `oracle_nonce` LE. Exposes
    /// every account so tests can pass a wrong treasury / creator / vault.
    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_arguments)]
    pub fn sweep_oracle_ix(
        &self,
        oracle: Pubkey,
        nonce: u64,
        stake_vault: Pubkey,
        protocol: Pubkey,
        dao_treasury: Pubkey,
        creator: Pubkey,
        oracle_meta: Option<Pubkey>,
    ) -> Instruction {
        kassandra_oracles_sdk::ix::sweep_oracle(
            &self.program_id,
            oracle,
            nonce,
            stake_vault,
            protocol,
            dao_treasury,
            creator,
            oracle_meta,
        )
    }
}
