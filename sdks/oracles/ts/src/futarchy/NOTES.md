# Futarchy v0.6 + Squads v4 + conditional_vault — SDK wire-format recon (Task G2)

This is the authoritative layout map the `sdk/src/futarchy/*` builders are derived
from. Every discriminator mirrors the binary-validated Rust CPI modules
(`programs/kassandra/src/cpi/metadao_v06.rs` for futarchy/Squads, `…/metadao.rs`
for the conditional_vault); every account ordering + arg layout is sourced as
noted per instruction. Where a wire format is NOT authoritatively determinable it
is marked **DEFERRED / STOP-REPORTED** rather than guessed.

Program IDs (`EXTERNAL_PROGRAM_IDS`, `metadao_v06.rs`):
- futarchy v0.6 `FUTARELBfJfQ8RDGhg1wdhddq1odMAJUePHFuBYfUxKq`
- conditional_vault `VLTX1ishMBbcX3rdBWGssxawAo1Q2X2qxYFYqiGodVg` (v0.4==v0.6)
- Meteora DAMM v2 (cp-amm) `cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG`
- Squads v4 `SQDS4ep65T869zMMBKyuUq6aD6EgTu8psMjkvj52pCf`

All futarchy instructions are `#[event_cpi]` → two TRAILING accounts appended
after the declared list: `event_authority` (`[b"__event_authority"]` under the
futarchy program) then the futarchy program id. The conditional_vault is likewise
`#[event_cpi]`.

---

## CRITICAL RECON GATE — `create_key == Dao PDA` — **CONFIRMED**

Source: `metaDAOproject/futarchy@v0.6.0`
`programs/futarchy/src/instructions/initialize_dao.rs` (fetched via
`gh api ... contents` — authoritative source, cross-checked against the dumped
`metadao_futarchy_v06.so` account-name strings).

`initialize_dao` ITSELF creates the Squads multisig via CPI
`squads_multisig_program::cpi::multisig_create_v2(...)` with:

```rust
create_key: dao.to_account_info(),   // <-- the Dao PDA IS the create_key
config_authority: Some(dao.key()),
threshold: 1,
members: [ { key: dao.key(), Vote|Execute },
           { key: permissionless_account (EP3SoC2SvR3d4c2eXVBvhEMWSr2j3YtoCY3UMiQV7BPD), Initiate|Execute } ],
```

The multisig PDA is constrained in the same instruction as
`[SEED_PREFIX, SEED_MULTISIG, dao.key()]` = `[b"multisig", b"multisig", Dao]`
under the Squads program, and the vault as
`[b"multisig", multisig, b"vault", 0u8]` (vault index 0). The `Dao` account then
stores `squads_multisig` and `squads_multisig_vault`.

**⇒ G1's hardened `set_governance` check is CORRECT.** It derives
`vault = squads_vault(squads_multisig(create_key == kass_dao), 0)` and requires
`dao_authority == vault`. Reality matches the assumption exactly.

**Deviation from the plan's literal wording (handled, not a blocker):** plan
Decision 1 / G2(c) describe the bootstrap as creating the multisig as a SEPARATE
step after `initialize_dao`. In reality `initialize_dao` creates the multisig +
vault atomically (the CPI above), so the bootstrap does NOT emit a standalone
`multisig_create_v2`; it calls `initialize_dao`, then DERIVES multisig/vault from
the Dao PDA, then calls `setGovernance`. A standalone `multisigCreateV2` builder
is still provided (args/accounts are determinable from the CPI site) but is unused
by the bootstrap.

---

## G3 ADDENDUM — DEPLOYED program is futarchy **v0.6.1**, not v0.6.0 (verified live)

