//! `open_challenge`: a challenger opens a MetaDAO decision market against a
//! proposer's [`AiClaim`] during the `Challenge` window.
//!
//! # Decomposed market (design §6)
//! The challenger composes the MetaDAO accounts in their OWN transactions
//! (like the Task 9 tests): a binary `question` whose resolver is the Kassandra
//! oracle PDA (outcome 0 = pass, 1 = fail), a KASS conditional vault, a USDC
//! conditional vault, and the pass/fail AMMs. This instruction does NOT create
//! them — it **verifies** they are bound to this oracle/claim, **records** them
//! in a [`Market`] PDA, performs the **program-signed** split of the proposer's
//! already-escrowed KASS bond into pass-KASS / fail-KASS, and flips
//! `ai_claim.challenged = 1`. AMM liquidity + trading + TWAP settlement are
//! exercised in tests / Task 11.
//!
//! # Dormant by default
//! A [`Market`] account exists ONLY for a challenged claim. Uncontested claims
//! cost nothing (no account, no CPI) — proven by the test that asserts no
//! Market PDA exists without an `open_challenge` call.
//!
//! # Program-signed KASS split
//! The proposer's bond lives in `oracle.stake_vault`, whose SPL authority is
//! the oracle PDA. The split's `user_underlying_token_account` is that vault and
//! its `authority` is the oracle PDA, signed here with the oracle seeds
//! `[b"oracle", nonce_le, [bump]]`. The pass/fail conditional KASS is minted to
//! two program-controlled token accounts **owned by the oracle PDA** (so Task 11
//! can redeem/merge them on settlement). The `nonce` is supplied in the payload
//! and verified by re-deriving the oracle PDA — the Oracle struct does not store
//! it, and adding a field would re-pin the whole ABI for one signer derivation;
//! verifying the derived PDA matches the passed oracle account is equally safe.
//!
//! # MetaDAO account layout offsets (verified against the deployed v0.4.0 source,
//! `declare_id! == VLTX1ishMBbcX3rdBWGssxawAo1Q2X2qxYFYqiGodVg`)
//! * `Question` (8-byte Anchor disc first): `question_id[32]` @8,
//!   `oracle:Pubkey` @40, `payout_numerators: Vec<u32>` len prefix @72. At
//!   `initialize_question` the Vec is `vec![0; num_outcomes]`, so its u32 LE
//!   length == `num_outcomes`.
//! * `ConditionalVault` (disc first): `question:Pubkey` @8,
//!   `underlying_token_mint:Pubkey` @40, `underlying_token_account:Pubkey` @72,
//!   `conditional_token_mints: Vec<Pubkey>` @104.
//!
//! # AMM binding (enforced HERE, at open)
//! Each `pass_amm`/`fail_amm` is fully bound before it is recorded on the
//! `Market` (via [`metadao::assert_amm_bound`]): owned by the AMM program,
//! carrying the `Amm` account discriminator, and whose `base_mint`/`quote_mint`
//! equal this market's pass/fail conditional (KASS, USDC) mints for that outcome;
//! and `pass_amm != fail_amm`. `settle_challenge` re-checks the SAME binding
//! before reading each TWAP. Binding at open is load-bearing: settle pins each
//! AMM to the address recorded here, so a market recorded with an unbindable AMM
//! could never settle — `open_challenge_count` would never return to 0,
//! `finalize_oracle` would be blocked forever, and every stake in the oracle
//! would be permanently locked.
//!
//! # Accounts
//! 0.  oracle              — writable, owned by this program; also the split
//!     authority (signs via the oracle PDA seeds)
//! 1.  ai_claim            — writable, the challenged claim
//! 2.  proposer            — writable, the claim's proposer (source of the bond)
//! 3.  market PDA          — writable, uninitialized (created here)
//! 4.  challenger          — signer, writable; pays the Market rent
//! 5.  question            — read-only MetaDAO question (resolver == oracle PDA)
//! 6.  kass_vault          — writable MetaDAO conditional vault (underlying KASS)
//! 7.  usdc_vault          — read-only MetaDAO conditional vault (underlying USDC)
//! 8.  pass_amm            — read-only, owned by the AMM program
//! 9.  fail_amm            — read-only, owned by the AMM program
//! 10. stake_vault         — writable; == `oracle.stake_vault` (split source)
//! 11. kass_vault_underlying_ata — writable; == kass_vault.underlying_token_account
//! 12. pass_kass_mint      — writable; conditional-token mint idx 0 of kass_vault
//! 13. fail_kass_mint      — writable; conditional-token mint idx 1 of kass_vault
//! 14. oracle_pass_kass    — writable; dest conditional-token acct, owner == oracle PDA
//! 15. oracle_fail_kass    — writable; dest conditional-token acct, owner == oracle PDA
//! 16. conditional_vault program
//! 17. token program
//! 18. system program
//! 19. cv_event_authority  — read-only; conditional_vault `#[event_cpi]` authority
//! 20. protocol            — read-only; the `[b"protocol"]` singleton (`kass_dao` source)
//! 21. kass_dao            — read-only; the futarchy `Dao` (== `protocol.kass_dao`), kass_price source
//! 22. usdc_mint           — read-only; == `oracle.usdc_mint` (escrow vault mint)
//! 23. challenger_usdc_src — writable; challenger's USDC source token account (challenger signs)
//! 24. challenger_usdc_vault — writable, uninit; market-owned USDC escrow created here
//!     at PDA `[b"challenge_usdc", market]`, token authority = oracle PDA
//!
//! # Challenger USDC escrow (Task C1)
//! The escrow is sized via `kass_price` (the governance-anchored futarchy spot
//! TWAP, raw USDC per raw KASS × `1e12`): `required_usdc = bond × twap /
//! KASS_PRICE_SCALE` (u128, overflow-checked), where `bond == proposer.bond`.
//! The cross-decimal (KASS 9dp / USDC 6dp) adjustment is folded into the raw
//! price, so no extra `10^Δdecimals` factor is needed (see
//! [`crate::config::KASS_PRICE_SCALE`]). The amount is computed ON-CHAIN and
//! transferred challenger→escrow; the legacy payload `challenger_usdc` field is
//! gone (it was never trustworthy).
//!
//! # Instruction payload (after the 1-byte discriminant)
//! `oracle_nonce: u64 LE` (exactly 8 bytes).

