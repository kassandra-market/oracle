use super::*;

impl TestCtx {
    // ----- seeding -----------------------------------------------------------

    /// Fabricate an oracle already in [`Phase::FactProposal`] with one proposer
    /// per spec, plus a funded stake vault. Returns the Oracle PDA.
    ///
    /// All counts and balances are kept internally consistent:
    /// `proposer_count == surviving_count == specs.len()`,
    /// `total_oracle_stake == Σ bond == vault token balance`.
    pub fn seed_disputed_oracle(&mut self, specs: &[ProposerSpec]) -> Pubkey {
        assert!(!specs.is_empty(), "need at least one proposer to dispute");

        let nonce = self.next_nonce;
        self.next_nonce += 1;
        let (oracle_pda, bump) = Self::oracle_pda(&self.program_id, nonce);

        let total_stake: u64 = specs.iter().map(|s| s.bond).sum();
        let max_option = specs.iter().map(|s| s.option).max().unwrap();
        // At least 2 options (a dispute needs ≥2), and enough to index every
        // proposed option.
        let options_count = (max_option as u16 + 1).max(2) as u8;

        // Stake vault: SPL token account on KASS, owner == oracle PDA, holding
        // exactly the summed bonds, BACKED by real mint supply. The backing is
        // required so a terminal InvalidDeadend burn (finalize_oracle /
        // finalize_no_facts burning the slashed `bond_pool` back to the reservoir)
        // has real supply to check-subtract — a real `Burn` underflows otherwise.
        // It mirrors reality (proposer bonds are circulating KASS) and is captured
        // by every supply-DELTA assertion (tests snapshot supply AFTER seeding).
        let stake_vault = self.create_token_account(self.kass_mint, oracle_pda, total_stake);
        self.add_mint_supply(self.kass_mint, total_stake);

        // Build and write the Oracle account.
        let now = self.now();
        let mut oracle = Oracle::zeroed();
        oracle.account_type = AccountType::Oracle.as_u8();
        oracle.creator = self.payer.pubkey().to_bytes().into();
        oracle.kass_mint = self.kass_mint.to_bytes().into();
        oracle.usdc_mint = self.usdc_mint.to_bytes().into();
        oracle.stake_vault = stake_vault.to_bytes().into();
        oracle.deadline = now;
        oracle.phase_ends_at = now + WINDOW;
        oracle.twap_window = TWAP_WINDOW;
        oracle.options_count = options_count;
        oracle.set_phase(Phase::FactProposal);
        oracle.proposer_count = specs.len() as u16;
        oracle.surviving_count = specs.len() as u16;
        oracle.fact_count = 0;
        oracle.total_oracle_stake = total_stake;
        oracle.bond_pool = 0;
        // Fixed fact-quorum denominator: Σ proposer bonds at dispute start.
        oracle.dispute_bond_total = total_stake;
        oracle.settled_count = 0;
        oracle.bump = bump;
        // F2: snapshot the governable behavioral params from the config consts,
        // exactly as `create_oracle` would (Protocol defaults == these consts),
        // so a fabricated oracle behaves identically to a real one.
        oracle.threshold_num = THRESHOLD_NUM;
        oracle.threshold_den = THRESHOLD_DEN;
        oracle.market_threshold_num = MARKET_THRESHOLD_NUM as u64;
        oracle.market_threshold_den = MARKET_THRESHOLD_DEN as u64;
        oracle.flip_slash_num = FLIP_SLASH_NUM;
        oracle.flip_slash_den = FLIP_SLASH_DEN;
        oracle.phase_window = PHASE_WINDOW;
        oracle.proposal_window = PROPOSAL_WINDOW;
        // Settlement-era (S1) snapshot fields. NOTE: deliberately kept at the
        // conservative pre-S1 defaults (no approve-voter slash, zero reward
        // weights) rather than the now-real `init_protocol`/`create_oracle`
        // defaults (1/2 slash, 2/1 weights). This keeps fabricated-oracle
        // `finalize_facts` behavior a pure counter (rejected facts add only the
        // submitter stake to `bond_pool`), so the existing `finalize_facts` /
        // conservation (`invariants.rs`) fixtures stay self-consistent. Tests
        // that exercise the approve-voter slash opt in via
        // [`TestCtx::set_fact_vote_slash`]. `fact_vote_slash_den` stays positive
        // so a settlement-era reader never divides by zero. The 3 S1 resolution
        // totals (`total_correct_proposer_stake` / `total_approved_fact_stake` /
        // `reward_pool`) stay 0 (their `zeroed()` default), correct pre-resolution.
        oracle.fact_vote_slash_num = 0;
        oracle.fact_vote_slash_den = 1;
        oracle.reward_proposer_weight = 0;
        oracle.reward_fact_weight = 0;
        // C1 challenge-fee config snapshot (matches init_protocol/create_oracle
        // defaults), so a fabricated oracle sizes/settles like a real one.
        oracle.challenge_fail_usdc_fee_num = CHALLENGE_FAIL_USDC_FEE_NUM;
        oracle.challenge_fail_usdc_fee_den = CHALLENGE_FAIL_USDC_FEE_DEN;
        oracle.challenge_success_kass_fee_num = CHALLENGE_SUCCESS_KASS_FEE_NUM;
        oracle.challenge_success_kass_fee_den = CHALLENGE_SUCCESS_KASS_FEE_DEN;
        self.set_program_account(oracle_pda, bytemuck::bytes_of(&oracle).to_vec());

        // Build and write each Proposer account.
        let mut proposers = Vec::with_capacity(specs.len());
        for spec in specs {
            let authority = Keypair::new();
            let (pda, p_bump) =
                Self::proposer_pda(&self.program_id, &oracle_pda, &authority.pubkey());

            let mut proposer = Proposer::zeroed();
            proposer.account_type = AccountType::Proposer.as_u8();
            proposer.oracle = oracle_pda.to_bytes().into();
            proposer.authority = authority.pubkey().to_bytes().into();
            proposer.bond = spec.bond;
            proposer.original_option = spec.option;
            proposer.claim_option = CLAIM_OPTION_NONE;
            proposer.disqualified = 0;
            proposer.slashed = 0;
            proposer.flipped = 0;
            proposer.bump = p_bump;
            self.set_program_account(pda, bytemuck::bytes_of(&proposer).to_vec());

            proposers.push(SeededProposer {
                authority,
                pda,
                option: spec.option,
                bond: spec.bond,
            });
        }

        self.oracles.insert(
            oracle_pda,
            SeededOracle {
                pda: oracle_pda,
                bump,
                nonce,
                stake_vault,
                proposers,
            },
        );
        oracle_pda
    }