The G2 layouts above were lifted from the `v0.6.0` SOURCE TAG. The program
actually DEPLOYED on mainnet (and thus on the surfpool fork) is **v0.6.1**
(confirmed by fetching the on-chain Anchor IDL at the program's IDL PDA). G3
verified every builder against that live IDL and corrected three:

- **`initialize_dao` args** — v0.6.1 `InitializeDaoParams` appends two trailing
  fields after `initial_spending_limit`: `team_sponsored_pass_threshold_bps: i16`
  then `team_address: Pubkey`. The None-spending-limit instruction data is
  therefore **117 bytes** (83 + 2 + 32), not 83. The builder defaults them to
  `0` / the zero (system) key. The DAO `invariant()` also now requires
  `seconds_per_proposal >= 86400` (1 day) and `>= 2×twap_start_delay`,
  `min_base/min_quote_futarchic_liquidity > 0`, `twap_max_observation_change > 0`,
  `pass_threshold_bps <= 1000`, and `team_sponsored ∈ [MIN,MAX]`.
- **`initialize_proposal` accounts** — v0.6.1 inserts `squads_multisig` as an
  explicit account at index 2 (12 accounts incl. the event_cpi tail).
- **`launch_proposal` accounts** — v0.6.1 inserts `squads_multisig` +
  `squads_proposal` before `system_program` (20 accounts incl. the tail).

`finalize_proposal` (25) + `conditional_swap` (25) + the conditional_vault +
Squads builders MATCH v0.6.1 unchanged. New builders added for G3:
`provide_liquidity` (disc `286e6b74ae7f61cc`; seeds the embedded spot AMM) +
the `AmmPosition` PDA `[b"amm_position", dao, position_authority]`.

**Squads `TransactionMessage` compact wire format** (used by G3 to stage the
`set_config`/`resolve_deadend` CPIs): `num_signers:u8, num_writable_signers:u8,
num_writable_non_signers:u8, account_keys:SmallVec<u8,Pubkey>,
instructions:SmallVec<u8,CompiledIx>, address_table_lookups:SmallVec<u8,_>`;
`CompiledIx = { program_id_index:u8, account_indexes:SmallVec<u8,u8>,
data:SmallVec<u16,u8> }` — the instruction `data` length prefix is **u16**, all
others u8. `account_keys` ordered [w-signers, ro-signers, w-non-signers,
ro-non-signers]; the inner program id sits among the ro-non-signers. The vault
PDA is a ro-SIGNER in the message (Squads `invoke_signed`s for it on execute);
`vault_transaction_execute.remaining_accounts` = the static `account_keys` in
order (no ALTs). Proposals are created with `draft:false` → `Active` (required by
`initialize_proposal`); the futarchy `finalize_proposal` CPIs `proposal_approve`
(threshold 1) on a PASS, after which `vault_transaction_execute` runs. The
multisig's only `Initiate` member is the PUBLIC permissionless keypair
(`EP3SoC2…`, futarchy `sdk/permissionless-account.json`) — it (not the Dao) must
sign `vault_transaction_create` / `proposal_create`.

## conditional_vault (FULLY VALIDATED against the real binary)

Validated by real-binary CPI in `tests/metadao_v06_cpi.rs::v06_conditional_vault_split`,
`tests/challenge_e2e.rs`, and `sdks/oracles/ts/test/surfpool/challenge-market-e2e.test.ts`.
Discriminators + account orders from `cpi/metadao.rs` (binary-validated).

Discriminators (`sha256("global:<name>")[..8]`):
- initialize_question `f5 97 6a bc 58 2c 41 d4`
- initialize_conditional_vault `25 58 fa d4 36 da e3 af`
- split_tokens `4f c3 74 00 8c b0 49 b3`
- merge_tokens `e2 59 fb 79 e1 82 b4 0e`
- redeem_tokens `f6 62 86 29 98 21 78 45`
- resolve_question `34 20 e0 b3 b4 08 00 f6`

PDA seeds (under conditional_vault):
- question `[b"question", question_id[32], oracle[32], [num_outcomes]]`
- vault `[b"conditional_vault", question, underlying_mint]`
- conditional_token_mint `[b"conditional_token", vault, [index]]`
- event_authority `[b"__event_authority"]`

Args:
- initialize_question: `disc ++ question_id[32] ++ oracle[32] ++ num_outcomes:u8` (73 bytes)
- initialize_conditional_vault: `disc` only (8)
- split_tokens / merge_tokens: `disc ++ amount:u64` (16)
- redeem_tokens: `disc` only (8)
- resolve_question (binary): `disc ++ Vec<u32> { len:u32==2, n0:u32, n1:u32 }` (20)

Account orders (incl. the two trailing event_cpi accounts):
- `initialize_question`: `[question(w,init), payer(w,signer), system_program, event_authority, program]`
- `initialize_conditional_vault`: `[vault(w,init), question, underlying_mint, vault_underlying_ata(w), payer(w,signer), token_program, ata_program, system_program, event_authority, program, +cond_mint[0..n](w)]`
- `split/merge/redeem` (shared `InteractWithVault`): `[question, vault(w), vault_underlying_ata(w), authority(signer), user_underlying_ata(w), token_program, event_authority, program, +cond_mint[0..n](w), +user_cond_token[0..n](w)]`
- `resolve_question`: `[question(w), oracle(signer), event_authority, program]`

---

## futarchy v0.6 (account orders + args from `metaDAOproject/futarchy@v0.6.0` source)

Discriminators (`metadao_v06.rs`, dispatch-probe-validated against the real
`metadao_futarchy_v06.so`):
- initialize_dao `80 e2 60 5a 27 38 18 c4`
- initialize_proposal `32 49 9c 62 81 95 15 9e`
- launch_proposal `10 d3 bd 77 f5 48 00 e5`
- finalize_proposal `17 44 33 a7 6d ad bb a4`
- update_dao `83 48 4b 19 70 d2 6d 02`
- spot_swap `a7 61 0c e7 ed 4e a6 fb`
- conditional_swap `c2 88 dc 59 f2 a9 82 9d`

Account discriminators: `Dao` `a3 09 2f 1f 34 55 c5 31`, `Proposal` `1a 5e bd bb 74 88 35 21`.

PDA seeds (under futarchy): dao `[b"dao", dao_creator, nonce:u64le]`;
proposal `[b"proposal", squads_proposal]`; event_authority `[b"__event_authority"]`.

### initialize_dao  (src: initialize_dao.rs)
`InitializeDaoParams` Borsh (matches `metadao_v06.rs::initialize_dao_data_no_limit`):
`twap_initial_observation:u128, twap_max_observation_change_per_update:u128,
twap_start_delay_seconds:u32, min_quote_futarchic_liquidity:u64,
min_base_futarchic_liquidity:u64, base_to_stake:u64, pass_threshold_bps:u16,
seconds_per_proposal:u32, nonce:u64, initial_spending_limit:Option<InitialSpendingLimit>`
(the bootstrap uses `None` → trailing `0x00`; total 83 bytes).
NOTE: the struct field order (= Borsh order) is min_quote BEFORE min_base; the
handler destructures min_base first but that does NOT affect the wire layout.

Accounts (then event_authority, program):
`[dao(w,init), dao_creator(signer), payer(w,signer), system_program, base_mint,
quote_mint, squads_multisig(w), squads_multisig_vault, squads_program,
squads_program_config, squads_program_config_treasury(w), spending_limit(w),
futarchy_amm_base_vault(w), futarchy_amm_quote_vault(w), token_program,
associated_token_program]`.
- squads_multisig PDA `[b"multisig", b"multisig", dao]` @ squads
- squads_multisig_vault PDA `[b"multisig", multisig, b"vault", 0u8]` @ squads
- squads_program_config PDA `[b"multisig", b"program_config"]` @ squads
- squads_program_config_treasury: NOT a PDA — read from `ProgramConfig.treasury`
  (the bootstrap takes it as a param / fetches it live in G3)
- spending_limit PDA `[b"multisig", multisig, b"spending_limit", dao]` @ squads
- futarchy_amm_{base,quote}_vault: ATA(mint={base,quote}, authority=dao)

### initialize_proposal  (src: initialize_proposal.rs) — NO args
Accounts (then event_authority, program):
`[proposal(w,init), squads_proposal, dao(w), question, quote_vault, base_vault,
proposer(signer), payer(w,signer), system_program]`.
- proposal PDA `[b"proposal", squads_proposal]`; the conditional `question.oracle`
  must equal the proposal PDA. quote_vault.underlying == dao.quote_mint,
  base_vault.underlying == dao.base_mint, both `has_one = question`.

### launch_proposal  (src: launch_proposal.rs) — NO args
Accounts (then event_authority, program):
`[proposal(w), base_vault, quote_vault, pass_base_mint, pass_quote_mint,
fail_base_mint, fail_quote_mint, dao(w), payer(w,signer),
amm_pass_base_vault(w,initIfNeeded), amm_pass_quote_vault(w), amm_fail_base_vault(w),
amm_fail_quote_vault(w), system_program, token_program, associated_token_program]`.
- pass_*_mint = conditional_token_mints[1], fail_*_mint = conditional_token_mints[0]
- amm_*_vault = ATA(mint=that cond mint, authority=dao)
- splits the embedded spot Pool reserves into pass/fail Pools (PoolState::Futarchy).

### finalize_proposal  (src: finalize_proposal.rs) — NO args
Accounts (then event_authority, program):
`[proposal(w), dao(w), question(w), squads_proposal(w), squads_multisig,
squads_multisig_program, amm_pass_base_vault(w), amm_pass_quote_vault(w),
amm_fail_base_vault(w), amm_fail_quote_vault(w), amm_base_vault(w), amm_quote_vault(w),
vault_program(=conditional_vault), vault_event_authority, token_program,
quote_vault(w), quote_vault_underlying_token_account(w), pass_quote_mint(w),
fail_quote_mint(w), pass_base_mint(w), fail_base_mint(w), base_vault(w),
base_vault_underlying_token_account(w)]`.
- Computes pass/fail TWAP from the embedded `Pool.oracle.get_twap()`, sets
  Proposal::{Passed,Failed}, CPIs `resolve_question` (signed by the proposal PDA),
  and on Passed CPIs Squads `proposal_approve` (signed by the Dao PDA) — this is
  what lets the staged `VaultTransaction` later execute.

### conditional_swap  (src: conditional_swap.rs)
`ConditionalSwapParams` Borsh: `market:Market(u8), swap_type:SwapType(u8),
input_amount:u64, min_output_amount:u64` (18 + 8 disc). `Market{Spot=0,Pass=1,Fail=2}`
(`require_neq!(market, Spot)`); `SwapType{Buy=0,Sell=1}`.
Accounts (then event_authority, program):
`[dao(w), amm_base_vault(w), amm_quote_vault(w), proposal, amm_pass_base_vault(w),
amm_pass_quote_vault(w), amm_fail_base_vault(w), amm_fail_quote_vault(w),
trader(signer), user_input_account(w), user_output_account(w), base_vault(w),
base_vault_underlying_token_account(w), quote_vault(w),
quote_vault_underlying_token_account(w), pass_base_mint(w), fail_base_mint(w),
pass_quote_mint(w), fail_quote_mint(w), conditional_vault_program,
vault_event_authority, question, token_program]`.

### spot_swap  (src: spot_swap.rs) — cranks the spot TWAP (the `kass_price` source)
`SpotSwapParams` Borsh: `input_amount:u64, swap_type:SwapType(u8), min_output_amount:u64`
(17 + 8 disc). Accounts (then event_authority, program):
`[dao(w), user_base_account(w), user_quote_account(w), amm_base_vault(w),
amm_quote_vault(w), user(signer), token_program]`.

---

## Squads v4 (`Squads-Protocol/v4@6d5235da`)

Seeds (state/seeds.rs): SEED_PREFIX=`b"multisig"`, SEED_MULTISIG=`b"multisig"`,
SEED_VAULT=`b"vault"`, SEED_TRANSACTION=`b"transaction"`, SEED_PROPOSAL=`b"proposal"`,
SEED_PROGRAM_CONFIG=`b"program_config"`, SEED_SPENDING_LIMIT=`b"spending_limit"`.
- multisig `[b"multisig", b"multisig", create_key]`
- vault `[b"multisig", multisig, b"vault", vault_index:u8le]`
- transaction `[b"multisig", multisig, b"transaction", index:u64le]`
- proposal `[b"multisig", multisig, b"transaction", index:u64le, b"proposal"]`
- program_config `[b"multisig", b"program_config"]`
- spending_limit `[b"multisig", multisig, b"spending_limit", create_key]`

Discriminators (`metadao_v06.rs`; `vault_transaction_execute` dispatch-probe-validated
against the real `squads_v4.so` in `tests/metadao_v06_cpi.rs`):
- multisig_create_v2 `32 dd c7 5d 28 f5 8b e9`
- vault_transaction_create `30 fa 4e a8 d0 e2 da d3`
- vault_transaction_execute `c2 08 a1 57 99 a4 19 ab`
- proposal_create `dc 3c 49 e0 1e 6c 4f 9f`

### vault_transaction_create  (src: vault_transaction_create.rs)
`VaultTransactionCreateArgs` Borsh: `vault_index:u8, ephemeral_signers:u8,
transaction_message:Vec<u8>(u32 len + bytes), memo:Option<String>`.
Accounts: `[multisig(w), transaction(w,init), creator(signer), rent_payer(w,signer),
system_program]`. transaction PDA uses index = multisig.transaction_index + 1.
The `transaction_message` is Squads' compact `TransactionMessage` encoding of the
inner instruction(s) — composing it (the staged `set_config` CPI) is a G3 concern.

### proposal_create  (src: proposal_create.rs)
`ProposalCreateArgs` Borsh: `transaction_index:u64, draft:bool`.
Accounts: `[multisig, proposal(w,init), creator(signer), rent_payer(w,signer),
system_program]`.

### vault_transaction_execute  (src: vault_transaction_execute.rs) — NO args
Accounts (FIXED): `[multisig, proposal(w), transaction, member(signer)]` then the
inner transaction's `remaining_accounts` (ALT accounts + the message account_keys,
in message order). The `member` is the Dao PDA (a Vote|Execute member). For a
passed proposal this `invoke_signed`s the inner `set_config`/`resolve_deadend`
into Kassandra signed by the Squads vault PDA == `Protocol.dao_authority`.

