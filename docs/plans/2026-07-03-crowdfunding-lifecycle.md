# Crowdfunding Lifecycle (Phase 1) Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Build the CPI-free core of the `kassandra-market` Pinocchio program — the governed config plus the crowdfunding lifecycle (`init_config`, `update_config`, `create_market`, `contribute`, `cancel`, `refund`) — fully covered by LiteSVM tests, with no MetaDAO integration yet.

**Architecture:** A Pinocchio program mirroring the sibling `kassandra` program's conventions (zero-copy `#[repr(C)]` Pod accounts with a leading `AccountType` tag byte, 1-byte instruction discriminant, one processor per file, `pod_read_unaligned` loaders). A `Market` PDA (one per oracle) accumulates KASS contributions into a program-owned escrow token account during a **Funding** phase; contributors get per-contributor `Contribution` records. If the underlying Kassandra oracle reaches a terminal phase while the market is still under-funded, anyone can `cancel` it and every contributor is refunded in full. Activation into a MetaDAO AMM and settlement are **out of scope** (Phase 2). Instruction building and PDA derivation live in a shared `kassandra-market-sdk` host crate (single source of truth), guarded by a parity test against the program's own constants.

**Tech Stack:** Rust, [Pinocchio](https://github.com/anza-xyz/pinocchio) 0.8, `bytemuck` 1, `pinocchio-token` 0.3, `pinocchio-system` 0.2. Tests: `litesvm` 0.6 + `solana-sdk` 2 + `spl-token` 6. Reads the Kassandra `Oracle` layout via a `no-entrypoint` path dependency on `kassandra-program`.

**Reference:** The design lives in `docs/plans/2026-07-03-kassandra-market-design.md`. All conventions below are copied from the real `../kassandra` program — match them exactly.

---

## Conventions (copy these idioms verbatim)

- Every account struct: `#[repr(C)] #[derive(Clone, Copy, Pod, Zeroable)]`, first field `account_type: u8` then `_pad_hdr: [u8; 7]` (real fields start at offset 8), explicit `_pad` arrays so `size_of` is an 8-byte multiple, `pub const LEN: usize = core::mem::size_of::<Self>();`. Booleans stored as `u8`, read via `!= 0`. `pub type Pubkey = [u8; 32];` local alias.
- Loaders: owned-by-program check → length check → `bytemuck::pod_read_unaligned::<T>(&data[..T::LEN])` → tag byte check. Never a zero-copy cast (buffers may be unaligned).
- State write: `let mut x = T::zeroed(); x.account_type = AccountType::X.as_u8(); …; data.copy_from_slice(bytemuck::bytes_of(&x));`
- PDA creation: `find_program_address` → `assert_key` the passed account → guard `lamports()==0 && data_is_empty()` → `create_pda(payer, pda, &seeds, rent, LEN, owner)` where `rent = Rent::get()?.minimum_balance(LEN)` and the last seed is the bump: `Seed::from(&[bump])`.
- Arithmetic: `checked_add`/`checked_sub().ok_or(ProgramError::ArithmeticOverflow)`. Workspace sets `overflow-checks = true`.
- Errors: custom enum → `ProgramError::Custom(e as u32)`; tests assert via `Some(code)`.

---

## Task 0: Workspace + program scaffold that builds

**Files:**
- Create: `Cargo.toml` (workspace root)
- Create: `rust-toolchain.toml`, `justfile`
- Create: `programs/kassandra-market/Cargo.toml`
- Create: `programs/kassandra-market/src/lib.rs`
- Create: `programs/kassandra-market/src/error.rs`
- Create: `sdk-rs/Cargo.toml`, `sdk-rs/src/lib.rs`

**Step 1: Root workspace `Cargo.toml`**

```toml
[workspace]
resolver = "2"
members = ["programs/kassandra-market", "sdk-rs"]

[workspace.dependencies]
pinocchio = "0.8"
pinocchio-pubkey = "0.2"
pinocchio-system = "0.2"
pinocchio-token = "0.3"
bytemuck = { version = "1", features = ["derive", "min_const_generics"] }

[profile.release]
overflow-checks = true
```

**Step 2: `rust-toolchain.toml` and `justfile`**

```toml
# rust-toolchain.toml
[toolchain]
channel = "stable"
```

```make
# justfile
build:
    cargo build-sbf --manifest-path programs/kassandra-market/Cargo.toml

test: build
    cargo test -p kassandra-market-program
```

**Step 3: Generate the program keypair and record the ID**

Run:
```bash
mkdir -p target/deploy
solana-keygen new --no-bip39-passphrase -s -o target/deploy/kassandra_market_program-keypair.json
solana-keygen pubkey target/deploy/kassandra_market_program-keypair.json
```
Copy the printed pubkey; it is the program ID used in Step 5 and in `sdk-rs`.

**Step 4: `programs/kassandra-market/Cargo.toml`**

```toml
[package]
name = "kassandra-market-program"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib", "lib"]

[dependencies]
pinocchio = { workspace = true }
pinocchio-pubkey = { workspace = true }
pinocchio-system = { workspace = true }
pinocchio-token = { workspace = true }
bytemuck = { workspace = true }
# Read the Kassandra Oracle layout without pulling in its entrypoint.
kassandra-program = { path = "../../../kassandra/programs/kassandra", default-features = false, features = ["no-entrypoint"] }

[dev-dependencies]
litesvm = "0.6"
solana-sdk = "2"
spl-token = { version = "6", features = ["no-entrypoint"] }
kassandra-market-sdk = { path = "../../sdk-rs" }

[features]
no-entrypoint = []
```

**Step 5: `programs/kassandra-market/src/lib.rs`** (paste the pubkey from Step 3)

```rust
#![allow(unexpected_cfgs)]
use pinocchio::{account_info::AccountInfo, pubkey::Pubkey, ProgramResult};

#[cfg(not(feature = "no-entrypoint"))]
use pinocchio::entrypoint;

#[cfg(not(feature = "no-entrypoint"))]
entrypoint!(process_instruction);

pub mod error;
pub mod instruction;
pub mod processor;
pub mod state;

pub const ID: Pubkey = pinocchio_pubkey::pubkey!("PASTE_PROGRAM_PUBKEY_HERE");

pub fn process_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    instruction_data: &[u8],
) -> ProgramResult {
    processor::process(program_id, accounts, instruction_data)
}
```

**Step 6: `programs/kassandra-market/src/error.rs`**

```rust
use pinocchio::program_error::ProgramError;

#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MarketError {
    InvalidAccount = 0,
    Unauthorized = 1,
    AlreadyInitialized = 2,
    InvalidSplit = 3,
    ZeroAmount = 4,
    NotFunding = 5,
    OracleNotTerminal = 6,
    AlreadyFunded = 7,
    AlreadyClaimed = 8,
    NotCancelled = 9,
    OracleResolved = 10,
    NotBinary = 11,
    WrongMint = 12,
}

impl From<MarketError> for ProgramError {
    fn from(e: MarketError) -> Self {
        ProgramError::Custom(e as u32)
    }
}
```

**Step 7: Stub the referenced modules so the crate compiles**

Create `src/state.rs`, `src/instruction.rs`, `src/processor.rs` (or `processor/mod.rs`) with minimal placeholders — a `process` fn that returns `Err(ProgramError::InvalidInstructionData)` is enough for now. Also create `sdk-rs/Cargo.toml` and an empty `sdk-rs/src/lib.rs`:

```toml
# sdk-rs/Cargo.toml
[package]
name = "kassandra-market-sdk"
version = "0.1.0"
edition = "2021"

[dependencies]
solana-sdk = "2"
```

**Step 8: Verify it builds**

Run: `just build`
Expected: compiles; `target/deploy/kassandra_market_program.so` exists.

**Step 9: Commit**

```bash
git add -A
git commit -m "chore: scaffold kassandra-market program + sdk workspace"
```

---

## Task 1: State structs + layout test

**Files:**
- Modify: `programs/kassandra-market/src/state.rs`
- Test: `programs/kassandra-market/tests/state_layout.rs`

**Step 1: Write the failing layout test**

```rust
// tests/state_layout.rs
use core::mem::{offset_of, size_of};
use kassandra_market_program::state::*;

#[test]
fn account_sizes_are_stable() {
    assert_eq!(size_of::<Config>(), Config::LEN);
    assert_eq!(size_of::<Market>(), Market::LEN);
    assert_eq!(size_of::<Contribution>(), Contribution::LEN);
    assert_eq!(Config::LEN, 88);
    assert_eq!(Market::LEN, 160);
    assert_eq!(Contribution::LEN, 88);
}

#[test]
fn field_offsets_are_pinned() {
    assert_eq!(offset_of!(Config, account_type), 0);
    assert_eq!(offset_of!(Config, authority), 8);
    assert_eq!(offset_of!(Config, kass_mint), 40);
    assert_eq!(offset_of!(Config, min_liquidity), 72);

    assert_eq!(offset_of!(Market, account_type), 0);
    assert_eq!(offset_of!(Market, oracle), 8);
    assert_eq!(offset_of!(Market, creator), 40);
    assert_eq!(offset_of!(Market, kass_mint), 72);
    assert_eq!(offset_of!(Market, escrow_vault), 104);
    assert_eq!(offset_of!(Market, min_liquidity), 136);
    assert_eq!(offset_of!(Market, total_contributed), 144);
    assert_eq!(offset_of!(Market, open_yes_bps), 152);
    assert_eq!(offset_of!(Market, status), 154);

    assert_eq!(offset_of!(Contribution, account_type), 0);
    assert_eq!(offset_of!(Contribution, market), 8);
    assert_eq!(offset_of!(Contribution, contributor), 40);
    assert_eq!(offset_of!(Contribution, amount), 72);
    assert_eq!(offset_of!(Contribution, claimed), 80);
}
```

**Step 2: Run to verify it fails**

Run: `cargo test -p kassandra-market-program --test state_layout`
Expected: FAIL (types not defined).

**Step 3: Implement `state.rs`**

```rust
//! Fixed-size, zero-copy on-chain account layouts.
//!
//! Every account struct is `#[repr(C)]`, `Pod` + `Zeroable`, fully packed with
//! explicit `_pad`, and carries an `AccountType` tag as its first byte so
//! processors can reject type-confusion.

use bytemuck::{Pod, Zeroable};

/// 32-byte key kept as a plain byte array so structs stay `Pod`.
pub type Pubkey = [u8; 32];

#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AccountType {
    Uninitialized = 0,
    Config = 1,
    Market = 2,
    Contribution = 3,
}
impl AccountType {
    pub fn as_u8(self) -> u8 {
        self as u8
    }
}

