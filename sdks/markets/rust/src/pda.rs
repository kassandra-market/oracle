use crate::PROGRAM_ID;
use solana_sdk::pubkey::Pubkey;

/// The BPF Upgradeable Loader — owns every upgradeable program's `ProgramData`
/// account. Mirror of `guards::BPF_UPGRADEABLE_LOADER_ID`.
pub const BPF_UPGRADEABLE_LOADER_ID: Pubkey =
    solana_sdk::pubkey!("BPFLoaderUpgradeab1e11111111111111111111111");

pub fn config() -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"config"], &PROGRAM_ID)
}

/// The BPF-Upgradeable-Loader `ProgramData` account of `program_id`:
/// `find_program_address([program_id], BPF_UPGRADEABLE_LOADER_ID)`. This account
/// stores the program's `upgrade_authority`, which `init_config` requires the
/// caller to be. Seeded from `program_id`, derived under the loader.
pub fn program_data(program_id: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[program_id.as_ref()], &BPF_UPGRADEABLE_LOADER_ID)
}
/// Per-outcome binary sub-market PDA: `[b"market", oracle, [outcome_index]]`.
/// Binary markets use `outcome_index = 0`.
pub fn market(oracle: &Pubkey, outcome_index: u8) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"market", oracle.as_ref(), &[outcome_index]], &PROGRAM_ID)
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

/// Market-PDA-owned cYES holder (transient split destination): `[b"cyes", market]`.
pub fn market_cyes(market: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"cyes", market.as_ref()], &PROGRAM_ID)
}
/// Market-PDA-owned cNO holder (transient split destination): `[b"cno", market]`.
pub fn market_cno(market: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"cno", market.as_ref()], &PROGRAM_ID)
}
/// Market-PDA-owned LP token account holding seeded liquidity: `[b"lp_vault", market]`.
pub fn lp_vault(market: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"lp_vault", market.as_ref()], &PROGRAM_ID)
}