---

## F2a — `collect_meteora_damm_fees` — **PINNED (≥2 authoritative sources agree)**

> **Scope — this is MetaDAO's protocol-rake op, NOT a Kassandra dependency.**
> `collect_meteora_damm_fees` sweeps a DAO's Meteora LP fees into **MetaDAO's OWN
> vault** (`token_{a,b}_account` are `associated_token::authority =
> metadao_multisig_vault::ID` = `6awyHMsh…`), gated on **MetaDAO's keeper**
> (`require_keys_eq!(admin, metadao_admin::ID = tSTp6B6k…)` under `production`).
> **Kassandra does NOT call it and does NOT depend on it.** The builder is KEPT +
> wire-verified at THREE levels (F2a offline bytes + F2b reach-the-admin-gate live
> proof + D2 litesvm full-drive) as a faithful pin of the deployed instruction, but
> the DAO collects its OWN Meteora treasury fees via the **admin-free, DAO-owned,
> governance-authorized path (D1)** — see "### D1" below.

Previously UNDETERMINED (only NAMED as a string). Now authoritatively pinned for
the DEPLOYED futarchy **v0.6.1** from TWO cross-confirming sources that AGREE
EXACTLY on the 27-account order + roles + NO args:

- **(a) source** — `metaDAOproject/programs@c1000ed84ef6d084203ad2a9c13940fd14feb53c`
  (develop; futarchy `Cargo.toml` version = `0.6.1`, `declare_id! == FUTAREL…`):
  `programs/futarchy/src/instructions/collect_meteora_damm_fees.rs` (the
  `CollectMeteoraDammFees` `#[derive(Accounts)] #[event_cpi]` struct + nested
  `MeteoraClaimPositionFeesAccounts`) + `lib.rs:158`
  (`pub fn collect_meteora_damm_fees(ctx) -> Result<()>` — no params). There is
  no `v0.6.1` git TAG; `develop` is the 0.6.1 line (the v0.6.0 tag has only the
  embedded-AMM `collect_fees`, NOT this Meteora instruction).