/// Market lifecycle. Phase 1 uses only `Funding` and `Cancelled`; the rest are
/// reserved for Phase 2 (activation + settlement).
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MarketStatus {
    Funding = 0,
    Active = 1,
    Resolved = 2,
    Void = 3,
    Cancelled = 4,
}
impl MarketStatus {
    pub fn as_u8(self) -> u8 {
        self as u8
    }
}

/// Governed singleton at PDA `[b"config"]`. `authority` is the KASS futarchy DAO
/// executor; only it may `update_config`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Config {
    pub account_type: u8, // AccountType::Config
    pub _pad_hdr: [u8; 7],
    pub authority: Pubkey,
    pub kass_mint: Pubkey,
    pub min_liquidity: u64,
    pub bump: u8,
    pub _pad: [u8; 7],
}
impl Config {
    pub const LEN: usize = core::mem::size_of::<Self>();
}

/// One market per oracle, PDA `[b"market", oracle]`. `min_liquidity` is snapshot
/// from `Config` at creation so in-flight markets are immune to governance
/// changes (config-as-state, mirroring Kassandra).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Market {
    pub account_type: u8, // AccountType::Market
    pub _pad_hdr: [u8; 7],
    pub oracle: Pubkey,
    pub creator: Pubkey,
    pub kass_mint: Pubkey,
    pub escrow_vault: Pubkey,
    pub min_liquidity: u64,
    pub total_contributed: u64,
    pub open_yes_bps: u16, // opening YES share in basis points, 1..=9999
    pub status: u8,        // MarketStatus
    pub bump: u8,
    pub escrow_bump: u8,
    pub _pad: [u8; 3],
}
impl Market {
    pub const LEN: usize = core::mem::size_of::<Self>();
}