use bytemuck::Zeroable;
use pinocchio::{
    account::AccountView as AccountInfo,
    address::Address as Pubkey,
    cpi::{Seed, Signer},
    error::ProgramError,
    instruction::InstructionAccount,
    ProgramResult,
};
use pinocchio_token::instructions::{InitializeAccount3, Transfer};
use pinocchio_token::state::Account as TokenAccount;

use crate::{
    clock::{now, require_before_end, require_phase},
    config::KASS_PRICE_SCALE,
    cpi::metadao,
    error::KassandraError,
    price::kass_price,
    processor::guards::{
        assert_key, assert_owned_by_program, assert_signer, assert_token_account, create_pda,
        load_ai_claim, load_oracle, load_proposer, load_protocol, verify_oracle_pda,
    },
    rent::minimum_rent,
    state::{AccountType, Market, Oracle, Phase},
};

/// Exact payload length: oracle_nonce[8].
const PAYLOAD_LEN: usize = 8;

/// Assert `account` is an SPL token account owned (token authority) by
/// `oracle_key` on `expected_mint`, else [`KassandraError::InvalidAccount`].
/// Defense-in-depth on the conditional-KASS split destinations: the
/// conditional_vault enforces the same constraints, but a clean local error is
/// clearer than a downstream MetaDAO custom error and pins the recorded
/// `Market.oracle_{pass,fail}_kass` contract for Task 11.
pub fn process(program_id: &Pubkey, accounts: &mut [AccountInfo], payload: &[u8]) -> ProgramResult {
    if payload.len() != PAYLOAD_LEN {
        return Err(ProgramError::InvalidInstructionData);
    }
    let oracle_nonce = u64::from_le_bytes(payload[0..8].try_into().unwrap());

    let [oracle_ai, ai_claim_ai, proposer_ai, market_ai, challenger_ai, question_ai, kass_vault_ai, usdc_vault_ai, pass_amm_ai, fail_amm_ai, stake_vault_ai, kass_vault_underlying_ai, pass_mint_ai, fail_mint_ai, oracle_pass_kass_ai, oracle_fail_kass_ai, cv_prog_ai, token_prog_ai, system_prog_ai, cv_event_auth_ai, protocol_ai, kass_dao_ai, usdc_mint_ai, challenger_usdc_src_ai, escrow_vault_ai, ..] =
        accounts
    else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };

    // --- signer + program ids ----------------------------------------------
    assert_signer(challenger_ai)?;
    assert_key(cv_prog_ai, &metadao::CONDITIONAL_VAULT_ID)?;
    assert_key(token_prog_ai, &pinocchio_token::ID)?;
    assert_key(system_prog_ai, &pinocchio_system::ID)?;

    // --- oracle + phase / window gates -------------------------------------
    let mut oracle: Oracle = load_oracle(oracle_ai, program_id)?;
    require_phase(&oracle, Phase::Challenge)?;
    let now = now()?;
    require_before_end(&oracle, now)?;

    // The oracle PDA (whose seeds sign the bond split below) must match the
    // passed account.
    verify_oracle_pda(program_id, oracle_ai, &oracle, oracle_nonce)?;

    // --- claim binding ------------------------------------------------------
    let mut ai_claim = load_ai_claim(ai_claim_ai, program_id)?;
    if ai_claim.oracle != *oracle_ai.address() {
        return Err(KassandraError::InvalidAccount.into());
    }
    if ai_claim.is_challenged() {
        return Err(KassandraError::AlreadyChallenged.into());
    }
    if ai_claim.proposer != *proposer_ai.address() {
        return Err(KassandraError::InvalidAccount.into());
    }

    // --- proposer binding ---------------------------------------------------
    let proposer = load_proposer(proposer_ai, program_id)?;
    if proposer.oracle != *oracle_ai.address() {
        return Err(KassandraError::InvalidAccount.into());
    }
    // A disqualified proposer's claim is already out — nothing to challenge.
    if proposer.is_disqualified() {
        return Err(KassandraError::Unauthorized.into());
    }

    // --- stake vault --------------------------------------------------------
    assert_key(stake_vault_ai, &oracle.stake_vault)?;

    // --- verify the MetaDAO question binds to THIS oracle -------------------
    // (Offsets are the single-source-of-truth consts in `cpi::metadao`.)
    assert_owned_by_program(question_ai, &metadao::CONDITIONAL_VAULT_ID)?;
    {
        let data = question_ai.try_borrow()?;
        let q_oracle = metadao::read_pubkey(&data, metadao::QUESTION_ORACLE_OFFSET)?;
        if &q_oracle != oracle_ai.address() {
            return Err(KassandraError::InvalidAccount.into());
        }
        let num_outcomes = metadao::read_u32(&data, metadao::QUESTION_NUM_OUTCOMES_LEN_OFFSET)?;
        if num_outcomes != 2 {
            return Err(KassandraError::InvalidAccount.into());
        }
    }

    // --- verify the KASS conditional vault ----------------------------------
    assert_owned_by_program(kass_vault_ai, &metadao::CONDITIONAL_VAULT_ID)?;
    {
        let data = kass_vault_ai.try_borrow()?;
        let v_question = metadao::read_pubkey(&data, metadao::VAULT_QUESTION_OFFSET)?;
        let v_underlying = metadao::read_pubkey(&data, metadao::VAULT_UNDERLYING_MINT_OFFSET)?;
        let v_underlying_acct =
            metadao::read_pubkey(&data, metadao::VAULT_UNDERLYING_ACCOUNT_OFFSET)?;
        if &v_question != question_ai.address()
            || v_underlying != oracle.kass_mint
            || &v_underlying_acct != kass_vault_underlying_ai.address()
        {
            return Err(KassandraError::InvalidAccount.into());
        }
    }

    // --- verify the USDC conditional vault ----------------------------------
    assert_owned_by_program(usdc_vault_ai, &metadao::CONDITIONAL_VAULT_ID)?;
    {
        let data = usdc_vault_ai.try_borrow()?;
        let v_question = metadao::read_pubkey(&data, metadao::VAULT_QUESTION_OFFSET)?;
        let v_underlying = metadao::read_pubkey(&data, metadao::VAULT_UNDERLYING_MINT_OFFSET)?;
        if &v_question != question_ai.address() || v_underlying != oracle.usdc_mint {
            return Err(KassandraError::InvalidAccount.into());
        }
    }

    // --- verify the conditional KASS mints derive from the KASS vault -------
    let (expect_pass_mint, _) = metadao::conditional_token_mint_pda(kass_vault_ai.address(), 0);
    let (expect_fail_mint, _) = metadao::conditional_token_mint_pda(kass_vault_ai.address(), 1);
    assert_key(pass_mint_ai, &expect_pass_mint)?;
    assert_key(fail_mint_ai, &expect_fail_mint)?;

    // --- bind the pass/fail AMMs NOW (owner + `Amm` disc + exact conditional
    //     (KASS,USDC) mint pair per outcome), and require pass_amm != fail_amm.
    // This MUST happen at open, not only at settle: settle pins each AMM to the
    // address RECORDED here, so a market recorded with an unbindable AMM (wrong
    // mints, or the same account twice) could never settle. That would leave
    // `open_challenge_count > 0` forever, blocking `finalize_oracle` and locking
    // every stake in the oracle permanently. (Same binding `settle_challenge`
    // re-checks before reading each TWAP.)
    let (expect_pass_usdc, _) = metadao::conditional_token_mint_pda(usdc_vault_ai.address(), 0);
    let (expect_fail_usdc, _) = metadao::conditional_token_mint_pda(usdc_vault_ai.address(), 1);
    metadao::assert_amm_bound(pass_amm_ai, &expect_pass_mint, &expect_pass_usdc)?;
    metadao::assert_amm_bound(fail_amm_ai, &expect_fail_mint, &expect_fail_usdc)?;
    if pass_amm_ai.address() == fail_amm_ai.address() {
        return Err(KassandraError::InvalidAccount.into());
    }

    // --- verify the conditional-KASS split DESTINATIONS (defense-in-depth) --
    // The vault enforces these too, but a clean InvalidAccount here beats a
    // downstream MetaDAO custom error and locks the contract the docstring
    // claims: each dest is an SPL token account owned by the oracle PDA on the
    // matching conditional KASS mint. Task 11 redeems from exactly these.
    assert_token_account(oracle_pass_kass_ai, &expect_pass_mint, oracle_ai.address())?;
    assert_token_account(oracle_fail_kass_ai, &expect_fail_mint, oracle_ai.address())?;

    // --- market PDA derivation + uninit check -------------------------------
    let (expected_market, market_bump) =
        Pubkey::find_program_address(&[b"market", ai_claim_ai.address().as_ref()], program_id);
    assert_key(market_ai, &expected_market)?;
    if market_ai.lamports() != 0 || !market_ai.is_data_empty() {
        return Err(KassandraError::AlreadyChallenged.into());
    }

    // --- program-signed KASS split (oracle PDA authority) -------------------
    // Move proposer.bond KASS from oracle.stake_vault into the KASS conditional
    // vault, minting pass-KASS/fail-KASS to the oracle-PDA-owned destinations.
    // NOTE: `oracle.total_oracle_stake` is intentionally NOT decremented — the
    // KASS is still in-system, now escrowed in the conditional vault recorded on
    // the Market (Task 13 conservation counts it there).
    let (cv_event_auth, _) = metadao::event_authority_pda(&metadao::CONDITIONAL_VAULT_ID);
    assert_key(cv_event_auth_ai, &cv_event_auth)?;

    let split_data = metadao::split_tokens_data(proposer.bond);
    let split_metas = [
        InstructionAccount::readonly(question_ai.address()),
        InstructionAccount::writable(kass_vault_ai.address()),
        InstructionAccount::writable(kass_vault_underlying_ai.address()),
        InstructionAccount::readonly_signer(oracle_ai.address()), // authority (oracle PDA)
        InstructionAccount::writable(stake_vault_ai.address()),   // user_underlying
        InstructionAccount::readonly(token_prog_ai.address()),
        InstructionAccount::readonly(cv_event_auth_ai.address()),
        InstructionAccount::readonly(cv_prog_ai.address()),
        // remaining: mints then user (oracle PDA) conditional token accounts
        InstructionAccount::writable(pass_mint_ai.address()),
        InstructionAccount::writable(fail_mint_ai.address()),
        InstructionAccount::writable(oracle_pass_kass_ai.address()),
        InstructionAccount::writable(oracle_fail_kass_ai.address()),
    ];
    let split_infos = [
        &*question_ai,
        &*kass_vault_ai,
        &*kass_vault_underlying_ai,
        &*oracle_ai,
        &*stake_vault_ai,
        &*token_prog_ai,
        &*cv_event_auth_ai,
        &*cv_prog_ai,
        &*pass_mint_ai,
        &*fail_mint_ai,
        &*oracle_pass_kass_ai,
        &*oracle_fail_kass_ai,
    ];
    let nonce_le = oracle_nonce.to_le_bytes();
    let bump_seed = [oracle.bump];
    let oracle_seeds = Oracle::signer_seeds(&nonce_le, &bump_seed);
    let oracle_signer = Signer::from(&oracle_seeds);
    metadao::invoke_conditional_vault_signed(
        &split_data,
        &split_metas,
        &split_infos,
        &[oracle_signer],
    )?;

    // --- create + populate the Market PDA (challenger pays) -----------------
    let rent = minimum_rent(Market::LEN)?;
    let market_bump_seed = [market_bump];
    let market_seeds = [
        Seed::from(b"market".as_ref()),
        Seed::from(ai_claim_ai.address().as_ref()),
        Seed::from(&market_bump_seed),
    ];
    create_pda(
        challenger_ai,
        market_ai,
        &market_seeds,
        rent,
        Market::LEN,
        program_id,
    )?;

    // --- size the challenger USDC escrow via kass_price (Task C1) -----------
    // All MetaDAO market bindings are verified above; now price the escrow. The
    // escrow vault's mint must be the oracle's canonical USDC mint. `kass_price`
    // asserts `protocol` is the `[b"protocol"]` singleton (load_protocol's
    // address pin), `kass_dao == protocol.kass_dao`, and the futarchy-program
    // ownership of `kass_dao`. The returned TWAP is raw USDC per raw KASS ×
    // KASS_PRICE_SCALE, so the cross-decimal (KASS 9dp / USDC 6dp) adjustment is
    // folded in: required_usdc = bond × twap / KASS_PRICE_SCALE (u128 intermediate,
    // overflow-checked back into u64).
    // POOL-ORIENTATION ASSUMPTION (load-bearing): `kass_price` reads the BLESSED
    // futarchy `kass_dao` spot pool, which is KASS-base / USDC-quote, so its TWAP
    // is `quote-per-base = raw-USDC per raw-KASS × KASS_PRICE_SCALE`. That is
    // exactly the "price of one KASS in USDC" we need to value a KASS bond in
    // USDC; if the pool were inverted (USDC-base/KASS-quote) this product would be
    // the reciprocal and the escrow would be nonsensical. The orientation is fixed
    // by `Protocol.kass_dao` (set once at governance handoff), so this holds for
    // every challenge under that protocol.
    assert_key(usdc_mint_ai, &oracle.usdc_mint)?;
    let protocol = load_protocol(protocol_ai, program_id)?;
    let twap = kass_price(&protocol, kass_dao_ai)?;
    let required_usdc = u64::try_from(
        (proposer.bond as u128)
            .checked_mul(twap)
            .ok_or(ProgramError::ArithmeticOverflow)?
            / KASS_PRICE_SCALE,
    )
    .map_err(|_| ProgramError::ArithmeticOverflow)?;
    // A zero escrow means the challenger stakes nothing (sub-micro KASS valuation
    // truncated to 0, or a zero bond). Reject: a challenge must put real USDC
    // skin-in-the-game, and a zero-escrow market has no source for the directional
    // USDC fee at settle. NOTE the truncation is DOWNWARD (`× twap / SCALE` floors),
    // so a funded escrow can be ≤ the exact fair value by < 1 USDC base unit —
    // settle's USDC conservation accounts for the escrow as recorded, not the ideal.
    if required_usdc == 0 {
        return Err(KassandraError::ZeroStake.into());
    }

    // --- create + fund the challenger USDC escrow vault (market-owned) ------
    // Bare SPL token account at PDA `[b"challenge_usdc", market]`, initialized
    // on the USDC mint with the oracle PDA as token authority (mirrors how
    // create_oracle stands up `stake_vault`), then funded by the challenger's
    // signed Transfer. An under-funded challenger's source account makes the
    // SPL Transfer fail, rejecting the whole instruction.
    // KNOWN LIMITATION (deferred, same mechanism as propose/submit_fact's PDA
    // creation): an attacker could grief by pre-funding this predicted escrow PDA
    // with 1 lamport so the `create_pda` CreateAccount fails. It is narrow — the
    // PDA is keyed by `market`, which is itself keyed by `ai_claim`, so it can
    // only block one specific, already-known challenge. The future fix is system
    // Allocate + Assign (tolerates a pre-funded account); not worth it now.
    let (expected_escrow, escrow_bump) = Pubkey::find_program_address(
        &[b"challenge_usdc", market_ai.address().as_ref()],
        program_id,
    );
    assert_key(escrow_vault_ai, &expected_escrow)?;
    let escrow_rent = minimum_rent(TokenAccount::LEN)?;
    let escrow_bump_seed = [escrow_bump];
    let escrow_seeds = [
        Seed::from(b"challenge_usdc".as_ref()),
        Seed::from(market_ai.address().as_ref()),
        Seed::from(&escrow_bump_seed),
    ];
    create_pda(
        challenger_ai,
        escrow_vault_ai,
        &escrow_seeds,
        escrow_rent,
        TokenAccount::LEN,
        &pinocchio_token::ID,
    )?;
    InitializeAccount3 {
        account: escrow_vault_ai,
        mint: usdc_mint_ai,
        owner: oracle_ai.address(),
    }
    .invoke()?;
    Transfer::new(
        challenger_usdc_src_ai,
        escrow_vault_ai,
        challenger_ai,
        required_usdc,
    )
    .invoke()?;

    let mut market = Market::zeroed();
    market.account_type = AccountType::Market.as_u8();
    market.oracle = *oracle_ai.address();
    market.ai_claim = *ai_claim_ai.address();
    market.proposer = *proposer_ai.address();
    market.challenger = *challenger_ai.address();
    market.question = *question_ai.address();
    market.kass_vault = *kass_vault_ai.address();
    market.usdc_vault = *usdc_vault_ai.address();
    market.pass_amm = *pass_amm_ai.address();
    market.fail_amm = *fail_amm_ai.address();
    market.oracle_pass_kass = *oracle_pass_kass_ai.address();
    market.oracle_fail_kass = *oracle_fail_kass_ai.address();
    market.challenger_usdc_vault = *escrow_vault_ai.address();
    market.twap_end = now
        .checked_add(oracle.twap_window)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    market.challenger_usdc = required_usdc;
    market.settled = 0;
    market.bump = market_bump;
    {
        let mut data = market_ai.try_borrow_mut()?;
        data.copy_from_slice(bytemuck::bytes_of(&market));
    }

    // --- flip the claim to challenged ---------------------------------------
    ai_claim.challenged = 1;
    {
        let mut data = ai_claim_ai.try_borrow_mut()?;
        data[..crate::state::AiClaim::LEN].copy_from_slice(bytemuck::bytes_of(&ai_claim));
    }

    // --- track the open challenge -------------------------------------------
    // One more market is now OPEN (not yet settled). `settle_challenge`
    // decrements this; Task 12 requires it == 0 before final plurality recompute
    // so an unsettled challenged proposer is never counted as surviving.
    oracle.open_challenge_count = oracle
        .open_challenge_count
        .checked_add(1)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    {
        let mut data = oracle_ai.try_borrow_mut()?;
        data[..Oracle::LEN].copy_from_slice(bytemuck::bytes_of(&oracle));
    }

    Ok(())
}