    /// Overwrite the phase byte of an already-seeded oracle. Lets tests stand
    /// up an oracle in a non-`FactProposal` phase (e.g. `FactVoting`) to drive
    /// wrong-phase paths, without a real phase-advance instruction.
    pub fn set_phase(&mut self, oracle: Pubkey, phase: Phase) {
        let mut o = self.oracle(oracle);
        o.set_phase(phase);
        self.set_program_account(oracle, bytemuck::bytes_of(&o).to_vec());
    }

    /// Airdrop SOL lamports to `account` (so it exists as a funded system
    /// account, e.g. a rent recipient).
    pub fn airdrop(&mut self, account: &Keypair, lamports: u64) {
        self.svm.airdrop(&account.pubkey(), lamports).unwrap();
    }

    /// Overwrite the `dispute_bond_total` of a seeded oracle. Lets tests drive
    /// the defensive zero-denominator path in `finalize_facts`.
    pub fn set_dispute_bond_total(&mut self, oracle: Pubkey, value: u64) {
        let mut o = self.oracle(oracle);
        o.dispute_bond_total = value;
        self.set_program_account(oracle, bytemuck::bytes_of(&o).to_vec());
    }

    /// Mark a seeded Proposer account as disqualified. Lets tests drive the
    /// disqualified-submitter rejection in `submit_ai_claim` and the defensive
    /// already-disqualified branch in `finalize_ai_claims` without a real slash
    /// instruction from an earlier phase.
    pub fn set_proposer_disqualified(&mut self, proposer: Pubkey) {
        let mut p = self.proposer(proposer);
        p.disqualified = 1;
        self.set_program_account(proposer, bytemuck::bytes_of(&p).to_vec());
    }