/// Per-contributor stake, PDA `[b"contribution", market, contributor]`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Contribution {
    pub account_type: u8, // AccountType::Contribution
    pub _pad_hdr: [u8; 7],
    pub market: Pubkey,
    pub contributor: Pubkey,
    pub amount: u64,
    pub claimed: u8, // bool: refunded/claimed
    pub bump: u8,
    pub _pad: [u8; 6],
}
impl Contribution {
    pub const LEN: usize = core::mem::size_of::<Self>();
}
```

**Step 4: Run to verify it passes**

Run: `cargo test -p kassandra-market-program --test state_layout`
Expected: PASS (both tests). If a `LEN` assertion fails, adjust `_pad` — do **not** change the pinned constants without intent.

**Step 5: Commit**

```bash
git add -A && git commit -m "feat: add Config/Market/Contribution account layouts"
```

---

## Task 2: Instruction enum, dispatch, guards, SDK constants + parity test

**Files:**
- Modify: `programs/kassandra-market/src/instruction.rs`
- Modify: `programs/kassandra-market/src/processor.rs` → convert to `processor/mod.rs`
- Create: `programs/kassandra-market/src/processor/guards.rs`
- Modify: `sdk-rs/src/lib.rs`; Create `sdk-rs/src/pda.rs`, `sdk-rs/src/ix.rs`
- Test: `programs/kassandra-market/tests/parity.rs`

**Step 1: `instruction.rs`**

```rust
//! Instruction wire format. First byte of `instruction_data` = discriminant.
//! Discriminants are a stable public contract; append, never renumber.

#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ix {
    InitConfig = 0,
    UpdateConfig = 1,
    CreateMarket = 2,
    Contribute = 3,
    Cancel = 4,
    Refund = 5,
}

impl Ix {
    pub fn from_u8(x: u8) -> Option<Self> {
        match x {
            0 => Some(Ix::InitConfig),
            1 => Some(Ix::UpdateConfig),
            2 => Some(Ix::CreateMarket),
            3 => Some(Ix::Contribute),
            4 => Some(Ix::Cancel),
            5 => Some(Ix::Refund),
            _ => None,
        }
    }
}
```

**Step 2: `processor/mod.rs`**

```rust
use pinocchio::{
    account_info::AccountInfo, program_error::ProgramError, pubkey::Pubkey, ProgramResult,
};

use crate::instruction::Ix;

pub mod guards;
pub mod init_config;
pub mod update_config;
pub mod create_market;
pub mod contribute;
pub mod cancel;
pub mod refund;

pub fn process(program_id: &Pubkey, accounts: &[AccountInfo], data: &[u8]) -> ProgramResult {
    let (&disc, payload) = data
        .split_first()
        .ok_or(ProgramError::InvalidInstructionData)?;
    match Ix::from_u8(disc).ok_or(ProgramError::InvalidInstructionData)? {
        Ix::InitConfig => init_config::process(program_id, accounts, payload),
        Ix::UpdateConfig => update_config::process(program_id, accounts, payload),
        Ix::CreateMarket => create_market::process(program_id, accounts, payload),
        Ix::Contribute => contribute::process(program_id, accounts, payload),
        Ix::Cancel => cancel::process(program_id, accounts, payload),
        Ix::Refund => refund::process(program_id, accounts, payload),
    }
}
```

Create empty `process` stubs in each of the six processor files (`pub fn process(_: &Pubkey, _: &[AccountInfo], _: &[u8]) -> ProgramResult { Err(ProgramError::InvalidInstructionData) }`) so the crate compiles; they are filled in Tasks 3–8.

**Step 3: `processor/guards.rs`** — shared helpers (mirrors Kassandra's `guards.rs`)

```rust
use pinocchio::{
    account_info::AccountInfo,
    instruction::{Seed, Signer},
    program_error::ProgramError,
    pubkey::Pubkey,
    ProgramResult,
};
use pinocchio_system::instructions::CreateAccount;

use crate::error::MarketError;
use crate::state::{AccountType, Config, Contribution, Market};

pub fn assert_owned_by_program(a: &AccountInfo, program_id: &Pubkey) -> ProgramResult {
    if !a.is_owned_by(program_id) {
        return Err(MarketError::InvalidAccount.into());
    }
    Ok(())
}
pub fn assert_signer(a: &AccountInfo) -> ProgramResult {
    if !a.is_signer() {
        return Err(MarketError::Unauthorized.into());
    }
    Ok(())
}
pub fn assert_key(a: &AccountInfo, expected: &Pubkey) -> ProgramResult {
    if a.key() != expected {
        return Err(MarketError::InvalidAccount.into());
    }
    Ok(())
}

pub fn create_pda(
    payer: &AccountInfo,
    pda: &AccountInfo,
    seeds: &[Seed],
    lamports: u64,
    space: usize,
    owner: &Pubkey,
) -> ProgramResult {
    CreateAccount { from: payer, to: pda, lamports, space: space as u64, owner }
        .invoke_signed(&[Signer::from(seeds)])
}

macro_rules! loader {
    ($name:ident, $ty:ident, $tag:expr) => {
        pub fn $name(a: &AccountInfo, program_id: &Pubkey) -> Result<$ty, ProgramError> {
            assert_owned_by_program(a, program_id)?;
            if a.data_len() < $ty::LEN {
                return Err(MarketError::InvalidAccount.into());
            }
            let v: $ty = {
                let d = a.try_borrow_data()?;
                bytemuck::pod_read_unaligned::<$ty>(&d[..$ty::LEN])
            };
            if v.account_type != $tag.as_u8() {
                return Err(MarketError::InvalidAccount.into());
            }
            Ok(v)
        }
    };
}
loader!(load_config, Config, AccountType::Config);
loader!(load_market, Market, AccountType::Market);
loader!(load_contribution, Contribution, AccountType::Contribution);
```

**Step 4: SDK `pda.rs` + `ix.rs` (single source of truth) and `lib.rs`**

`sdk-rs/src/lib.rs`:
```rust
pub mod ix;
pub mod pda;

use solana_sdk::pubkey::Pubkey;
// Paste the same pubkey used in the program's lib.rs.
pub const PROGRAM_ID: Pubkey = solana_sdk::pubkey!("PASTE_PROGRAM_PUBKEY_HERE");

// Discriminants — mirror programs/kassandra-market/src/instruction.rs::Ix.
// Guarded by tests/parity.rs.
pub const IX_INIT_CONFIG: u8 = 0;
pub const IX_UPDATE_CONFIG: u8 = 1;
pub const IX_CREATE_MARKET: u8 = 2;
pub const IX_CONTRIBUTE: u8 = 3;
pub const IX_CANCEL: u8 = 4;
pub const IX_REFUND: u8 = 5;
```

`sdk-rs/src/pda.rs`:
```rust
use solana_sdk::pubkey::Pubkey;
use crate::PROGRAM_ID;

