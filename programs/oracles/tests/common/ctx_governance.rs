use super::*;

impl TestCtx {
    // ----- real instruction helpers -----------------------------------------

    /// Send a real `InitProtocol` instruction with `admin == payer`, recording
    /// the harness KASS/USDC mints. Returns the Protocol singleton PDA. The
    /// returned [`TransactionResult`] lets tests assert success or the
    /// double-init / wrong-PDA failure paths.
    #[allow(clippy::result_large_err)]
    pub fn init_protocol(&mut self) -> (Pubkey, TransactionResult) {
        let (protocol_pda, _) = Self::protocol_pda(&self.program_id);
        let ix = self.init_protocol_ix(protocol_pda);
        let res = self.send(ix, &[]);
        (protocol_pda, res)
    }

    /// Build an `InitProtocol` instruction targeting `protocol` (so tests can
    /// pass a deliberately wrong PDA). Admin = payer (fee payer signs).
    pub fn init_protocol_ix(&self, protocol: Pubkey) -> Instruction {
        kassandra_oracles_sdk::ix::init_protocol(
            &self.program_id,
            protocol,
            self.payer.pubkey(),
            self.kass_mint,
            self.usdc_mint,
        )
    }

    /// Stand-in governance keys for F1 tests: an arbitrary "Squads vault" PDA
    /// and an arbitrary "futarchy Dao" pubkey. F1's `set_governance` stores
    /// whatever is passed (the real Squads/futarchy setup is F6), so any
    /// distinct non-zero pubkeys suffice. Deterministic per call site via the
    /// supplied `tag`.
    pub fn stand_in_governance(tag: u8) -> (Pubkey, Pubkey) {
        let dao_authority = Pubkey::new_from_array([tag; 32]);
        let kass_dao = Pubkey::new_from_array([tag.wrapping_add(1).max(1); 32]);
        (dao_authority, kass_dao)
    }

    /// Send a real `SetGovernance` instruction signed by `authority`, recording
    /// `dao_authority` (Squads vault) + `kass_dao` (futarchy Dao) in the
    /// Protocol. Returns the Protocol PDA + result so tests can assert success
    /// or the authorization/one-shot rejection paths.
    #[allow(clippy::result_large_err)]
    pub fn set_governance(
        &mut self,
        authority: &Keypair,
        dao_authority: Pubkey,
        kass_dao: Pubkey,
    ) -> (Pubkey, TransactionResult) {
        let (protocol_pda, _) = Self::protocol_pda(&self.program_id);
        let ix = self.set_governance_ix(protocol_pda, authority.pubkey(), dao_authority, kass_dao);
        // The payer is always a signer; only co-sign `authority` when it differs.
        let res = if authority.pubkey() == self.payer.pubkey() {
            self.send(ix, &[])
        } else {
            self.send(ix, &[authority])
        };
        (protocol_pda, res)
    }

    /// Build a `SetGovernance` instruction. Exposes `protocol`/`authority` so
    /// tests can pass a wrong signer. Account order (Task G1):
    /// `[0] protocol(w) [1] authority(signer) [2] kass_dao(ro)`. Payload =
    /// `dao_authority ++ kass_dao`. The `kass_dao` ACCOUNT is the same pubkey as
    /// the payload `kass_dao` (the hardened processor asserts they match).
    pub fn set_governance_ix(
        &self,
        protocol: Pubkey,
        authority: Pubkey,
        dao_authority: Pubkey,
        kass_dao: Pubkey,
    ) -> Instruction {
        kassandra_oracles_sdk::ix::set_governance(
            &self.program_id,
            protocol,
            authority,
            dao_authority,
            kass_dao,
        )
    }

    /// Derive the Squads v4 multisig **vault** PDA (the DAO execution authority)
    /// for a futarchy `Dao` pubkey, via the documented seed builders in
    /// [`md6`] and the real Squads v4 program id: the multisig's `create_key`
    /// IS the `Dao` (`[b"multisig", b"multisig", dao]`), then the vault at index
    /// 0 (`[b"multisig", multisig, b"vault", [0]]`). This is the value the
    /// hardened `set_governance` (Task G1) requires as `dao_authority`.
    pub fn squads_vault_for_dao(dao: &Pubkey) -> Pubkey {
        let squads_id = Pubkey::new_from_array(md6::SQUADS_V4_ID.to_bytes());
        let dao_arr = dao.to_bytes();
        let (multisig, _) =
            Pubkey::find_program_address(&md6::squads_multisig_seeds(&dao_arr.into()), &squads_id);
        let multisig_arr = multisig.to_bytes();
        let (vault, _) = Pubkey::find_program_address(
            &md6::squads_vault_seeds(&multisig_arr.into(), &[0u8]),
            &squads_id,
        );
        vault
    }

