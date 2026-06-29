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
//! # AMM binding (DEFERRED, documented)
//! The standalone MetaDAO `amm` v0.4 (`AMMyu…`) was migrated out of the current
//! futarchy source tree (to Meteora DAMM v2), so its account layout is not
//! re-derivable from `main`. We therefore verify only that `pass_amm`/`fail_amm`
//! are **owned by the AMM program** (the strongest binding we can make without a
//! reliable layout). Binding each AMM to its specific pass/fail conditional KASS
//! and USDC mints is left to Task 11, where the TWAP read accounts pin them.
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
//!
//! # Instruction payload (after the 1-byte discriminant)
//! `challenger_usdc: u64 LE` ++ `oracle_nonce: u64 LE` (exactly 16 bytes).

use bytemuck::Zeroable;
use pinocchio::{
    account_info::AccountInfo,
    instruction::{AccountMeta, Seed, Signer},
    program_error::ProgramError,
    pubkey::{find_program_address, Pubkey},
    sysvars::{rent::Rent, Sysvar},
    ProgramResult,
};

use crate::{
    clock::{now, require_before_end, require_phase},
    cpi::metadao,
    error::KassandraError,
    processor::guards::{
        assert_key, assert_owned_by_program, assert_signer, create_pda, load_ai_claim, load_oracle,
        load_proposer,
    },
    state::{AccountType, Market, Oracle, Phase},
};

/// Exact payload length: challenger_usdc[8] ++ oracle_nonce[8].
const PAYLOAD_LEN: usize = 16;

/// Read a 32-byte pubkey out of `data` at byte `off`, or `InvalidAccount`.
fn read_pubkey(data: &[u8], off: usize) -> Result<Pubkey, ProgramError> {
    data.get(off..off + 32)
        .and_then(|s| s.try_into().ok())
        .ok_or_else(|| KassandraError::InvalidAccount.into())
}

/// Read a little-endian `u32` out of `data` at byte `off`, or `InvalidAccount`.
fn read_u32(data: &[u8], off: usize) -> Result<u32, ProgramError> {
    data.get(off..off + 4)
        .and_then(|s| s.try_into().ok())
        .map(u32::from_le_bytes)
        .ok_or_else(|| KassandraError::InvalidAccount.into())
}

/// Minimum size of an SPL token account (`spl_token::state::Account::LEN`).
const SPL_TOKEN_ACCOUNT_LEN: usize = 165;
/// `spl_token::state::Account.mint` byte offset.
const SPL_TOKEN_MINT_OFFSET: usize = 0;
/// `spl_token::state::Account.owner` byte offset.
const SPL_TOKEN_OWNER_OFFSET: usize = 32;

/// Assert `account` is an SPL token account owned (token authority) by
/// `oracle_key` on `expected_mint`, else [`KassandraError::InvalidAccount`].
/// Defense-in-depth on the conditional-KASS split destinations: the
/// conditional_vault enforces the same constraints, but a clean local error is
/// clearer than a downstream MetaDAO custom error and pins the recorded
/// `Market.oracle_{pass,fail}_kass` contract for Task 11.
fn assert_oracle_owned_token(
    account: &AccountInfo,
    expected_mint: &Pubkey,
    oracle_key: &Pubkey,
) -> ProgramResult {
    if !account.is_owned_by(&pinocchio_token::ID) {
        return Err(KassandraError::InvalidAccount.into());
    }
    let data = account.try_borrow_data()?;
    if data.len() < SPL_TOKEN_ACCOUNT_LEN {
        return Err(KassandraError::InvalidAccount.into());
    }
    let mint = read_pubkey(&data, SPL_TOKEN_MINT_OFFSET)?;
    let owner = read_pubkey(&data, SPL_TOKEN_OWNER_OFFSET)?;
    if &mint != expected_mint || &owner != oracle_key {
        return Err(KassandraError::InvalidAccount.into());
    }
    Ok(())
}