pub fn config() -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"config"], &PROGRAM_ID)
}
pub fn market(oracle: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"market", oracle.as_ref()], &PROGRAM_ID)
}
pub fn escrow(market: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"escrow", market.as_ref()], &PROGRAM_ID)
}
pub fn contribution(market: &Pubkey, contributor: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[b"contribution", market.as_ref(), contributor.as_ref()],
        &PROGRAM_ID,
    )
}
```

`sdk-rs/src/ix.rs`: instruction builders are added incrementally as each processor task lands (Tasks 3–8). Start it empty with `use` lines:
```rust
use solana_sdk::{instruction::{AccountMeta, Instruction}, pubkey::Pubkey};
use crate::*;
```

**Step 5: Parity test**

```rust
// tests/parity.rs
use kassandra_market_program::instruction::Ix;
use kassandra_market_sdk as sdk;

#[test]
fn ix_discriminants_match_sdk() {
    assert_eq!(Ix::InitConfig as u8, sdk::IX_INIT_CONFIG);
    assert_eq!(Ix::UpdateConfig as u8, sdk::IX_UPDATE_CONFIG);
    assert_eq!(Ix::CreateMarket as u8, sdk::IX_CREATE_MARKET);
    assert_eq!(Ix::Contribute as u8, sdk::IX_CONTRIBUTE);
    assert_eq!(Ix::Cancel as u8, sdk::IX_CANCEL);
    assert_eq!(Ix::Refund as u8, sdk::IX_REFUND);
}

#[test]
fn program_id_matches_sdk() {
    assert_eq!(
        kassandra_market_program::ID,
        kassandra_market_sdk::PROGRAM_ID.to_bytes()
    );
}
```

**Step 6: Verify build + parity**

Run: `just build && cargo test -p kassandra-market-program --test parity`
Expected: PASS.

**Step 7: Commit**

```bash
git add -A && git commit -m "feat: instruction enum, dispatch, guards, sdk PDAs + parity test"
```

---

## Task 3: `init_config`

Creates the `Config` singleton once. **Payload:** `authority: [u8;32]` + `min_liquidity: u64 LE` (40 bytes). **Accounts:** `[config(pda,w), payer(signer,w), kass_mint(ro), system_program]`.

**Files:**
- Modify: `programs/kassandra-market/src/processor/init_config.rs`
- Modify: `sdk-rs/src/ix.rs`
- Create: `programs/kassandra-market/tests/common/mod.rs` (shared harness — build it here, reuse in later tasks)
- Test: `programs/kassandra-market/tests/init_config.rs`

**Step 1: Build the LiteSVM harness** (`tests/common/mod.rs`)

Mirror Kassandra's harness exactly (see reference): `LiteSVM::new()`, airdrop payer, `svm.add_program(Pubkey::new_from_array(kassandra_market_program::ID), include_bytes!("../../../../target/deploy/kassandra_market_program.so"))`, a `send(ix, signers)` that rotates blockhash, a `read_pod<T: bytemuck::Pod>(key) -> T`, a `create_mint(decimals)` + `fund_token(owner, mint, amount)` helper (using `spl_token::state::{Mint, Account}` packed via `set_account`), and a `custom_code(&res) -> Option<u32>` helper. Add `mod common; use common::*;` at the top of every test file.

**Step 2: Write the failing test** (`tests/init_config.rs`)

```rust
mod common;
use common::*;
use kassandra_market_program::state::{AccountType, Config};
use solana_sdk::signature::{Keypair, Signer};

#[test]
fn init_config_happy_path() {
    let mut ctx = TestCtx::new();
    let kass = ctx.create_mint(9);
    let authority = Keypair::new().pubkey();
    let (cfg_pda, res) = ctx.init_config(authority, kass, 1_000_000_000);
    assert!(res.is_ok(), "{res:?}");
    let c: Config = ctx.read_pod(cfg_pda);
    assert_eq!(c.account_type, AccountType::Config.as_u8());
    assert_eq!(c.authority, authority.to_bytes());
    assert_eq!(c.kass_mint, kass.to_bytes());
    assert_eq!(c.min_liquidity, 1_000_000_000);
}

#[test]
fn init_config_twice_fails() {
    let mut ctx = TestCtx::new();
    let kass = ctx.create_mint(9);
    let authority = Keypair::new().pubkey();
    let (_pda, res1) = ctx.init_config(authority, kass, 1);
    assert!(res1.is_ok());
    let (_pda, res2) = ctx.init_config(authority, kass, 2);
    assert!(res2.is_err()); // account already exists
}
```

Add `init_config` to the harness (`impl TestCtx`): derive `pda::config()`, build the ix via `kassandra_market_sdk::ix::init_config(...)`, `send`.

**Step 3: Run to verify it fails**

Run: `just build && cargo test -p kassandra-market-program --test init_config`
Expected: FAIL (`process` stub returns error).

**Step 4: SDK builder** (`sdk-rs/src/ix.rs`)

```rust
pub fn init_config(
    payer: &Pubkey,
    kass_mint: &Pubkey,
    authority: &Pubkey,
    min_liquidity: u64,
) -> Instruction {
    let (config, _) = crate::pda::config();
    let mut data = vec![IX_INIT_CONFIG];
    data.extend_from_slice(authority.as_ref());
    data.extend_from_slice(&min_liquidity.to_le_bytes());
    Instruction {
        program_id: PROGRAM_ID,
        accounts: vec![
            AccountMeta::new(config, false),
            AccountMeta::new(*payer, true),
            AccountMeta::new_readonly(*kass_mint, false),
            AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
        ],
        data,
    }
}
```

**Step 5: Implement the processor** (`processor/init_config.rs`)

```rust
use bytemuck::Zeroable;
use pinocchio::{
    account_info::AccountInfo,
    instruction::Seed,
    program_error::ProgramError,
    pubkey::{find_program_address, Pubkey},
    sysvars::{rent::Rent, Sysvar},
    ProgramResult,
};

use crate::{
    error::MarketError,
    processor::guards::{assert_key, assert_signer, create_pda},
    state::{AccountType, Config},
};

const PAYLOAD_LEN: usize = 40;

