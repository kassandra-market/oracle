use super::*;

impl TestCtx {
    /// Send a real `CreateOracle` instruction with `creator == payer`, using the
    /// harness KASS/USDC mints and the protocol singleton. Returns the Oracle PDA
    /// derived from `nonce`. `init_protocol` must have been called first. The
    /// returned [`TransactionResult`] lets tests assert success or the various
    /// rejection paths.
    #[allow(clippy::result_large_err)]
    pub fn create_oracle(
        &mut self,
        nonce: u64,
        options_count: u8,
        deadline: i64,
        twap_window: i64,
    ) -> (Pubkey, TransactionResult) {
        let (oracle_pda, _) = Self::oracle_pda(&self.program_id, nonce);
        let ix = self.create_oracle_ix(
            nonce,
            options_count,
            deadline,
            twap_window,
            oracle_pda,
            self.kass_mint,
            self.usdc_mint,
        );
        let res = self.send(ix, &[]);
        (oracle_pda, res)
    }

    /// Send a `WriteOracleMeta` for `oracle` (creator = payer). Subject/options
    /// go on-chain in the `oracle_meta` PDA; `uri`/`uri_hash` reference the JSON.
    #[allow(clippy::result_large_err)]
    pub fn write_oracle_meta(
        &mut self,
        oracle: Pubkey,
        subject: &str,
        options: &[&str],
        uri: &str,
        uri_hash: [u8; 32],
    ) -> TransactionResult {
        let ix = kassandra_oracles_sdk::ix::write_oracle_meta(
            &self.program_id,
            oracle,
            self.payer.pubkey(),
            subject,
            options,
            uri,
            &uri_hash,
        );
        self.send(ix, &[])
    }

    /// Build a `CreateOracle` instruction. Exposes the oracle account and the
    /// KASS/USDC mints as parameters so tests can pass deliberately wrong values
    /// (mint spoof, etc.). Creator = payer (fee payer signs, pays rent).
    #[allow(clippy::too_many_arguments)]
    pub fn create_oracle_ix(
        &self,
        nonce: u64,
        options_count: u8,
        deadline: i64,
        twap_window: i64,
        oracle: Pubkey,
        kass_mint: Pubkey,
        usdc_mint: Pubkey,
    ) -> Instruction {
        kassandra_oracles_sdk::ix::create_oracle(
            &self.program_id,
            nonce,
            options_count,
            deadline,
            twap_window,
            oracle,
            kass_mint,
            usdc_mint,
            self.payer.pubkey(),
            self.payer_kass,
        )
    }

    /// Send a real `Propose` instruction registering `authority`'s proposal
    /// (`option` + KASS `bond`) against `oracle`. Airdrops the authority SOL for
    /// rent and funds it a KASS token account holding `bond` (the bond source).
    /// `authority` co-signs. Returns the Proposer PDA + result. The oracle must
    /// already be in [`Phase::Proposal`] (i.e. created via `create_oracle`).
    #[allow(clippy::result_large_err)]
    pub fn propose(
        &mut self,
        oracle: Pubkey,
        authority: &Keypair,
        option: u8,
        bond: u64,
    ) -> (Pubkey, TransactionResult) {
        // Fund the authority: SOL for the Proposer-PDA rent, and a KASS account
        // holding the bond. Fund at least 1 base unit so the bond==0 path still
        // has a valid source account.
        self.svm
            .airdrop(&authority.pubkey(), 1_000_000_000)
            .unwrap();
        let authority_kass = self.fund_kass(authority, bond.max(1));
        let (proposer_pda, _) = Self::proposer_pda(&self.program_id, &oracle, &authority.pubkey());
        let (vault, _) = Self::stake_vault_pda(&self.program_id, &oracle);
        let ix = self.propose_ix(
            oracle,
            proposer_pda,
            authority.pubkey(),
            authority_kass,
            vault,
            option,
            bond,
        );
        let res = self.send(ix, &[authority]);
        (proposer_pda, res)
    }

    /// Build a `Propose` instruction with the locked-in account order. Exposes
    /// the proposer/authority-KASS/vault accounts so tests can pass deliberately
    /// wrong values.
    #[allow(clippy::too_many_arguments)]
    pub fn propose_ix(
        &self,
        oracle: Pubkey,
        proposer: Pubkey,
        authority: Pubkey,
        authority_kass: Pubkey,
        vault: Pubkey,
        option: u8,
        bond: u64,
    ) -> Instruction {
        kassandra_oracles_sdk::ix::propose(
            &self.program_id,
            oracle,
            proposer,
            authority,
            authority_kass,
            vault,
            option,
            bond,
        )
    }

    /// Build a `FinalizeProposals` instruction: `[0] oracle(w)` followed by the
    /// given proposer accounts as a READ-ONLY tail. Exposes the full proposer
    /// slice so tests can pass a subset, a duplicate, or a foreign-oracle account.
    pub fn finalize_proposals_ix(&self, oracle: Pubkey, proposers: &[Pubkey]) -> Instruction {
        kassandra_oracles_sdk::ix::finalize_proposals(&self.program_id, oracle, proposers)
    }

    // ----- real-flow builders ------------------------------------------------