    /// Overwrite a seeded Proposer's `claim_option`. Lets `finalize_oracle`
    /// tests stand up surviving proposers with chosen post-AI-claim votes
    /// without driving the full submit/finalize-AI-claim flow.
    pub fn set_proposer_claim_option(&mut self, proposer: Pubkey, option: u8) {
        let mut p = self.proposer(proposer);
        p.claim_option = option;
        self.set_program_account(proposer, bytemuck::bytes_of(&p).to_vec());
    }

    /// Overwrite an oracle's `surviving_count`. Lets `finalize_oracle` tests
    /// keep the count consistent with a hand-disqualified proposer set (e.g.
    /// the all-disqualified dead-end) without a real slash instruction.
    pub fn set_surviving_count(&mut self, oracle: Pubkey, count: u16) {
        let mut o = self.oracle(oracle);
        o.surviving_count = count;
        self.set_program_account(oracle, bytemuck::bytes_of(&o).to_vec());
    }

    /// Overwrite an oracle's `open_challenge_count`. Lets `finalize_oracle`
    /// tests drive the `ChallengesOutstanding` gate (an unsettled challenge
    /// market) without standing up a real MetaDAO challenge.
    pub fn set_open_challenge_count(&mut self, oracle: Pubkey, count: u16) {
        let mut o = self.oracle(oracle);
        o.open_challenge_count = count;
        self.set_program_account(oracle, bytemuck::bytes_of(&o).to_vec());
    }

    /// Stamp a prior (flip) slash on a seeded proposer: `slashed_amount = amount`,
    /// `slashed = flipped = 1`, still surviving + NOT disqualified, and add
    /// `amount` to the oracle's `bond_pool` (keeping the per-proposer identity
    /// `slashed_amount == bond_pool contribution`). Lets `settle_challenge` tests
    /// stand up the finalize_ai_claims flip-slash → challenged → disqualified
    /// cross-path without driving that earlier phase.
    pub fn set_proposer_prior_slash(&mut self, oracle: Pubkey, proposer: Pubkey, amount: u64) {
        let mut p = self.proposer(proposer);
        p.slashed = 1;
        p.flipped = 1;
        p.slashed_amount = amount;
        self.set_program_account(proposer, bytemuck::bytes_of(&p).to_vec());
        let mut o = self.oracle(oracle);
        o.bond_pool += amount;
        self.set_program_account(oracle, bytemuck::bytes_of(&o).to_vec());
    }

    /// Overwrite a seeded oracle's `fact_vote_slash` snapshot (the rejected-fact
    /// approve-voter slash fraction `num/den`, the same field `create_oracle`
    /// snapshots from `Protocol` and `set_config` retunes). Lets `finalize_facts`
    /// tests drive the approve-voter aggregate slash without the full real flow.
    pub fn set_fact_vote_slash(&mut self, oracle: Pubkey, num: u64, den: u64) {
        let mut o = self.oracle(oracle);
        o.fact_vote_slash_num = num;
        o.fact_vote_slash_den = den;
        self.set_program_account(oracle, bytemuck::bytes_of(&o).to_vec());
    }