pub fn process(program_id: &Pubkey, accounts: &[AccountInfo], payload: &[u8]) -> ProgramResult {
    if payload.len() != PAYLOAD_LEN {
        return Err(ProgramError::InvalidInstructionData);
    }
    let mut authority = [0u8; 32];
    authority.copy_from_slice(&payload[0..32]);
    let min_liquidity = u64::from_le_bytes(payload[32..40].try_into().unwrap());

    let [config_ai, payer_ai, kass_mint_ai, system_prog_ai, ..] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    assert_signer(payer_ai)?;
    assert_key(system_prog_ai, &pinocchio_system::ID)?;

    let (expected, bump) = find_program_address(&[b"config"], program_id);
    assert_key(config_ai, &expected)?;
    if config_ai.lamports() != 0 || !config_ai.data_is_empty() {
        return Err(MarketError::AlreadyInitialized.into());
    }

    let rent = Rent::get()?.minimum_balance(Config::LEN);
    let bump_seed = [bump];
    let seeds = [Seed::from(b"config".as_ref()), Seed::from(&bump_seed)];
    create_pda(payer_ai, config_ai, &seeds, rent, Config::LEN, program_id)?;

    let mut config = Config::zeroed();
    config.account_type = AccountType::Config.as_u8();
    config.authority = authority;
    config.kass_mint = *kass_mint_ai.key();
    config.min_liquidity = min_liquidity;
    config.bump = bump;
    {
        let mut data = config_ai.try_borrow_mut_data()?;
        data.copy_from_slice(bytemuck::bytes_of(&config));
    }
    Ok(())
}
```

**Step 6: Run to verify it passes**

Run: `just build && cargo test -p kassandra-market-program --test init_config`
Expected: PASS (both tests).

**Step 7: Commit**

```bash
git add -A && git commit -m "feat: init_config instruction + LiteSVM harness"
```

---

## Task 4: `update_config`

Futarchy-gated update of `min_liquidity`. **Payload:** `min_liquidity: u64 LE` (8 bytes). **Accounts:** `[config(w), authority(signer)]`. Guard: `assert_signer(authority)` and `authority.key() == config.authority`.

**Files:** Modify `processor/update_config.rs`, `sdk-rs/src/ix.rs`; Test `tests/update_config.rs`.

**Step 1: Failing test**

```rust
mod common;
use common::*;
use kassandra_market_program::state::Config;
use solana_sdk::signature::{Keypair, Signer};

#[test]
fn update_config_by_authority_ok() {
    let mut ctx = TestCtx::new();
    let kass = ctx.create_mint(9);
    let authority = Keypair::new();
    let (cfg, _) = ctx.init_config(authority.pubkey(), kass, 100);
    let res = ctx.update_config(&authority, 500);
    assert!(res.is_ok(), "{res:?}");
    let c: Config = ctx.read_pod(cfg);
    assert_eq!(c.min_liquidity, 500);
}

#[test]
fn update_config_by_stranger_fails() {
    let mut ctx = TestCtx::new();
    let kass = ctx.create_mint(9);
    let authority = Keypair::new();
    ctx.init_config(authority.pubkey(), kass, 100);
    let stranger = Keypair::new();
    ctx.svm_airdrop(&stranger.pubkey());
    let res = ctx.update_config(&stranger, 999);
    assert_eq!(custom_code(&res), Some(kassandra_market_program::error::MarketError::Unauthorized as u32));
}
```

**Step 2: Run → FAIL.** `cargo test -p kassandra-market-program --test update_config`

**Step 3: SDK builder** — `update_config(authority: &Pubkey, min_liquidity: u64)`: data `[IX_UPDATE_CONFIG] ++ min_liquidity.to_le_bytes()`, accounts `[config(w,false), authority(ro,true-signer)]` (use `AccountMeta::new_readonly(*authority, true)`).

**Step 4: Implement processor**

```rust
pub fn process(program_id: &Pubkey, accounts: &[AccountInfo], payload: &[u8]) -> ProgramResult {
    if payload.len() != 8 {
        return Err(ProgramError::InvalidInstructionData);
    }
    let min_liquidity = u64::from_le_bytes(payload[0..8].try_into().unwrap());
    let [config_ai, authority_ai, ..] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    assert_signer(authority_ai)?;
    let config = load_config(config_ai, program_id)?;
    if authority_ai.key() != &config.authority {
        return Err(MarketError::Unauthorized.into());
    }
    let mut updated = config;
    updated.min_liquidity = min_liquidity;
    let mut data = config_ai.try_borrow_mut_data()?;
    data[..Config::LEN].copy_from_slice(bytemuck::bytes_of(&updated));
    Ok(())
}
```

**Step 5: Run → PASS. Step 6: Commit** `feat: update_config (futarchy-gated)`.

---

## Task 5: `create_market`

Creates a `Market` (PDA `[b"market", oracle]`) + KASS escrow token account (PDA `[b"escrow", market]`, SPL owner = market PDA) + the creator's `Contribution`, and transfers the creator's seed into escrow. **Payload:** `open_yes_bps: u16 LE` + `seed_amount: u64 LE` (10 bytes). **Accounts:** `[config(ro), oracle(ro), market(pda,w), escrow(pda,w), kass_mint(ro), creator(signer,w), creator_kass_ata(w), contribution(pda,w), token_program, system_program]`.

Validation: `load_config`; read the Kassandra oracle (owned by `kassandra_program::ID`, `account_type == Oracle`); `oracle.options_count == 2` else `NotBinary`; oracle **not** terminal (`phase < Resolved(7)`) else `OracleResolved`; `kass_mint == config.kass_mint` else `WrongMint`; `1 <= open_yes_bps <= 9999` else `InvalidSplit`; `seed_amount > 0` else `ZeroAmount`. Snapshot `min_liquidity` from config into the market.

**Files:** Modify `processor/create_market.rs`; Create `processor/contribution.rs` (shared `record_contribution` helper); Modify `sdk-rs/src/ix.rs`; Test `tests/create_market.rs`.

**Step 1: Read the Kassandra Oracle layout** — add a guard `load_kassandra_oracle(a, ) -> Oracle`:

```rust
// in guards.rs
use kassandra_program::state::Oracle as KassOracle;
pub fn load_kassandra_oracle(a: &AccountInfo) -> Result<KassOracle, ProgramError> {
    if !a.is_owned_by(&kassandra_program::ID) {
        return Err(MarketError::InvalidAccount.into());
    }
    if a.data_len() < KassOracle::LEN {
        return Err(MarketError::InvalidAccount.into());
    }
    let o = { let d = a.try_borrow_data()?; bytemuck::pod_read_unaligned::<KassOracle>(&d[..KassOracle::LEN]) };
    if o.account_type != kassandra_program::state::AccountType::Oracle.as_u8() {
        return Err(MarketError::InvalidAccount.into());
    }
    Ok(o)
}
```
(Confirm `Oracle`, `AccountType`, and `Phase` are `pub` in `kassandra_program::state`; the reference confirms they are.)

**Step 2: Shared contribution helper** (`processor/contribution.rs`)

```rust
use pinocchio::{
    account_info::AccountInfo, instruction::Seed, program_error::ProgramError,
    pubkey::{find_program_address, Pubkey}, sysvars::{rent::Rent, Sysvar}, ProgramResult,
};
use pinocchio_token::instructions::Transfer;
use bytemuck::Zeroable;
use crate::{error::MarketError, processor::guards::{assert_key, create_pda, load_contribution},
    state::{AccountType, Contribution}};