- **(b) on-chain Anchor IDL** of `FUTARELBfJfQ8RDGhg1wdhddq1odMAJUePHFuBYfUxKq`
  (legacy IDL account at `Cgg4TGESEzsewehRcFnbDaGmBznwkH23Ro1HqWz5VdtG` =
  `createWithSeed(PDA([],prog), "anchor:idl", prog)`, inflated; `version 0.6.1`),
  instruction `collectMeteoraDammFees` — same 27 accounts, same isMut/isSigner,
  `args: []`.

Disc `sha256("global:collect_meteora_damm_fees")[..8]` = **`8b d4 69 76 7e 36 d6 8f`**.
NO positional args → instruction data = the 8-byte disc only.

The handler builds a cp-amm `claim_position_fee` CPI, stages it in the DAO's
Squads multisig (`vault_transaction_create` → `proposal_create` →
`proposal_approve` → `vault_transaction_execute`, all internal CPIs), so the DAO's
Squads vault signs the actual claim. Fees are swept to the MetaDAO protocol
vault's ATAs (`metadao_multisig_vault::ID` = `6awyHMsh…`), NOT the DAO's own vault.

Account order (then the `#[event_cpi]` tail), role w=writable / s=signer:
```
 0 dao                              w        Account<Dao> (mut)
 1 admin                            w s      rent payer; == metadao_admin::ID (tSTp6B6k…) under `production`
 2 squads_multisig                  w        PDA [b"multisig", b"multisig", dao] @ squads
 3 squads_multisig_vault            w        PDA [b"multisig", multisig, b"vault", 0u8] @ squads
 4 squads_multisig_vault_transaction w       PDA [b"multisig", multisig, b"transaction", index:u64] @ squads
 5 squads_multisig_proposal         w        PDA [.. b"transaction", index, b"proposal"] @ squads
 6 squads_permissionless_account      s      == permissionless_account::id() (EP3SoC2…)
 7 damm_v2_program                            cp-amm program id (cpamd…)
 8 damm_v2_event_authority                    cp-amm PDA [b"__event_authority"]
 9 pool_authority                             == pool_authority::ID (HLnpSz9h…) = cp-amm PDA [b"pool_authority"]
10 pool                                       cp-amm Pool
11 position                         w         cp-amm Position
12 token_a_account                  w         ATA(6awyHMsh…, base_mint)
13 token_b_account                  w         ATA(6awyHMsh…, quote_mint)
14 token_a_vault                    w         cp-amm base vault
15 token_b_vault                    w         cp-amm quote vault
16 token_a_mint                               == dao.base_mint
17 token_b_mint                               == dao.quote_mint
18 position_nft_account                       token account holding the position NFT
19 owner                                      position owner (usually the DAO squads vault)
20 token_a_program                            SPL Token (base)
21 token_b_program                            SPL Token (quote)
22 system_program
23 token_program
24 squads_program                             SQDS4ep6…
25 event_authority                            futarchy PDA [b"__event_authority"]
26 program                                    FUTARCHY_ID
```
Builder: `futarchy.collectMeteoraDammFees` (`instructions.ts`); offline byte/meta
test in `test/futarchy.test.ts` (`describe("collect_meteora_damm_fees …")`).
Confidence: HIGH — both sources agree exactly.