    /// Fabricate a real futarchy-owned `Dao` account (valid Anchor
    /// discriminator) at a fresh key and return `(kass_dao, derived vault PDA)`.
    /// The returned vault is exactly what the hardened `set_governance` requires
    /// as `dao_authority`, so `ctx.set_governance(&admin, vault, kass_dao)`
    /// records the REAL linkage and succeeds. The embedded TWAP fields are valid
    /// but arbitrary (these accept-path tests don't read the price).
    pub fn fabricate_dao_and_vault(&mut self) -> (Pubkey, Pubkey) {
        let kass_dao = Pubkey::new_unique();
        let owner = Pubkey::new_from_array(md6::FUTARCHY_ID.to_bytes());
        self.fabricate_owned_account(kass_dao, owner, build_dao_blob(1, 1_000_000, 0, 0));
        let vault = Self::squads_vault_for_dao(&kass_dao);
        (kass_dao, vault)
    }

    /// Directly write the DAO linkage into the `Protocol` singleton, BYPASSING
    /// the (Task G1-hardened) `set_governance` instruction. The gating tests for
    /// `set_config`/`resolve_deadend`/emissions need an ARBITRARY, SIGNABLE
    /// keypair recorded as `dao_authority` to exercise the accept path — which is
    /// impossible through the real handoff, since that now requires
    /// `dao_authority == squads_vault_for_dao(kass_dao)` (a PDA no keypair can
    /// sign). This mirrors the harness's existing direct account-seeding
    /// philosophy (see [`TestCtx::seed_disputed_oracle`]). Marks
    /// `governance_set = 1`. Requires the protocol to already exist.
    pub fn force_governance(&mut self, dao_authority: Pubkey, kass_dao: Pubkey) -> Pubkey {
        let (protocol_pda, _) = Self::protocol_pda(&self.program_id);
        let mut p = self.protocol(protocol_pda);
        p.dao_authority = dao_authority.to_bytes().into();
        p.kass_dao = kass_dao.to_bytes().into();
        p.governance_set = 1;
        self.set_program_account(protocol_pda, bytemuck::bytes_of(&p).to_vec());
        protocol_pda
    }

    /// Send a real `SetConfig` instruction signed by `authority`, overwriting
    /// the `Protocol`-resident governable params with `params`. Returns the
    /// Protocol PDA + result so tests can assert success / the
    /// `Unauthorized` / `InvalidConfig` rejection paths. `set_governance` must
    /// have recorded `authority` as the `dao_authority` first.
    #[allow(clippy::result_large_err)]
    pub fn set_config(
        &mut self,
        authority: &Keypair,
        params: ConfigParams,
    ) -> (Pubkey, TransactionResult) {
        let (protocol_pda, _) = Self::protocol_pda(&self.program_id);
        let ix = self.set_config_ix(protocol_pda, authority.pubkey(), params);
        let res = if authority.pubkey() == self.payer.pubkey() {
            self.send(ix, &[])
        } else {
            self.send(ix, &[authority])
        };
        (protocol_pda, res)
    }

    /// Build a `SetConfig` instruction. Exposes `protocol`/`authority` so tests
    /// can pass a wrong signer. Payload = the 144-byte packed `ConfigParams`.
    pub fn set_config_ix(
        &self,
        protocol: Pubkey,
        authority: Pubkey,
        params: ConfigParams,
    ) -> Instruction {
        kassandra_oracles_sdk::ix::set_config(&self.program_id, protocol, authority, &params)
    }