/// Transfer `amount` KASS from `src_ata` (authority = signer) into `escrow`,
/// and create-or-increment the contributor's Contribution record.
#[allow(clippy::too_many_arguments)]
pub fn record_contribution(
    program_id: &Pubkey,
    market_key: &Pubkey,
    contributor_ai: &AccountInfo,
    src_ata_ai: &AccountInfo,
    escrow_ai: &AccountInfo,
    contribution_ai: &AccountInfo,
    token_prog_ai: &AccountInfo,
    payer_ai: &AccountInfo,
    amount: u64,
) -> Result<(), ProgramError> {
    if amount == 0 {
        return Err(MarketError::ZeroAmount.into());
    }
    assert_key(token_prog_ai, &pinocchio_token::ID)?;

    let (expected, bump) =
        find_program_address(&[b"contribution", market_key.as_ref(), contributor_ai.key().as_ref()], program_id);
    assert_key(contribution_ai, &expected)?;

    // Move the KASS first (authority is the contributor signer).
    Transfer { from: src_ata_ai, to: escrow_ai, authority: contributor_ai, amount }.invoke()?;

    if contribution_ai.lamports() == 0 && contribution_ai.data_is_empty() {
        let rent = Rent::get()?.minimum_balance(Contribution::LEN);
        let bump_seed = [bump];
        let seeds = [
            Seed::from(b"contribution".as_ref()),
            Seed::from(market_key.as_ref()),
            Seed::from(contributor_ai.key().as_ref()),
            Seed::from(&bump_seed),
        ];
        create_pda(payer_ai, contribution_ai, &seeds, rent, Contribution::LEN, program_id)?;
        let mut c = Contribution::zeroed();
        c.account_type = AccountType::Contribution.as_u8();
        c.market = *market_key;
        c.contributor = *contributor_ai.key();
        c.amount = amount;
        c.bump = bump;
        let mut data = contribution_ai.try_borrow_mut_data()?;
        data.copy_from_slice(bytemuck::bytes_of(&c));
    } else {
        let mut c = load_contribution(contribution_ai, program_id)?;
        c.amount = c.amount.checked_add(amount).ok_or(ProgramError::ArithmeticOverflow)?;
        let mut data = contribution_ai.try_borrow_mut_data()?;
        data[..Contribution::LEN].copy_from_slice(bytemuck::bytes_of(&c));
    }
    Ok(())
}
```

**Step 3: Failing test** (`tests/create_market.rs`) — seed a fake Kassandra oracle account with `set_account` (owned by `kassandra_program::ID`, `account_type = Oracle`, `options_count = 2`, `phase = Proposal`). Add a harness helper `seed_kass_oracle(options_count, phase) -> Pubkey` that builds `kassandra_program::state::Oracle::zeroed()`, stamps those fields, and `set_account`s it owned by `Pubkey::new_from_array(kassandra_program::ID)`. Then:

```rust
#[test]
fn create_market_happy_path() {
    let mut ctx = TestCtx::new();
    let kass = ctx.create_mint(9);
    let authority = solana_sdk::signature::Keypair::new();
    ctx.init_config(authority.pubkey(), kass, 1_000_000_000);
    let oracle = ctx.seed_kass_oracle(2, 1 /* Proposal */);
    let creator = ctx.funded_kass_holder(kass, 500_000_000); // Keypair + funded ATA
    let (market, res) = ctx.create_market(&creator, oracle, kass, 5000, 200_000_000);
    assert!(res.is_ok(), "{res:?}");
    let m: kassandra_market_program::state::Market = ctx.read_pod(market);
    assert_eq!(m.status, kassandra_market_program::state::MarketStatus::Funding.as_u8());
    assert_eq!(m.total_contributed, 200_000_000);
    assert_eq!(m.min_liquidity, 1_000_000_000);
    assert_eq!(m.open_yes_bps, 5000);
    // escrow holds the seed
    assert_eq!(ctx.token_balance(m.escrow_vault_pubkey()), 200_000_000);
}
```
Add failure tests: `open_yes_bps = 0` → `InvalidSplit`; `options_count = 3` oracle → `NotBinary`; seed `= 0` → `ZeroAmount`; resolved oracle (phase 7) → `OracleResolved`.

**Step 4: Run → FAIL.**

**Step 5: SDK builder** `create_market(creator, oracle, kass_mint, creator_kass_ata, open_yes_bps, seed_amount)` — derive `market`, `escrow`, `contribution(market, creator)`, `config`; data `[IX_CREATE_MARKET] ++ open_yes_bps.to_le_bytes() ++ seed_amount.to_le_bytes()`; account metas in the order listed above (config ro, oracle ro, market w, escrow w, kass_mint ro, creator signer+w, creator_kass_ata w, contribution w, token_program, system_program).

**Step 6: Implement processor** (`processor/create_market.rs`)

```rust
const PAYLOAD_LEN: usize = 10;
const SPL_TOKEN_ACCOUNT_LEN: usize = 165;

