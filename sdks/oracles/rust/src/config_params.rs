//! `set_config` (Ix 14) governable-parameter payload.

use kassandra_program::config::{
    CHALLENGE_FAIL_USDC_FEE_DEN, CHALLENGE_FAIL_USDC_FEE_NUM, CHALLENGE_SUCCESS_KASS_FEE_DEN,
    CHALLENGE_SUCCESS_KASS_FEE_NUM, FEE_EMA_HALFLIFE_SECS, FEE_EMA_INCREMENT, FEE_PER_EMA_UNIT,
    FLIP_SLASH_DEN, FLIP_SLASH_NUM, MARKET_THRESHOLD_DEN, MARKET_THRESHOLD_NUM, PHASE_WINDOW,
    PROPOSAL_WINDOW, STAKE_FLOOR_EMA_CAP, STAKE_FLOOR_EMA_THRESHOLD, STAKE_FLOOR_MAX,
    THRESHOLD_DEN, THRESHOLD_NUM,
};

/// The full set of `Protocol`-resident governable params for `set_config`
/// (Ix 14), in the fixed wire order. [`ConfigParams::defaults`] returns a VALID
/// baseline (passes every bound); callers mutate one field to drive rejection
/// paths in tests. Packed into a fixed 200-byte little-endian payload by
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
    pub stake_floor_ema_threshold: u64,
    pub stake_floor_ema_cap: u64,
    pub stake_floor_max: u64,
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
            stake_floor_ema_threshold: STAKE_FLOOR_EMA_THRESHOLD,
            stake_floor_ema_cap: STAKE_FLOOR_EMA_CAP,
            stake_floor_max: STAKE_FLOOR_MAX,
        }
    }

    /// Pack into the fixed 200-byte little-endian wire layout `set_config` expects.
    pub fn to_payload(self) -> [u8; 200] {
        let mut out = [0u8; 200];
        let fields: [[u8; 8]; 25] = [
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
            self.stake_floor_ema_threshold.to_le_bytes(),
            self.stake_floor_ema_cap.to_le_bytes(),
            self.stake_floor_max.to_le_bytes(),
        ];
        for (i, f) in fields.iter().enumerate() {
            out[i * 8..i * 8 + 8].copy_from_slice(f);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_payload_is_200_bytes_le_in_field_order() {
        // Each field carries its 1-based wire position as its value, so the k-th
        // 8-byte little-endian window MUST read back k — pinning both the
        // field→offset order and the LE packing. A struct/array reorder (or a
        // field dropped/duplicated) breaks exactly one window and fails here.
        let p = ConfigParams {
            emission_num: 1,
            emission_den: 2,
            total_supply_cap: 3,
            fee_ema_halflife: 4,
            fee_per_ema_unit: 5,
            fee_ema_increment: 6,
            threshold_num: 7,
            threshold_den: 8,
            market_threshold_num: 9,
            market_threshold_den: 10,
            flip_slash_num: 11,
            flip_slash_den: 12,
            phase_window: 13,
            proposal_window: 14,
            fact_vote_slash_num: 15,
            fact_vote_slash_den: 16,
            reward_proposer_weight: 17,
            reward_fact_weight: 18,
            challenge_fail_usdc_fee_num: 19,
            challenge_fail_usdc_fee_den: 20,
            challenge_success_kass_fee_num: 21,
            challenge_success_kass_fee_den: 22,
            stake_floor_ema_threshold: 23,
            stake_floor_ema_cap: 24,
            stake_floor_max: 25,
        };
        let payload = p.to_payload();
        assert_eq!(payload.len(), 200);
        for k in 0..25usize {
            let off = k * 8;
            let word = u64::from_le_bytes(payload[off..off + 8].try_into().unwrap());
            assert_eq!(
                word,
                k as u64 + 1,
                "field #{} at offset {off} out of order",
                k + 1
            );
        }
    }

    #[test]
    fn defaults_encode_a_valid_full_width_payload() {
        let payload = ConfigParams::defaults().to_payload();
        assert_eq!(payload.len(), 200);
        // emission_den (field #2 @8) and reward_proposer_weight (#17 @128) default
        // to 1 — the struct's non-zero baselines land at their pinned offsets.
        assert_eq!(u64::from_le_bytes(payload[8..16].try_into().unwrap()), 1);
        assert_eq!(u64::from_le_bytes(payload[128..136].try_into().unwrap()), 1);
    }
}