    /// Send a real `ResolveDeadend` instruction signed by `authority`, setting
    /// `option` as the final outcome of a dead-ended `oracle`. Returns the
    /// Protocol PDA + result so tests can assert success / the `Unauthorized` /
    /// `WrongPhase` / `InvalidOptionsCount` rejection paths. `set_governance`
    /// must have recorded `authority` as the `dao_authority` first.
    #[allow(clippy::result_large_err)]
    pub fn resolve_deadend(
        &mut self,
        oracle: Pubkey,
        authority: &Keypair,
        option: u8,
    ) -> (Pubkey, TransactionResult) {
        let (protocol_pda, _) = Self::protocol_pda(&self.program_id);
        let ix = self.resolve_deadend_ix(protocol_pda, oracle, authority.pubkey(), option);
        let res = if authority.pubkey() == self.payer.pubkey() {
            self.send(ix, &[])
        } else {
            self.send(ix, &[authority])
        };
        (protocol_pda, res)
    }

    /// Build a `ResolveDeadend` instruction. Exposes `protocol`/`oracle`/
    /// `authority` so tests can pass a wrong signer or a substituted protocol.
    /// Account order: `[0] protocol(ro)`, `[1] oracle(w)`, `[2] authority(signer)`.
    /// Payload = the single `option` byte.
    pub fn resolve_deadend_ix(
        &self,
        protocol: Pubkey,
        oracle: Pubkey,
        authority: Pubkey,
        option: u8,
    ) -> Instruction {
        kassandra_oracles_sdk::ix::resolve_deadend(&self.program_id, protocol, oracle, authority, option)
    }

    /// Build a `KassPrice` instruction (Task F5): reads the futarchy `Dao`
    /// account's spot TWAP. Account order: `[0] protocol(ro)`, `[1] kass_dao(ro)`.
    /// Exposes both accounts so tests can pass a substituted protocol or a
    /// wrong/foreign-owned `kass_dao`. No payload.
    pub fn kass_price_ix(&self, protocol: Pubkey, kass_dao: Pubkey) -> Instruction {
        kassandra_oracles_sdk::ix::kass_price(&self.program_id, protocol, kass_dao)
    }

    /// Fabricate an account at `key` owned by `owner` holding `data`. Used by F5
    /// to stand up a futarchy-owned `Dao` account carrying a hand-built spot
    /// `TwapOracle` (and the wrong-owner negative case).
    pub fn fabricate_owned_account(&mut self, key: Pubkey, owner: Pubkey, data: Vec<u8>) {
        let lamports = self.svm.minimum_balance_for_rent_exemption(data.len());
        self.svm
            .set_account(
                key,
                Account {
                    lamports,
                    data,
                    owner,
                    executable: false,
                    rent_epoch: 0,
                },
            )
            .unwrap();
    }

    /// Ensure the Protocol singleton exists and hand governance off with a
    /// hand-built futarchy `Dao` account whose embedded spot TWAP equals
    /// [`KASS_PRICE_TWAP`], recorded as `Protocol.kass_dao`. This makes
    /// `open_challenge`'s `kass_price` read return a deterministic value so the
    /// escrow size ([`required_escrow_usdc`]) is computable. Returns the
    /// `kass_dao` account key. One-shot per `TestCtx` (set_governance is
    /// one-shot).
    pub fn bless_kass_price(&mut self) -> Pubkey {
        self.ensure_protocol();
        let kass_dao = Pubkey::new_unique();
        let owner = Pubkey::new_from_array(md6::FUTARCHY_ID.to_bytes());
        // twap = aggregator / (last_updated - (created_at + start_delay)).
        // Pick a 1_000_000s window so aggregator = twap * 1e6 yields KASS_PRICE_TWAP.
        let last_updated: i64 = 1_000_000;
        let created_at: i64 = 0;
        let start_delay: u32 = 0;
        let aggregator: u128 = KASS_PRICE_TWAP * 1_000_000;
        self.fabricate_owned_account(
            kass_dao,
            owner,
            build_dao_blob(aggregator, last_updated, created_at, start_delay),
        );
        // The kass_price tests only read `kass_dao`; the recorded `dao_authority`
        // is irrelevant to them. Record the linkage DIRECTLY (force_governance)
        // rather than through the Task G1-hardened handoff, which would require a
        // matching derived Squads vault here for no test benefit.
        let (dao_authority, _) = Self::stand_in_governance(0x77);
        self.force_governance(dao_authority, kass_dao);
        kass_dao
    }
}
