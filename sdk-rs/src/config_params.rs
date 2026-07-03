//! `set_config` (Ix 14) governable-parameter payload.

use kassandra_program::config::{
    CHALLENGE_FAIL_USDC_FEE_DEN, CHALLENGE_FAIL_USDC_FEE_NUM, CHALLENGE_SUCCESS_KASS_FEE_DEN,
    CHALLENGE_SUCCESS_KASS_FEE_NUM, FEE_EMA_HALFLIFE_SECS, FEE_EMA_INCREMENT, FEE_PER_EMA_UNIT,
    FLIP_SLASH_DEN, FLIP_SLASH_NUM, MARKET_THRESHOLD_DEN, MARKET_THRESHOLD_NUM, PHASE_WINDOW,
    PROPOSAL_WINDOW, THRESHOLD_DEN, THRESHOLD_NUM,
};

/// The full set of `Protocol`-resident governable params for `set_config`
/// (Ix 14), in the fixed wire order. [`ConfigParams::defaults`] returns a VALID
/// baseline (passes every bound); callers mutate one field to drive rejection
/// paths in tests. Packed into a fixed 176-byte little-endian payload by
/// [`ConfigParams::to_payload`].
#[derive(Clone, Copy, Debug)]
pub struct ConfigParams {
    pub emission_num: u64,
    pub emission_den: u64,
    pub total_supply_cap: u64,
    pub fee_ema_halflife: i64,
    pub fee_per_ema_unit: u64,
    pub fee_ema_increment: u64,
    pub threshold_num: u64,
    pub threshold_den: u64,
    pub market_threshold_num: u64,
    pub market_threshold_den: u64,
    pub flip_slash_num: u64,
    pub flip_slash_den: u64,
    pub phase_window: i64,
    pub proposal_window: i64,
    pub fact_vote_slash_num: u64,
    pub fact_vote_slash_den: u64,
    pub reward_proposer_weight: u64,
    pub reward_fact_weight: u64,
    pub challenge_fail_usdc_fee_num: u64,
    pub challenge_fail_usdc_fee_den: u64,
    pub challenge_success_kass_fee_num: u64,
    pub challenge_success_kass_fee_den: u64,
}

impl ConfigParams {
    /// A VALID baseline mirroring `init_protocol`'s defaults, except the reward
    /// weights are set to 1/1 (init defaults both to 0, but `set_config`
    /// requires at least one reward weight > 0).
    pub fn defaults() -> Self {
        Self {
            emission_num: 0,
            emission_den: 1,
            total_supply_cap: 0,
            fee_ema_halflife: FEE_EMA_HALFLIFE_SECS,
            fee_per_ema_unit: FEE_PER_EMA_UNIT,
            fee_ema_increment: FEE_EMA_INCREMENT,
            threshold_num: THRESHOLD_NUM,
            threshold_den: THRESHOLD_DEN,
            market_threshold_num: MARKET_THRESHOLD_NUM as u64,
            market_threshold_den: MARKET_THRESHOLD_DEN as u64,
            flip_slash_num: FLIP_SLASH_NUM,
            flip_slash_den: FLIP_SLASH_DEN,
            phase_window: PHASE_WINDOW,
            proposal_window: PROPOSAL_WINDOW,
            fact_vote_slash_num: 0,
            fact_vote_slash_den: 1,
            reward_proposer_weight: 1,
            reward_fact_weight: 1,
            challenge_fail_usdc_fee_num: CHALLENGE_FAIL_USDC_FEE_NUM,
            challenge_fail_usdc_fee_den: CHALLENGE_FAIL_USDC_FEE_DEN,
            challenge_success_kass_fee_num: CHALLENGE_SUCCESS_KASS_FEE_NUM,
            challenge_success_kass_fee_den: CHALLENGE_SUCCESS_KASS_FEE_DEN,
        }
    }

    /// Pack into the fixed 176-byte little-endian wire layout `set_config` expects.
    pub fn to_payload(self) -> [u8; 176] {
        let mut out = [0u8; 176];
        let fields: [[u8; 8]; 22] = [
            self.emission_num.to_le_bytes(),
            self.emission_den.to_le_bytes(),
            self.total_supply_cap.to_le_bytes(),
            self.fee_ema_halflife.to_le_bytes(),
            self.fee_per_ema_unit.to_le_bytes(),
            self.fee_ema_increment.to_le_bytes(),
            self.threshold_num.to_le_bytes(),
            self.threshold_den.to_le_bytes(),
            self.market_threshold_num.to_le_bytes(),
            self.market_threshold_den.to_le_bytes(),
            self.flip_slash_num.to_le_bytes(),
            self.flip_slash_den.to_le_bytes(),
            self.phase_window.to_le_bytes(),
            self.proposal_window.to_le_bytes(),
            self.fact_vote_slash_num.to_le_bytes(),
            self.fact_vote_slash_den.to_le_bytes(),
            self.reward_proposer_weight.to_le_bytes(),
            self.reward_fact_weight.to_le_bytes(),
            self.challenge_fail_usdc_fee_num.to_le_bytes(),
            self.challenge_fail_usdc_fee_den.to_le_bytes(),
            self.challenge_success_kass_fee_num.to_le_bytes(),
            self.challenge_success_kass_fee_den.to_le_bytes(),
        ];
        for (i, f) in fields.iter().enumerate() {
            out[i * 8..i * 8 + 8].copy_from_slice(f);
        }
        out
    }
}
