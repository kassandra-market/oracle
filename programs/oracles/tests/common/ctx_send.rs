use super::*;

/// Human-readable name for an instruction discriminant (the first payload byte),
/// so the CU meter can label every metered transaction. Mirrors
/// [`kassandra_oracles_program::instruction::Ix`].
pub fn ix_name(disc: u8) -> &'static str {
    match disc {
        0 => "submit_fact",
        1 => "vote_fact",
        2 => "finalize_facts",
        3 => "submit_ai_claim",
        4 => "open_challenge",
        5 => "settle_challenge",
        6 => "finalize_oracle",
        7 => "advance_phase",
        8 => "finalize_ai_claims",
        9 => "init_protocol",
        10 => "create_oracle",
        11 => "propose",
        12 => "finalize_proposals",
        13 => "set_governance",
        14 => "set_config",
        15 => "resolve_deadend",
        16 => "kass_price",
        17 => "claim_proposer",
        18 => "claim_fact",
        19 => "claim_fact_vote",
        20 => "close_ai_claim",
        21 => "close_market",
        22 => "sweep_oracle",
        _ => "unknown",
    }
}

/// Compute-unit meter. Every successful transaction the harness sends via
/// [`TestCtx::send`] records `(instruction, compute_units_consumed)` here, keyed
/// by the instruction discriminant. A metering test can then report per-
/// instruction CU and guard against regressions ([`TestCtx::cu_report`] /
/// [`TestCtx::cu_max`]).
#[derive(Default)]
pub struct CuMeter {
    rows: Vec<(&'static str, u64)>,
}

impl CuMeter {
    fn record(&mut self, name: &'static str, cu: u64) {
        self.rows.push((name, cu));
    }

    /// The maximum CU observed for `name`, or `None` if it was never sent.
    pub fn max(&self, name: &str) -> Option<u64> {
        self.rows
            .iter()
            .filter(|(n, _)| *n == name)
            .map(|(_, c)| *c)
            .max()
    }

    /// Max CU per instruction seen (alphabetical, for a stable report).
    pub fn max_by_ix(&self) -> BTreeMap<&'static str, u64> {
        let mut m: BTreeMap<&'static str, u64> = BTreeMap::new();
        for (n, c) in &self.rows {
            let e = m.entry(*n).or_insert(0);
            *e = (*e).max(*c);
        }
        m
    }

    /// A human-readable table: instruction · max CU · number of calls.
    pub fn report(&self) -> String {
        use std::fmt::Write as _;
        let mut counts: BTreeMap<&'static str, u64> = BTreeMap::new();
        for (n, _) in &self.rows {
            *counts.entry(*n).or_insert(0) += 1;
        }
        let mut s = String::from("\n=== compute-unit metering (max CU per instruction) ===\n");
        for (n, cu) in self.max_by_ix() {
            let _ = writeln!(s, "  {n:<20} {cu:>7} CU   (x{})", counts[n]);
        }
        s.push_str("=======================================================\n");
        s
    }
}

impl TestCtx {
    // ----- transaction submission --------------------------------------------