    /// Ensure the Protocol singleton exists, calling `init_protocol` exactly
    /// once per `TestCtx` (idempotent thereafter). Many real oracles share one
    /// protocol, so this guards against a double-init `AlreadyInitialized`
    /// failure when several `create_real_oracle` calls run in the same context.
    /// Returns the Protocol PDA.
    pub fn ensure_protocol(&mut self) -> Pubkey {
        let (protocol_pda, _) = Self::protocol_pda(&self.program_id);
        if !self.protocol_initialized {
            let (_p, res) = self.init_protocol();
            assert!(res.is_ok(), "init_protocol should succeed: {res:?}");
            self.protocol_initialized = true;
        }
        protocol_pda
    }

    /// Real-flow oracle builder: `init_protocol` (once per ctx) + a real
    /// `create_oracle` with a near `deadline`, then warps to the deadline so the
    /// proposal window is open. Uses a fresh nonce from the internal counter and
    /// records the oracle in the bookkeeping map so [`TestCtx::seeded`],
    /// [`TestCtx::proposers`], and the stake-vault accessor work for it exactly
    /// like a `seed_disputed_oracle` oracle. Returns the Oracle PDA.
    pub fn create_real_oracle(&mut self, options_count: u8, twap_window: i64) -> Pubkey {
        self.ensure_protocol();
        let nonce = self.next_nonce;
        self.next_nonce += 1;
        let (oracle, bump) = Self::oracle_pda(&self.program_id, nonce);
        let deadline = self.now() + DEADLINE_DELTA;
        let (created, res) = self.create_oracle(nonce, options_count, deadline, twap_window);
        assert!(res.is_ok(), "create_oracle should succeed: {res:?}");
        debug_assert_eq!(created, oracle);
        // Warp to the deadline: proposals open at `deadline`, window now open.
        self.warp(DEADLINE_DELTA);

        let (stake_vault, _) = Self::stake_vault_pda(&self.program_id, &oracle);
        self.oracles.insert(
            oracle,
            SeededOracle {
                pda: oracle,
                bump,
                nonce,
                stake_vault,
                proposers: Vec::new(),
            },
        );
        oracle
    }

    /// Real-flow proposer registration: funds a fresh authority and sends a real
    /// `propose` (`option` + KASS `bond`) against `oracle`. Records the proposer
    /// in the oracle's bookkeeping (so [`TestCtx::proposers`] and
    /// [`TestCtx::finalize_proposals_real`] see the full set) and returns the
    /// authority keypair + Proposer PDA. Panics if the propose fails.
    pub fn propose_real(&mut self, oracle: Pubkey, option: u8, bond: u64) -> (Keypair, Pubkey) {
        let authority = Keypair::new();
        let (pda, res) = self.propose(oracle, &authority, option, bond);
        assert!(
            res.is_ok(),
            "propose(option={option}) should succeed: {res:?}"
        );
        if let Some(seeded) = self.oracles.get_mut(&oracle) {
            seeded.proposers.push(SeededProposer {
                authority: authority.insecure_clone(),
                pda,
                option,
                bond,
            });
        }
        (authority, pda)
    }

    /// Real-flow proposal finalization: warp past the proposal window, then send
    /// a real `finalize_proposals` carrying the FULL tracked proposer set as the
    /// read-only tail. Returns the transaction result so callers can assert the
    /// resolve / open-dispute outcome (or a rejection).
    #[allow(clippy::result_large_err)]
    pub fn finalize_proposals_real(&mut self, oracle: Pubkey) -> TransactionResult {
        self.warp(PROPOSAL_WINDOW + 1);
        let proposers: Vec<Pubkey> = self.proposers(oracle).iter().map(|p| p.pda).collect();
        let ix = self.finalize_proposals_ix(oracle, &proposers);
        self.send(ix, &[])
    }

    /// Real-flow analogue of [`TestCtx::seed_disputed_oracle`]: drive
    /// create_oracle → propose (one per spec) → finalize_proposals through the
    /// genuine entry points, landing the oracle in [`Phase::FactProposal`] with
    /// the proposers registered and `dispute_bond_total` set. The specs MUST
    /// contain at least two DISTINCT options (otherwise finalize_proposals
    /// resolves instead of opening a dispute); this is asserted post-finalize.
    /// Returns the Oracle PDA.
    pub fn dispute_via_real_flow(&mut self, specs: &[ProposerSpec]) -> Pubkey {
        assert!(
            specs.len() >= 2,
            "a real-flow dispute needs at least two proposers"
        );
        let max_option = specs.iter().map(|s| s.option).max().unwrap();
        let options_count = (max_option as u16 + 1).max(2) as u8;
        let oracle = self.create_real_oracle(options_count, TWAP_WINDOW);
        for spec in specs {
            self.propose_real(oracle, spec.option, spec.bond);
        }
        let res = self.finalize_proposals_real(oracle);
        assert!(res.is_ok(), "finalize_proposals should succeed: {res:?}");
        assert_eq!(
            self.oracle(oracle).phase,
            Phase::FactProposal.as_u8(),
            "dispute_via_real_flow needs >=2 distinct options to open a dispute"
        );
        oracle
    }
}