pub fn process(program_id: &Pubkey, accounts: &[AccountInfo], payload: &[u8]) -> ProgramResult {
    if payload.len() != PAYLOAD_LEN {
        return Err(ProgramError::InvalidInstructionData);
    }
    let open_yes_bps = u16::from_le_bytes(payload[0..2].try_into().unwrap());
    let seed_amount = u64::from_le_bytes(payload[2..10].try_into().unwrap());

    let [config_ai, oracle_ai, market_ai, escrow_ai, kass_mint_ai, creator_ai, creator_ata_ai, contribution_ai, token_prog_ai, system_prog_ai, ..] = accounts
    else { return Err(ProgramError::NotEnoughAccountKeys); };

    assert_signer(creator_ai)?;
    assert_key(token_prog_ai, &pinocchio_token::ID)?;
    assert_key(system_prog_ai, &pinocchio_system::ID)?;

    let config = load_config(config_ai, program_id)?;
    assert_key(kass_mint_ai, &config.kass_mint)?; // WrongMint via InvalidAccount, or map explicitly
    if !(1..=9999).contains(&open_yes_bps) {
        return Err(MarketError::InvalidSplit.into());
    }

    let oracle = load_kassandra_oracle(oracle_ai)?;
    if oracle.options_count != 2 {
        return Err(MarketError::NotBinary.into());
    }
    // Phase enum: Resolved = 7, InvalidDeadend = 8 — reject terminal.
    if oracle.phase >= kassandra_program::state::Phase::Resolved.as_u8() {
        return Err(MarketError::OracleResolved.into());
    }

    // Derive + validate market and escrow PDAs.
    let (market_key, market_bump) =
        find_program_address(&[b"market", oracle_ai.key().as_ref()], program_id);
    assert_key(market_ai, &market_key)?;
    if market_ai.lamports() != 0 || !market_ai.data_is_empty() {
        return Err(MarketError::InvalidAccount.into()); // one market per oracle
    }
    let (escrow_key, escrow_bump) =
        find_program_address(&[b"escrow", market_ai.key().as_ref()], program_id);
    assert_key(escrow_ai, &escrow_key)?;

    // Create the market account.
    let market_rent = Rent::get()?.minimum_balance(Market::LEN);
    let mbump = [market_bump];
    let market_seeds = [Seed::from(b"market".as_ref()), Seed::from(oracle_ai.key().as_ref()), Seed::from(&mbump)];
    create_pda(creator_ai, market_ai, &market_seeds, market_rent, Market::LEN, program_id)?;

    // Create the escrow SPL token account owned by the market PDA.
    let vault_rent = Rent::get()?.minimum_balance(SPL_TOKEN_ACCOUNT_LEN);
    let ebump = [escrow_bump];
    let escrow_seeds = [Seed::from(b"escrow".as_ref()), Seed::from(market_ai.key().as_ref()), Seed::from(&ebump)];
    create_pda(creator_ai, escrow_ai, &escrow_seeds, vault_rent, SPL_TOKEN_ACCOUNT_LEN, &pinocchio_token::ID)?;
    InitializeAccount3 { account: escrow_ai, mint: kass_mint_ai, owner: market_ai.key() }.invoke()?;

    // Stamp initial market state (Funding, seed not yet counted).
    let mut market = Market::zeroed();
    market.account_type = AccountType::Market.as_u8();
    market.oracle = *oracle_ai.key();
    market.creator = *creator_ai.key();
    market.kass_mint = config.kass_mint;
    market.escrow_vault = *escrow_ai.key();
    market.min_liquidity = config.min_liquidity;
    market.total_contributed = 0;
    market.open_yes_bps = open_yes_bps;
    market.status = MarketStatus::Funding.as_u8();
    market.bump = market_bump;
    market.escrow_bump = escrow_bump;
    { let mut d = market_ai.try_borrow_mut_data()?; d.copy_from_slice(bytemuck::bytes_of(&market)); }

    // Record the creator's seed as the first contribution (transfers KASS in).
    record_contribution(program_id, market_ai.key(), creator_ai, creator_ata_ai, escrow_ai, contribution_ai, token_prog_ai, creator_ai, seed_amount)?;

    // Bump total_contributed.
    let mut m2 = market;
    m2.total_contributed = seed_amount;
    { let mut d = market_ai.try_borrow_mut_data()?; d[..Market::LEN].copy_from_slice(bytemuck::bytes_of(&m2)); }
    Ok(())
}
```
(Imports: `InitializeAccount3` from `pinocchio_token::instructions`, plus the guards/state/`record_contribution`. If mapping `kass_mint` mismatch to a distinct `WrongMint` code is desired, replace `assert_key` with an explicit check returning `MarketError::WrongMint`.)

**Step 7: Run → PASS (all create_market tests). Step 8: Commit** `feat: create_market + shared record_contribution`.

---

## Task 6: `contribute`

Adds KASS from any contributor while `status == Funding`. **Payload:** `amount: u64 LE`. **Accounts:** `[market(w), oracle(ro-unused-here? no), escrow(w), contributor(signer,w), contributor_kass_ata(w), contribution(pda,w), token_program]`. (No oracle account needed.)

**Files:** Modify `processor/contribute.rs`, `sdk-rs/src/ix.rs`; Test `tests/contribute.rs`.

**Step 1: Failing test** — happy path: second contributor adds; `total_contributed` sums; escrow balance sums; their `Contribution.amount` correct. Repeat-contribution by same key increments. Failure: contributing to a non-`Funding` market → `NotFunding` (simulate by cancelling first in a later task, or seed a market with a non-Funding status via `set_account`).

**Step 2: Run → FAIL.**

**Step 3: SDK builder** `contribute(contributor, market, escrow, contributor_ata, amount)` (derive `contribution(market, contributor)`).

**Step 4: Implement processor**

```rust
pub fn process(program_id: &Pubkey, accounts: &[AccountInfo], payload: &[u8]) -> ProgramResult {
    if payload.len() != 8 {
        return Err(ProgramError::InvalidInstructionData);
    }
    let amount = u64::from_le_bytes(payload[0..8].try_into().unwrap());
    let [market_ai, escrow_ai, contributor_ai, contributor_ata_ai, contribution_ai, token_prog_ai, ..] = accounts
    else { return Err(ProgramError::NotEnoughAccountKeys); };

    assert_signer(contributor_ai)?;
    let market = load_market(market_ai, program_id)?;
    if market.status != MarketStatus::Funding.as_u8() {
        return Err(MarketError::NotFunding.into());
    }
    assert_key(escrow_ai, &market.escrow_vault)?;

    record_contribution(program_id, market_ai.key(), contributor_ai, contributor_ata_ai, escrow_ai, contribution_ai, token_prog_ai, contributor_ai, amount)?;

    let mut m = market;
    m.total_contributed = m.total_contributed.checked_add(amount).ok_or(ProgramError::ArithmeticOverflow)?;
    let mut d = market_ai.try_borrow_mut_data()?;
    d[..Market::LEN].copy_from_slice(bytemuck::bytes_of(&m));
    Ok(())
}
```

**Step 5: Run → PASS. Step 6: Commit** `feat: contribute instruction`.

---

## Task 7: `cancel`

Marks an under-funded market `Cancelled` once its oracle is terminal. **Payload:** empty. **Accounts:** `[market(w), oracle(ro)]`. Guards: `status == Funding` else `NotFunding`; `market.oracle == oracle.key()`; oracle terminal (`phase == Resolved(7) || phase == InvalidDeadend(8)`) else `OracleNotTerminal`; `total_contributed < min_liquidity` else `AlreadyFunded` (a fully-funded market must be activated, not cancelled — activation is Phase 2).

> **Deferred:** a funding-deadline-based cancel (so contributors aren't stuck until the oracle resolves) is a Phase-2 refinement; for now cancellation requires oracle terminality.

**Files:** Modify `processor/cancel.rs`, `sdk-rs/src/ix.rs`; Test `tests/cancel.rs`.

**Step 1: Failing test** — seed oracle in `Resolved` phase, create market under-funded, `cancel`, assert `status == Cancelled`. Failure: oracle still in `Proposal` → `OracleNotTerminal`; funded ≥ min → `AlreadyFunded`.

**Step 2: Run → FAIL. Step 3: SDK builder** `cancel(market, oracle)`.

**Step 4: Implement processor**

```rust
pub fn process(program_id: &Pubkey, accounts: &[AccountInfo], payload: &[u8]) -> ProgramResult {
    if !payload.is_empty() {
        return Err(ProgramError::InvalidInstructionData);
    }
    let [market_ai, oracle_ai, ..] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    let market = load_market(market_ai, program_id)?;
    if market.status != MarketStatus::Funding.as_u8() {
        return Err(MarketError::NotFunding.into());
    }
    assert_key(oracle_ai, &market.oracle)?;
    let oracle = load_kassandra_oracle(oracle_ai)?;
    let terminal = oracle.phase == kassandra_program::state::Phase::Resolved.as_u8()
        || oracle.phase == kassandra_program::state::Phase::InvalidDeadend.as_u8();
    if !terminal {
        return Err(MarketError::OracleNotTerminal.into());
    }
    if market.total_contributed >= market.min_liquidity {
        return Err(MarketError::AlreadyFunded.into());
    }
    let mut m = market;
    m.status = MarketStatus::Cancelled.as_u8();
    let mut d = market_ai.try_borrow_mut_data()?;
    d[..Market::LEN].copy_from_slice(bytemuck::bytes_of(&m));
    Ok(())
}
```

**Step 5: Run → PASS. Step 6: Commit** `feat: cancel under-funded market on oracle terminality`.

---

## Task 8: `refund`

Permissionless per-contributor refund from a `Cancelled` market. **Payload:** empty. **Accounts:** `[market(w? ro), escrow(w), contribution(w), contributor_kass_ata(w), token_program, payer(signer)]`. Guards: `market.status == Cancelled` else `NotCancelled`; `contribution.market == market.key()`; `contribution.claimed == 0` else `AlreadyClaimed`; the destination ATA's SPL owner (bytes 32..64 of the token account) `== contribution.contributor`. Transfer `contribution.amount` KASS from escrow to the ATA, **program-signed** with the market PDA seeds `[b"market", oracle, market.bump]` (escrow's SPL authority is the market PDA). Set `claimed = 1`.

**Files:** Modify `processor/refund.rs`, `sdk-rs/src/ix.rs`; Test `tests/refund.rs`.

**Step 1: Failing test** — create under-funded market with two contributors, seed oracle terminal, `cancel`, then `refund` each; assert each contributor's KASS ATA regains their amount, escrow drains to 0, `Contribution.claimed == 1`. Failure: double refund → `AlreadyClaimed`; refund before cancel → `NotCancelled`; wrong destination ATA owner → `InvalidAccount`.

**Step 2: Run → FAIL. Step 3: SDK builder** `refund(payer, market, oracle, escrow, contribution, contributor_ata)`.

**Step 4: Implement processor**

```rust
pub fn process(program_id: &Pubkey, accounts: &[AccountInfo], payload: &[u8]) -> ProgramResult {
    if !payload.is_empty() {
        return Err(ProgramError::InvalidInstructionData);
    }
    let [market_ai, escrow_ai, contribution_ai, dest_ata_ai, token_prog_ai, ..] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    assert_key(token_prog_ai, &pinocchio_token::ID)?;

    let market = load_market(market_ai, program_id)?;
    if market.status != MarketStatus::Cancelled.as_u8() {
        return Err(MarketError::NotCancelled.into());
    }
    assert_key(escrow_ai, &market.escrow_vault)?;

    let contribution = load_contribution(contribution_ai, program_id)?;
    if contribution.market != *market_ai.key() {
        return Err(MarketError::InvalidAccount.into());
    }
    if contribution.claimed != 0 {
        return Err(MarketError::AlreadyClaimed.into());
    }
    // Destination must belong to the recorded contributor (SPL owner @ offset 32).
    {
        let d = dest_ata_ai.try_borrow_data()?;
        if d.len() < 64 || d[32..64] != contribution.contributor {
            return Err(MarketError::InvalidAccount.into());
        }
    }

    // Program-signed transfer out of escrow (authority = market PDA).
    let mbump = [market.bump];
    let market_seeds = [Seed::from(b"market".as_ref()), Seed::from(market.oracle.as_ref()), Seed::from(&mbump)];
    Transfer { from: escrow_ai, to: dest_ata_ai, authority: market_ai, amount: contribution.amount }
        .invoke_signed(&[Signer::from(&market_seeds)])?;

    let mut c = contribution;
    c.claimed = 1;
    let mut data = contribution_ai.try_borrow_mut_data()?;
    data[..Contribution::LEN].copy_from_slice(bytemuck::bytes_of(&c));
    Ok(())
}
```

**Step 5: Run → PASS. Step 6: Commit** `feat: permissionless refund from cancelled market`.

---

## Task 9: Full-lifecycle integration test + wrap-up

**Files:** Test `programs/kassandra-market/tests/lifecycle.rs`.

**Step 1:** One end-to-end test: `init_config` → `create_market` (creator seeds < min) → two `contribute`s (still < min) → oracle warps to `Resolved` → `cancel` → `refund` all three → assert everyone whole, escrow empty, all `Contribution.claimed == 1`, market `Cancelled`.

**Step 2:** Run the whole suite: `just test`. Expected: all tests pass.

**Step 3:** Update `docs/plans/2026-07-03-kassandra-market-design.md`'s status note to mark Phase 1 (funding lifecycle) implemented.

**Step 4: Commit** `test: full crowdfunding-lifecycle integration test`.

---

## Out of scope (Phase 2 — separate plan)

- `activate` — compose the MetaDAO `Question` (oracle authority = Market PDA) + `conditional_vault` (underlying = KASS) + `cYES`/`cNO` AMM pool; verify + record bindings; split escrow per `open_yes_bps` and `add_liquidity`.
- `claim_lp` — pro-rata LP distribution to contributors after activation.
- `resolve_market` — read `resolved_option`, CPI `resolve_question` (`[1,0]`/`[0,1]`; `[1,1]` void).
- surfpool mainnet-fork e2e against real MetaDAO programs.
- TS SDK + app.
- Funding-deadline-based cancel; categorical (N>2) markets; protocol fee switch.