pub fn process(program_id: &Pubkey, accounts: &[AccountInfo], payload: &[u8]) -> ProgramResult {
    if payload.len() != PAYLOAD_LEN {
        return Err(ProgramError::InvalidInstructionData);
    }
    let challenger_usdc = u64::from_le_bytes(payload[0..8].try_into().unwrap());
    let oracle_nonce = u64::from_le_bytes(payload[8..16].try_into().unwrap());

    let [oracle_ai, ai_claim_ai, proposer_ai, market_ai, challenger_ai, question_ai, kass_vault_ai, usdc_vault_ai, pass_amm_ai, fail_amm_ai, stake_vault_ai, kass_vault_underlying_ai, pass_mint_ai, fail_mint_ai, oracle_pass_kass_ai, oracle_fail_kass_ai, cv_prog_ai, token_prog_ai, system_prog_ai, cv_event_auth_ai, ..] =
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
    let oracle: Oracle = load_oracle(oracle_ai, program_id)?;
    require_phase(&oracle, Phase::Challenge)?;
    let now = now()?;
    require_before_end(&oracle, now)?;

    // Re-derive the oracle PDA from the supplied nonce and verify it matches
    // the passed oracle account + stored bump (the split signer seeds).
    let (derived_oracle, derived_bump) =
        find_program_address(&[b"oracle", &oracle_nonce.to_le_bytes()], program_id);
    if &derived_oracle != oracle_ai.key() || derived_bump != oracle.bump {
        return Err(KassandraError::InvalidAccount.into());
    }

    // --- claim binding ------------------------------------------------------
    let mut ai_claim = load_ai_claim(ai_claim_ai, program_id)?;
    if ai_claim.oracle != *oracle_ai.key() {
        return Err(KassandraError::InvalidAccount.into());
    }
    if ai_claim.is_challenged() {
        return Err(KassandraError::AlreadyChallenged.into());
    }
    if ai_claim.proposer != *proposer_ai.key() {
        return Err(KassandraError::InvalidAccount.into());
    }

    // --- proposer binding ---------------------------------------------------
    let proposer = load_proposer(proposer_ai, program_id)?;
    if proposer.oracle != *oracle_ai.key() {
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
        let data = question_ai.try_borrow_data()?;
        let q_oracle = read_pubkey(&data, metadao::QUESTION_ORACLE_OFFSET)?;
        if &q_oracle != oracle_ai.key() {
            return Err(KassandraError::InvalidAccount.into());
        }
        let num_outcomes = read_u32(&data, metadao::QUESTION_NUM_OUTCOMES_LEN_OFFSET)?;
        if num_outcomes != 2 {
            return Err(KassandraError::InvalidAccount.into());
        }
    }

    // --- verify the KASS conditional vault ----------------------------------
    assert_owned_by_program(kass_vault_ai, &metadao::CONDITIONAL_VAULT_ID)?;
    {
        let data = kass_vault_ai.try_borrow_data()?;
        let v_question = read_pubkey(&data, metadao::VAULT_QUESTION_OFFSET)?;
        let v_underlying = read_pubkey(&data, metadao::VAULT_UNDERLYING_MINT_OFFSET)?;
        let v_underlying_acct = read_pubkey(&data, metadao::VAULT_UNDERLYING_ACCOUNT_OFFSET)?;
        if &v_question != question_ai.key()
            || v_underlying != oracle.kass_mint
            || &v_underlying_acct != kass_vault_underlying_ai.key()
        {
            return Err(KassandraError::InvalidAccount.into());
        }
    }

    // --- verify the USDC conditional vault ----------------------------------
    assert_owned_by_program(usdc_vault_ai, &metadao::CONDITIONAL_VAULT_ID)?;
    {
        let data = usdc_vault_ai.try_borrow_data()?;
        let v_question = read_pubkey(&data, metadao::VAULT_QUESTION_OFFSET)?;
        let v_underlying = read_pubkey(&data, metadao::VAULT_UNDERLYING_MINT_OFFSET)?;
        if &v_question != question_ai.key() || v_underlying != oracle.usdc_mint {
            return Err(KassandraError::InvalidAccount.into());
        }
    }

    // --- verify the AMMs -----------------------------------------------------
    // DEFERRED-MUST-VERIFY-IN-TASK-11: only owner==AMM_ID is checked here; the
    // standalone v0.4 AMM layout is not re-derivable from current MetaDAO
    // source. settle_challenge MUST verify each AMM is bound to this market's
    // pass/fail conditional (KASS,USDC) mint pair and that pass_amm != fail_amm
    // before reading its TWAP.
    assert_owned_by_program(pass_amm_ai, &metadao::AMM_ID)?;
    assert_owned_by_program(fail_amm_ai, &metadao::AMM_ID)?;

    // --- verify the conditional KASS mints derive from the KASS vault -------
    let (expect_pass_mint, _) = metadao::conditional_token_mint_pda(kass_vault_ai.key(), 0);
    let (expect_fail_mint, _) = metadao::conditional_token_mint_pda(kass_vault_ai.key(), 1);
    assert_key(pass_mint_ai, &expect_pass_mint)?;
    assert_key(fail_mint_ai, &expect_fail_mint)?;

    // --- verify the conditional-KASS split DESTINATIONS (defense-in-depth) --
    // The vault enforces these too, but a clean InvalidAccount here beats a
    // downstream MetaDAO custom error and locks the contract the docstring
    // claims: each dest is an SPL token account owned by the oracle PDA on the
    // matching conditional KASS mint. Task 11 redeems from exactly these.
    assert_oracle_owned_token(oracle_pass_kass_ai, &expect_pass_mint, oracle_ai.key())?;
    assert_oracle_owned_token(oracle_fail_kass_ai, &expect_fail_mint, oracle_ai.key())?;

    // --- market PDA derivation + uninit check -------------------------------
    let (expected_market, market_bump) =
        find_program_address(&[b"market", ai_claim_ai.key().as_ref()], program_id);
    assert_key(market_ai, &expected_market)?;
    if market_ai.lamports() != 0 || !market_ai.data_is_empty() {
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
        AccountMeta::readonly(question_ai.key()),
        AccountMeta::writable(kass_vault_ai.key()),
        AccountMeta::writable(kass_vault_underlying_ai.key()),
        AccountMeta::readonly_signer(oracle_ai.key()), // authority (oracle PDA)
        AccountMeta::writable(stake_vault_ai.key()),   // user_underlying
        AccountMeta::readonly(token_prog_ai.key()),
        AccountMeta::readonly(cv_event_auth_ai.key()),
        AccountMeta::readonly(cv_prog_ai.key()),
        // remaining: mints then user (oracle PDA) conditional token accounts
        AccountMeta::writable(pass_mint_ai.key()),
        AccountMeta::writable(fail_mint_ai.key()),
        AccountMeta::writable(oracle_pass_kass_ai.key()),
        AccountMeta::writable(oracle_fail_kass_ai.key()),
    ];
    let split_infos = [
        question_ai,
        kass_vault_ai,
        kass_vault_underlying_ai,
        oracle_ai,
        stake_vault_ai,
        token_prog_ai,
        cv_event_auth_ai,
        cv_prog_ai,
        pass_mint_ai,
        fail_mint_ai,
        oracle_pass_kass_ai,
        oracle_fail_kass_ai,
    ];
    let nonce_le = oracle_nonce.to_le_bytes();
    let bump_seed = [oracle.bump];
    let oracle_seeds = [
        Seed::from(b"oracle".as_ref()),
        Seed::from(nonce_le.as_ref()),
        Seed::from(&bump_seed),
    ];
    let oracle_signer = Signer::from(&oracle_seeds);
    metadao::invoke_conditional_vault_signed(
        &split_data,
        &split_metas,
        &split_infos,
        &[oracle_signer],
    )?;

    // --- create + populate the Market PDA (challenger pays) -----------------
    let rent = Rent::get()?.minimum_balance(Market::LEN);
    let market_bump_seed = [market_bump];
    let market_seeds = [
        Seed::from(b"market".as_ref()),
        Seed::from(ai_claim_ai.key().as_ref()),
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

    let mut market = Market::zeroed();
    market.account_type = AccountType::Market.as_u8();
    market.oracle = *oracle_ai.key();
    market.ai_claim = *ai_claim_ai.key();
    market.proposer = *proposer_ai.key();
    market.challenger = *challenger_ai.key();
    market.question = *question_ai.key();
    market.kass_vault = *kass_vault_ai.key();
    market.usdc_vault = *usdc_vault_ai.key();
    market.pass_amm = *pass_amm_ai.key();
    market.fail_amm = *fail_amm_ai.key();
    market.oracle_pass_kass = *oracle_pass_kass_ai.key();
    market.oracle_fail_kass = *oracle_fail_kass_ai.key();
    market.twap_end = now
        .checked_add(oracle.twap_window)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    market.challenger_usdc = challenger_usdc;
    market.settled = 0;
    market.bump = market_bump;
    {
        let mut data = market_ai.try_borrow_mut_data()?;
        data.copy_from_slice(bytemuck::bytes_of(&market));
    }

    // --- flip the claim to challenged ---------------------------------------
    ai_claim.challenged = 1;
    {
        let mut data = ai_claim_ai.try_borrow_mut_data()?;
        data[..crate::state::AiClaim::LEN].copy_from_slice(bytemuck::bytes_of(&ai_claim));
    }

    Ok(())
}