    /// Overwrite a seeded oracle's directional challenge-fee snapshot (the same
    /// fields `create_oracle` snapshots from `Protocol` and `set_config` retunes).
    /// Lets `settle_challenge` tests prove the fee is read from the per-oracle
    /// snapshot — i.e. a governance fee change flows into a new challenge's settle
    /// — without driving the full real create/propose/finalize/challenge flow.
    pub fn set_challenge_fees(
        &mut self,
        oracle: Pubkey,
        fail_usdc_num: u64,
        fail_usdc_den: u64,
        success_kass_num: u64,
        success_kass_den: u64,
    ) {
        let mut o = self.oracle(oracle);
        o.challenge_fail_usdc_fee_num = fail_usdc_num;
        o.challenge_fail_usdc_fee_den = fail_usdc_den;
        o.challenge_success_kass_fee_num = success_kass_num;
        o.challenge_success_kass_fee_den = success_kass_den;
        self.set_program_account(oracle, bytemuck::bytes_of(&o).to_vec());
    }

    /// Stamp a seeded oracle's `reward_emission` (the KASS minted at creation,
    /// Task S3) AND physically place that KASS in its `stake_vault`, backed by
    /// mint supply — so a `finalize_oracle` InvalidDeadend burn-back has real
    /// tokens + supply to subtract (no underflow). Lets `finalize_oracle` tests
    /// drive the emission fold-in / burn-back without the full create flow.
    pub fn set_reward_emission(&mut self, oracle: Pubkey, amount: u64) {
        let mut o = self.oracle(oracle);
        o.reward_emission = amount;
        let vault = Pubkey::new_from_array(o.stake_vault.to_bytes());
        self.set_program_account(oracle, bytemuck::bytes_of(&o).to_vec());
        self.add_token_balance(vault, amount);
        self.add_mint_supply(self.kass_mint, amount);
    }

    /// Overwrite the KASS mint's SPL `mint_authority` (a `COption<Pubkey>`).
    /// Lets the mint-authority-mismatch test point the canonical mint at a
    /// non-PDA authority so `create_oracle`'s emission mint is rejected with
    /// [`kassandra_oracles_program::error::KassandraError::BadMintAuthority`].
    pub fn set_kass_mint_authority(&mut self, authority: Pubkey) {
        let mint = self.kass_mint;
        let acc = self.svm.get_account(&mint).expect("kass mint not found");
        let mut state = Mint::unpack(&acc.data).expect("not a mint");
        state.mint_authority = COption::Some(authority);
        let mut data = vec![0u8; Mint::LEN];
        state.pack_into_slice(&mut data);
        self.svm
            .set_account(
                mint,
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

    /// Build a `FinalizeOracle` instruction (Ix 6). Account order:
    /// `[0] oracle(w) [1] kass_mint(w) [2] stake_vault(w) [3] token program`
    /// followed by the read-only proposer tail. Payload = `oracle_nonce` LE
    /// (signs the InvalidDeadend emission burn-back). The oracle must be in the
    /// bookkeeping map (seeded or real-flow) so its nonce/vault are known.
    pub fn finalize_oracle_ix(&self, oracle: Pubkey, tail: &[Pubkey]) -> Instruction {
        let seeded = self.seeded(oracle);
        kassandra_oracles_sdk::ix::finalize_oracle(
            &self.program_id,
            oracle,
            self.kass_mint,
            seeded.stake_vault,
            seeded.nonce,
            tail,
        )
    }

    /// Build a `FinalizeFacts` instruction (Ix 2). Account order (mirrors
    /// `finalize_oracle`'s burn prefix): `[0] oracle(w) [1] kass_mint(w)
    /// [2] stake_vault(w) [3] token program` followed by a WRITABLE tail (the
    /// fact / proposer subset being settled). Payload = `oracle_nonce` LE (signs
    /// the no-facts dead-end `bond_pool` + emission burn). The oracle must be in
    /// the bookkeeping map (seeded or real-flow) so its nonce/vault are known.
    pub fn finalize_facts_ix(&self, oracle: Pubkey, tail: &[Pubkey]) -> Instruction {
        let seeded = self.seeded(oracle);
        kassandra_oracles_sdk::ix::finalize_facts(
            &self.program_id,
            oracle,
            self.kass_mint,
            seeded.stake_vault,
            seeded.nonce,
            tail,
        )
    }
}