### F2b — reach-the-admin-gate LIVE proof (full sweep DEFERRED)

The 27-account wire format is now also **verified LIVE against the DEPLOYED
futarchy binary** by the gated fork E2E
`sdks/oracles/ts/test/surfpool/futarchy-meteora-treasury-e2e.test.ts`. The handler's admin
check lives in `validate()` (`collect_meteora_damm_fees.rs:117`,
`#[cfg(feature = "production")] require_keys_eq!(admin, metadao_admin::ID,
InvalidAdmin)`) and is wired via `#[access_control(ctx.accounts.validate())]`
(`lib.rs:157`), so it runs ONLY AFTER Anchor's `try_accounts` deserializes +
constraint-checks all 27 accounts. The E2E runs the real `initialize_dao`
(genuine `Dao` + Squads multisig/vault on the fork), fabricates the two
fee-recipient ATAs (owned by `metadao_multisig_vault::ID` = `6awyHMsh…`), builds
the `collectMeteoraDammFees` ix with a STAND-IN admin (≠ `tSTp6B6k…`) + the public
permissionless signer, submits it to the deployed futarchy, and asserts it is
rejected SPECIFICALLY at `InvalidAdmin` (Anchor custom **6020**) — captured log:
`AnchorError thrown in …/collect_meteora_damm_fees.rs:119. Error Code:
InvalidAdmin. Error Number: 6020`. Because that gate is only reachable once all 27
accounts pass their layout/PDA/associated-token/address constraints, a
wire-format bug would have failed EARLIER (`ConstraintSeeds`/`ConstraintRaw`/
`AccountNotInitialized`/`ConstraintAddress`) — so reaching 6020 PROVES the layout
deserializes on the deployed binary. A second arm cross-verifies the Squads
multisig/vault PDA derivations against a REAL mainnet `Dao`'s recorded fields.