    /// Sign and submit a single-instruction transaction, returning the LiteSVM
    /// result so tests can assert `Ok`/`Err` and introspect the
    /// [`TransactionError`](solana_transaction_error::TransactionError).
    ///
    /// The transaction is signed by the payer (fee payer) plus every keypair in
    /// `signers`. The blockhash is expired and re-fetched on each call so that
    /// two otherwise-identical transactions (same instruction + signers) get
    /// distinct signatures and never collide as duplicates.
    #[allow(clippy::result_large_err)]
    pub fn send(&mut self, ix: Instruction, signers: &[&Keypair]) -> TransactionResult {
        // Rotate the blockhash to guarantee signature uniqueness across calls.
        self.svm.expire_blockhash();
        let blockhash = self.svm.latest_blockhash();
        // Label the CU record by the instruction discriminant (first payload byte).
        let name = ix_name(ix.data.first().copied().unwrap_or(u8::MAX));
        let mut all_signers: Vec<&Keypair> = Vec::with_capacity(signers.len() + 1);
        all_signers.push(&self.payer);
        all_signers.extend_from_slice(signers);
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&self.payer.pubkey()),
            &all_signers,
            blockhash,
        );
        let res = self.svm.send_transaction(tx);
        if let Ok(meta) = &res {
            self.cu_meter.record(name, meta.compute_units_consumed);
        }
        res
    }

    /// Send `ix` expecting success and return the compute units it consumed (also
    /// recorded in the CU meter). Convenience over `send(..).expect(..).compute_
    /// units_consumed` for metering call sites.
    pub fn send_cu(&mut self, ix: Instruction, signers: &[&Keypair]) -> u64 {
        self.send(ix, signers)
            .expect("send_cu: transaction should succeed")
            .compute_units_consumed
    }

    /// The maximum CU observed for an instruction (by name; see [`ix_name`]).
    pub fn cu_max(&self, name: &str) -> Option<u64> {
        self.cu_meter.max(name)
    }

    /// A human-readable per-instruction CU report over everything sent so far.
    pub fn cu_report(&self) -> String {
        self.cu_meter.report()
    }

    /// Sign and submit a MULTI-instruction transaction (e.g. a ComputeBudget
    /// prefix plus a CPI-heavy instruction). Mirrors [`TestCtx::send`] but takes
    /// a slice of instructions. Signed by the payer plus every keypair in
    /// `signers`; the blockhash is rotated for signature uniqueness.
    #[allow(clippy::result_large_err)]
    pub fn send_many(&mut self, ixs: &[Instruction], signers: &[&Keypair]) -> TransactionResult {
        self.svm.expire_blockhash();
        let blockhash = self.svm.latest_blockhash();
        let mut all_signers: Vec<&Keypair> = Vec::with_capacity(signers.len() + 1);
        all_signers.push(&self.payer);
        all_signers.extend_from_slice(signers);
        let tx = Transaction::new_signed_with_payer(
            ixs,
            Some(&self.payer.pubkey()),
            &all_signers,
            blockhash,
        );
        self.svm.send_transaction(tx)
    }

    /// Current on-chain unix timestamp from the `Clock` sysvar.
    pub fn now(&self) -> i64 {
        self.svm.get_sysvar::<Clock>().unix_timestamp
    }

    /// Advance the `Clock`: add `seconds` to `unix_timestamp` and bump `slot`
    /// by exactly **1** (not proportional to `seconds`). This is enough to
    /// cross `phase_ends_at`, which is keyed off `unix_timestamp`.
    ///
    /// NOTE: the later TWAP tasks (11-12) reason about *slots*, so they will
    /// likely need a `warp_slots` variant that advances the slot proportionally.
    /// Not built yet (YAGNI).
    pub fn warp(&mut self, seconds: i64) {
        let mut clock = self.svm.get_sysvar::<Clock>();
        clock.unix_timestamp += seconds;
        clock.slot += 1;
        self.svm.set_sysvar(&clock);
    }

    /// Advance the `Clock` by `seconds` of unix time AND `slots` of slot height.
    /// The TWAP tasks (11-12) reason about *slots* (the MetaDAO AMM records an
    /// observation only once per `ONE_MINUTE_IN_SLOTS == 150` slots and weights
    /// the aggregator by elapsed slots), so they need to move the slot height
    /// independently of wall-clock seconds.
    pub fn warp_slots(&mut self, seconds: i64, slots: u64) {
        let mut clock = self.svm.get_sysvar::<Clock>();
        clock.unix_timestamp += seconds;
        clock.slot += slots;
        self.svm.set_sysvar(&clock);
    }

    /// Read the current `Clock` slot height.
    pub fn slot(&self) -> u64 {
        self.svm.get_sysvar::<Clock>().slot
    }
}
