use super::*;

impl TestCtx {
    /// Fabricate a program-owned account at a fresh address holding `data`.
    /// Used by type-confusion tests to stand up an account with a wrong (or
    /// missing) `account_type` tag.
    pub fn seed_program_account(&mut self, data: Vec<u8>) -> Pubkey {
        let key = Pubkey::new_unique();
        self.set_program_account(key, data);
        key
    }

    /// Fabricate a program-owned account at a SPECIFIC address holding `data`.
    /// Lets tests stand up a PDA-addressed account (e.g. an `AiClaim` at its
    /// `[b"claim", oracle, proposer]` PDA) without the create/submit flow.
    pub fn seed_program_account_at(&mut self, key: Pubkey, data: Vec<u8>) {
        self.set_program_account(key, data);
    }

    /// Create an SPL token account on the KASS mint owned by `owner` and fund
    /// it with `amount` base units of KASS, BACKED by real mint supply. Returns
    /// the token account address. Used to bankroll a fact submitter / voter /
    /// proposer bond source. The supply backing keeps the KASS that flows into a
    /// stake vault physically real, so a terminal InvalidDeadend burn of the
    /// slashed `bond_pool` (which may include rejected-fact stakes + approve-voter
    /// slashes) does not underflow the mint supply. Every emission-calc test
    /// snapshots supply right before its measured `create_oracle`, so this earlier
    /// funding is captured consistently and never skews the emission.
    pub fn fund_kass(&mut self, owner: &Keypair, amount: u64) -> Pubkey {
        let acct = self.create_token_account(self.kass_mint, owner.pubkey(), amount);
        self.add_mint_supply(self.kass_mint, amount);
        acct
    }

    /// Create an SPL token account on the USDC mint owned by `owner` and fund it
    /// with `amount` base units. Returns the token account address. Mirrors
    /// [`TestCtx::fund_kass`]; the challenge-escrow source for `open_challenge`.
    pub fn fund_usdc(&mut self, owner: &Keypair, amount: u64) -> Pubkey {
        self.create_token_account(self.usdc_mint, owner.pubkey(), amount)
    }

    /// Like [`TestCtx::fund_kass`] but ALSO increases the KASS mint's `supply` by
    /// `amount`, so the fabricated balance is backed by real supply. A real
    /// SPL `Burn` checks-subtracts the mint supply, so a burn source that was
    /// only fabricated (supply still 0) would underflow; this keeps them
    /// consistent for the creation-fee burn tests.
    pub fn fund_kass_minted(&mut self, owner: Pubkey, amount: u64) -> Pubkey {
        let acct = self.create_token_account(self.kass_mint, owner, amount);
        self.add_mint_supply(self.kass_mint, amount);
        acct
    }

    /// Read an SPL mint's circulating `supply` (base units).
    pub fn mint_supply(&self, mint: Pubkey) -> u64 {
        let acc = self
            .svm
            .get_account(&mint)
            .unwrap_or_else(|| panic!("mint {mint} not found"));
        Mint::unpack(&acc.data).expect("not a mint").supply
    }

    /// Retrieve the bookkeeping for a previously seeded oracle.
    pub fn seeded(&self, oracle: Pubkey) -> &SeededOracle {
        self.oracles.get(&oracle).expect("oracle not seeded")
    }

    /// Convenience: the seeded proposers for an oracle (spec order).
    pub fn proposers(&self, oracle: Pubkey) -> &[SeededProposer] {
        &self.seeded(oracle).proposers
    }

    /// The `nonce` an oracle was created with (its PDA seed).
    pub fn oracle_nonce(&self, oracle: Pubkey) -> u64 {
        self.seeded(oracle).nonce
    }

    // ----- accessors ---------------------------------------------------------

    /// Read and decode an `Oracle` account.
    pub fn oracle(&self, key: Pubkey) -> Oracle {
        self.read_pod(key)
    }

    /// Read and decode a `Proposer` account.
    pub fn proposer(&self, key: Pubkey) -> Proposer {
        self.read_pod(key)
    }

    /// Read and decode a `Fact` account.
    pub fn fact(&self, key: Pubkey) -> kassandra_oracles_program::state::Fact {
        self.read_pod(key)
    }

    /// Read and decode a `FactVote` account.
    pub fn fact_vote(&self, key: Pubkey) -> kassandra_oracles_program::state::FactVote {
        self.read_pod(key)
    }

    /// Read and decode an `AiClaim` account.
    pub fn ai_claim(&self, key: Pubkey) -> kassandra_oracles_program::state::AiClaim {
        self.read_pod(key)
    }

    /// Read and decode the `Protocol` singleton account.
    pub fn protocol(&self, key: Pubkey) -> kassandra_oracles_program::state::Protocol {
        self.read_pod(key)
    }

    /// Test-only: overwrite an already-created oracle's snapshotted `min_stake`
    /// floor, to exercise the activity-scaled stake gate directly (without driving
    /// the fee-EMA up through many creations).
    pub fn set_oracle_min_stake(&mut self, oracle: Pubkey, min_stake: u64) {
        let mut o = self.oracle(oracle);
        o.min_stake = min_stake;
        self.set_program_account(oracle, bytemuck::bytes_of(&o).to_vec());
    }

    /// Test-only: set the Protocol's stake-floor curve params, so a subsequent
    /// `create_oracle` snapshots a floor derived from the decayed fee-EMA.
    pub fn set_protocol_stake_floor(
        &mut self,
        protocol: Pubkey,
        threshold: u64,
        cap: u64,
        max: u64,
    ) {
        let mut p = self.protocol(protocol);
        p.stake_floor_ema_threshold = threshold;
        p.stake_floor_ema_cap = cap;
        p.stake_floor_max = max;
        self.set_program_account(protocol, bytemuck::bytes_of(&p).to_vec());
    }

    /// Read an SPL token account's `(mint, owner, amount)`, with `mint`/`owner`
    /// as raw 32-byte arrays so callers can compare against `Pubkey::to_bytes()`
    /// without crossing the `solana_program` / `solana_sdk` Pubkey type boundary.
    pub fn token_account(&self, key: Pubkey) -> ([u8; 32], [u8; 32], u64) {
        let acc = self
            .svm
            .get_account(&key)
            .unwrap_or_else(|| panic!("token account {key} not found"));
        let ta = TokenAccount::unpack(&acc.data).expect("not a token account");
        (ta.mint.to_bytes(), ta.owner.to_bytes(), ta.amount)
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

    /// Read a program account and reinterpret its data as a `Pod` struct `T`.
    ///
    /// Uses [`bytemuck::pod_read_unaligned`] so correctness does not depend on
    /// the alignment of the allocator-provided account-data buffer.
    pub fn read_pod<T: bytemuck::Pod>(&self, key: Pubkey) -> T {
        let acc = self
            .svm
            .get_account(&key)
            .unwrap_or_else(|| panic!("account {key} not found"));
        bytemuck::pod_read_unaligned::<T>(&acc.data)
    }
}