**DEFERRED — full live sweep.** The end-to-end fee collection cannot be driven on
a fork: the `production` admin (`tSTp6B6k…`, a MetaDAO-controlled key) must sign,
and the handler then stages the cp-amm `claim_position_fee` through the DAO's
Squads permissionless-member chain — so a real sweep needs a MetaDAO-admin
context. The builder is wire-verified (F2a byte test) + layout-verified live (this
F2b reach-the-admin-gate proof); the live sweep is deferred.

### D1 — DAO-owned admin-free Meteora fee claim (the FIX; driven LIVE on surfpool)

The CORRECT/supported treasury path — the DAO collects its OWN Meteora fees
WITHOUT any MetaDAO admin — is proven end-to-end LIVE on the mainnet fork by
`sdks/oracles/ts/test/surfpool/dao-meteora-treasury-e2e.test.ts`. It does NOT use
`collect_meteora_damm_fees` at all. Instead it composes the DAO's Squads
governance directly with the M1 cp-amm `claim_position_fee` builder:

1. **The Squads vault OWNS the cp-amm position.** cp-amm `initialize_pool` mints
   the FUNDED first position's NFT to `creator`, which is an `UncheckedAccount`
   with `token::authority = creator` (see
   `MeteoraAg/damm-v2@bdd8a1e` `ix_initialize_pool.rs:74,325`), so calling it with
   `creator == the DAO's Squads vault` makes the vault the position owner outright
   — the payer funds the liquidity but the NFT authority is the vault. Verified by
   decoding the position NFT account's authority (owner @ offset 32) == the vault.
   (Route (a); no NFT transfer needed. `create_position` is identical —
   `owner: UncheckedAccount`, `token::authority = owner` — for the empty case.)
