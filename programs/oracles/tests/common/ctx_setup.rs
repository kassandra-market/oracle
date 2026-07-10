use super::*;

impl TestCtx {
    /// Build a fresh context: a funded payer plus KASS (9 dp) and USDC (6 dp)
    /// mints, both with the payer as mint authority, and the compiled
    /// `kassandra_oracles_program` deployed so tests can submit real transactions via
    /// [`TestCtx::send`].
    ///
    /// The `.so` is `include_bytes!`'d at compile time, so `just build`
    /// (`cargo build-sbf`) must run **before** `cargo test`.
    pub fn new() -> Self {
        let mut svm = LiteSVM::new();
        let payer = Keypair::new();
        svm.airdrop(&payer.pubkey(), 1_000_000_000_000).unwrap();

        let program_id = Pubkey::new_from_array(kassandra_oracles_program::ID.to_bytes());
        svm.add_program(
            program_id,
            include_bytes!("../../../../target/deploy/kassandra_oracles_program.so"),
        )
        .unwrap();

        let mut ctx = Self {
            svm,
            payer,
            kass_mint: Pubkey::default(),
            usdc_mint: Pubkey::default(),
            payer_kass: Pubkey::default(),
            program_id,
            next_nonce: 0,
            oracles: HashMap::new(),
            protocol_initialized: false,
            cu_meter: CuMeter::default(),
        };

        // Mint-authority bootstrap (Task S3): the KASS mint's authority MUST be
        // the program's mint-authority PDA `[b"mint_authority"]`, so the program
        // (and ONLY the program, via the emission `MintTo` CPI) can mint KASS.
        // This harness fabricates every token balance directly (`create_token_
        // account` writes the balance; `add_mint_supply` rewrites the mint supply
        // field) and burns via the creator (token-account owner) as the SPL Burn
        // authority — NONE of which uses the mint authority — so handing the mint
        // authority to the PDA leaves all existing test funding working while
        // enabling the program's emission mint. USDC's authority stays the payer
        // (no program ever mints USDC).
        let (mint_auth, _) = Self::mint_authority_pda(&ctx.program_id);
        ctx.kass_mint = ctx.create_mint(KASS_DECIMALS, mint_auth);
        ctx.usdc_mint = ctx.create_mint(USDC_DECIMALS, ctx.payer.pubkey());
        // Bankroll the payer with KASS backed by real mint supply so the
        // creation-fee burn reduces both the balance AND the supply.
        let payer = ctx.payer.pubkey();
        ctx.payer_kass = ctx.fund_kass_minted(payer, 1_000_000_000_000_000);
        ctx
    }

    // ----- seed-derivation helpers (thin wrappers over `kassandra_oracles_sdk::pda`) --
    // The seed conventions are the program's public contract; the SDK owns the
    // derivations so there is a single source of truth. These wrappers keep the
    // harness's `*_pda` names stable for the existing test call sites.

    /// Derive the Oracle PDA from a `nonce`: seeds `[b"oracle", nonce_le]`.
    pub fn oracle_pda(program_id: &Pubkey, nonce: u64) -> (Pubkey, u8) {
        kassandra_oracles_sdk::pda::oracle(program_id, nonce)
    }

    /// Derive the Protocol singleton PDA: seeds `[b"protocol"]`.
    pub fn protocol_pda(program_id: &Pubkey) -> (Pubkey, u8) {
        kassandra_oracles_sdk::pda::protocol(program_id)
    }

    /// Derive the KASS mint-authority PDA: seeds `[b"mint_authority"]`.
    pub fn mint_authority_pda(program_id: &Pubkey) -> (Pubkey, u8) {
        kassandra_oracles_sdk::pda::mint_authority(program_id)
    }

    /// Derive the stake-vault PDA for an oracle: seeds `[b"vault", oracle]`.
    pub fn stake_vault_pda(program_id: &Pubkey, oracle: &Pubkey) -> (Pubkey, u8) {
        kassandra_oracles_sdk::pda::stake_vault(program_id, oracle)
    }

    /// Derive the challenger USDC escrow vault PDA: seeds `[b"challenge_usdc", market]`.
    pub fn challenge_usdc_vault_pda(program_id: &Pubkey, market: &Pubkey) -> (Pubkey, u8) {
        kassandra_oracles_sdk::pda::challenge_usdc_vault(program_id, market)
    }

    /// Derive the Proposer PDA: seeds `[b"proposer", oracle, authority]`.
    pub fn proposer_pda(program_id: &Pubkey, oracle: &Pubkey, authority: &Pubkey) -> (Pubkey, u8) {
        kassandra_oracles_sdk::pda::proposer(program_id, oracle, authority)
    }

    /// Derive the Fact PDA: seeds `[b"fact", oracle, content_hash]`.
    pub fn fact_pda(program_id: &Pubkey, oracle: &Pubkey, content_hash: &[u8; 32]) -> (Pubkey, u8) {
        kassandra_oracles_sdk::pda::fact(program_id, oracle, content_hash)
    }

    /// Derive the FactVote PDA: seeds `[b"vote", fact, voter]`.
    pub fn vote_pda(program_id: &Pubkey, fact: &Pubkey, voter: &Pubkey) -> (Pubkey, u8) {
        kassandra_oracles_sdk::pda::vote(program_id, fact, voter)
    }
}