2. **Fees accrue + are claimed via the DAO's OWN governance.** A→B swaps accrue a
   token-B (quote) LP fee. The cp-amm `claim_position_fee` ix (`signer == the
   vault`, recipients == the DAO's OWN vault-owned ATAs — NOT `6awyHMsh…`) is
   compiled into a Squads compact `TransactionMessage` and staged via
   `vault_transaction_create` + `proposal_create`. A REAL futarchy proposal is
   then driven to a PASS TWAP verdict, whose `finalize_proposal` CPI-approves the
   Squads proposal (**threshold 1, the sole Vote member is the Dao PDA — so a
   passing futarchy proposal is the ONLY way the DAO's vault ever acts; there is
   no shortcut approve**). `vault_transaction_execute` (member = the public
   permissionless member) then `invoke_signed`s the cp-amm claim AS THE VAULT
   (`cp-amm` sees `signer == position_nft_account.owner == vault` → its
   `assert_authority` passes). This is EXACTLY the CPI the MetaDAO op does
   internally, but authorized by the DAO's own futarchy verdict and paying the DAO,
   not MetaDAO.
3. **Asserted:** the DAO's OWN ATA balance rose by a NONZERO fee; the vault
   position's `fee_{a,b}_pending` cleared to 0; and NO `metadao_admin` (`tSTp6B6k…`)
   or `metadao_multisig_vault` (`6awyHMsh…`) appears in ANY account of the inner
   claim, the staged Squads message, or the `vault_transaction_execute`
   remaining-accounts. DAO-owned, governance-authorized, admin-free. ~14s live.

**⇒ This is the Kassandra treasury path.** `collect_meteora_damm_fees` (F2a/F2b/D2)
is MetaDAO's separate protocol-rake op — kept + verified, but not a dependency.

### D2 — litesvm FULL-DRIVE to COMPLETION (the sweep F2b had to defer)

The DEFERRED full sweep is now **driven to COMPLETION** — past the admin gate,
through the entire handler — by the gated litesvm test
`sdks/oracles/ts/test/meteora-collect-litesvm.test.ts`. litesvm can do the one thing surfpool
cannot: `svm.withSigverify(false)`, which lets the REAL production admin
(`tSTp6B6k…`) be presented as a required-but-UNSIGNED signer (its slot filled with
a zero signature), so the handler runs past `require_keys_eq!(admin,
metadao_admin::ID)` without forging a MetaDAO signature.

litesvm hosts all **three real deployed programs** — futarchy (`FUTAREL…`, 1.24
MB), cp-amm (`cpamd…`, 2.17 MB) and Squads v4 (`SQDS4ep6…`, 1.47 MB) — from
committed `.so` fixtures (`test/fixtures/programs/`, dumped via `solana program
dump`), plus the real Squads `ProgramConfig` + a public cp-amm `Config` as account
fixtures; SPL Token / Token-2022 / ATA come from litesvm builtins. The test builds
GENUINE state with the real instructions: `initialize_dao` (→ Dao + Squads
multisig/vault via the futarchy→Squads `multisig_create_v2` CPI), cp-amm
`initialize_pool` (the first position OWNED BY the DAO's Squads vault — `creator`
is an UncheckedAccount, so `creator == vault`), and `swap`s that accrue a real LP
fee (token-B side on this Config, per the M2 finding). Then it drives
`collectMeteoraDammFees`: the internal `vault_transaction_create →
proposal_create → proposal_approve → vault_transaction_execute` chain and the
inner cp-amm `claim_position_fee` all execute (visible in the CPI logs), and the
accrued fee is **swept to the MetaDAO vault's ATAs** (`ATA(6awyHMsh…, {base,quote}
mint)`) — the test decodes those ATAs before/after and asserts the balance ROSE by
a nonzero fee, and that the Position's `fee_b_pending` cleared. Notes: the tx needs
`setComputeUnitLimit(1_400_000)` (the 4-deep CPI chain exceeds the 200k default), a
mainnet-like Clock (cp-amm's fee scheduler reads it), and the admin funded as the
Squads rent-payer.

So the F2a 27-account wire format is now proven at THREE levels: offline bytes
(F2a), reach-the-admin-gate on the deployed binary (F2b, surfpool), and END-TO-END
completion (D2, litesvm). **The `withSigverify(false)` bypass is a TEST-ONLY
device** — the production program still requires the real MetaDAO keeper's
signature. This is a completeness proof of **MetaDAO's protocol-rake op**, which
**Kassandra does NOT call**: the DAO collects its OWN Meteora treasury fees
admin-free via its Squads vault (the D1 path,
`sdks/oracles/ts/test/surfpool/dao-meteora-treasury-e2e.test.ts`).

---

## Meteora DAMM v2 — **DONE (SDK builders + decoders; offsets verified vs deployed)**

The v0.6 conditional pass/fail VERDICT markets are the futarchy program's OWN
EMBEDDED AMM (`Dao.amm` `PoolState::Futarchy{pass,fail,spot}`), driven by
`launch_proposal` + `conditional_swap` + `finalize_proposal` (above) — NOT Meteora.
The TWAP verdict comes from the embedded `Pool.oracle.get_twap()`. Meteora cp-amm
is only used for the DAO's SPOT liquidity / fee collection (`collect_meteora_damm_fees`,
`provide_liquidity`), which the proposal→TWAP→Squads→set_config loop does NOT need.

The full Meteora cp-amm spot-path SDK now lives in `sdk/src/meteora/` (M1/M2/F1): the
6 position-based builders (`initializePool`, `createPosition`, `addLiquidity`,
`removeLiquidity`, `swap`, `claimPositionFee`) + the `Pool`/`Position` zero-copy
decoders, all byte-sourced from `MeteoraAg/damm-v2@bdd8a1e`. The previously-deferred
unknown — the `Pool` field offsets behind the nested C-padded `PoolFeesStruct`
(`sqrt_price` @ abs 456, reserves @ 680/688, etc.) — is RESOLVED: the repo source
pins every offset AND they are now VERIFIED against the DEPLOYED binary by the gated
mainnet-fork E2E (`sdks/oracles/ts/test/surfpool/meteora-spot-e2e.test.ts`), which clones a real
public cp-amm `Config`, drives init→add→swap→create_position through the real program
over RPC, and decodes the resulting Pool/Position (sqrt_price moved the correct
direction, reserves match the live vaults, unlocked_liquidity matches the deposit) —
plus decodes a genuine mainnet pool (sqrt_price² ≈ reserve ratio). Not STOP-REPORTED
anymore.

**F1 — ALL 6 builders now DRIVEN LIVE (not just unit-tested).** The two remaining
builders `claimPositionFee` + `removeLiquidity` are now driven through the deployed
cp-amm in the same E2E (previously unit-tested-only). `claimPositionFee` sweeps a
NONZERO swap-accrued LP fee (on the cloned public Config the `collect_fee_mode`
collects in token B for both directions, so `fee_b_pending` is nonzero after A→B
swaps; the owner's token-B account rises by exactly the pending fee and the position
clears). `removeLiquidity` withdraws all `unlocked_liquidity` (position/pool liquidity
drop by the exact delta, both reserves fall, owner receives the withdrawn amounts).
