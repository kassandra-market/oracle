use super::*;

// ===========================================================================
// Terminal-oracle seeding for the S2 claim-and-close tests.
// ===========================================================================

/// One proposer to seed into a TERMINAL oracle (Task S2 claims).
#[derive(Clone, Copy, Debug)]
pub struct ClaimProposerSpec {
    pub bond: u64,
    /// Post-AI-claim vote; compared to `resolved_option` for the correct/wrong split.
    pub claim_option: u8,
    pub disqualified: bool,
    /// KASS already forfeited to `bond_pool` (only meaningful when disqualified).
    pub slashed_amount: u64,
}

/// One fact vote to seed under a fact.
#[derive(Clone, Copy, Debug)]
pub struct ClaimVoteSpec {
    pub stake: u64,
    pub kind: u8, // VOTE_APPROVE / VOTE_DUPLICATE
}

/// One fact (submitter + its votes) to seed into a TERMINAL oracle.
#[derive(Clone, Debug)]
pub struct ClaimFactSpec {
    pub stake: u64, // submitter stake
    pub agreed: bool,
    pub duplicate: bool,
    pub votes: Vec<ClaimVoteSpec>,
}

/// A seeded claimant account: the program-owned account to be closed, its
/// signing authority, the authority-owned KASS destination (starts at 0), and
/// the expected KASS entitlement per the matrix.
pub struct SeededClaim {
    pub account: Pubkey,
    pub authority: Keypair,
    pub dest_kass: Pubkey,
    pub expected: u64,
    pub kind: u8, // for votes; ignored otherwise
}

/// A seeded fact: its submitter claim plus the per-vote claims.
pub struct SeededFactClaim {
    pub submitter: SeededClaim,
    pub votes: Vec<SeededClaim>,
}

/// A fully-seeded terminal oracle and everything the claim tests need.
pub struct TerminalSeed {
    pub oracle: Pubkey,
    pub nonce: u64,
    pub bump: u8,
    pub stake_vault: Pubkey,
    pub vault_initial: u64,
    pub reward_pool: u64,
    pub total_correct_proposer_stake: u64,
    pub total_approved_fact_stake: u64,
    pub resolved_option: u8,
    pub proposers: Vec<SeededClaim>,
    pub facts: Vec<SeededFactClaim>,
}

/// Reward cohort weights the terminal seeder stamps (mirrors the real defaults).
const SEED_PW: u64 = kassandra_oracles_program::config::REWARD_PROPOSER_WEIGHT;
const SEED_FW: u64 = kassandra_oracles_program::config::REWARD_FACT_WEIGHT;

impl TestCtx {
    /// Fabricate an oracle in a TERMINAL phase (`Resolved` or `InvalidDeadend`)
    /// with the given proposers/facts/votes, a stake vault funded to the
    /// post-settlement balance, and self-consistent resolution stamps
    /// (`reward_pool`, `total_correct_proposer_stake`, `total_approved_fact_stake`).
    ///
    /// The "slashed pool" = Σ `slashed_amount` (disqualified OR flip-slashed) + Σ
    /// rejected fact submitter stake + Σ rejected-fact approve-voter slash (floor
    /// `num/den`). On **Resolved** it is the distributable `reward_pool` and the
    /// vault holds the full `gross` (Σ bonds + stakes), so a complete claim sweep
    /// drains it to floor-division dust. On **InvalidDeadend** it is instead the
    /// amount the finalize site BURNED out of the vault (a dead-end distributes
    /// nothing): the vault is funded with `gross − slashed_pool`, `reward_pool ==
    /// 0`, the slashed pool is recorded on `bond_pool`, and the claims (rejected
    /// submitters/voters forfeit, survivors get `bond − slashed_amount`) drain it
    /// to dust. Each claimant's `expected` entitlement is precomputed via the
    /// program's own [`reward`] helpers.
    ///
    /// `slash_num/slash_den` is the approve-voter slash fraction stamped on the
    /// oracle (`1/2` matches the real default). To keep the aggregate
    /// `bond_pool` counter equal to the per-voter physical slash (no dust gap),
    /// callers should give rejected-fact approve votes EVEN stakes when
    /// `slash_den == 2`.
    pub fn seed_terminal_oracle(
        &mut self,
        phase: Phase,
        resolved_option: u8,
        proposers: &[ClaimProposerSpec],
        facts: &[ClaimFactSpec],
        slash_num: u64,
        slash_den: u64,
    ) -> TerminalSeed {
        let resolved = phase == Phase::Resolved;

        let nonce = self.next_nonce;
        self.next_nonce += 1;
        let (oracle_pda, bump) = Self::oracle_pda(&self.program_id, nonce);

        // ----- totals + reward_pool (the resolution stamps) -----------------
        let mut total_correct: u64 = 0;
        for p in proposers {
            if resolved && !p.disqualified && p.claim_option == resolved_option {
                total_correct += p.bond;
            }
        }
        let mut total_approved: u64 = 0;
        for f in facts {
            if resolved && f.agreed {
                let approve: u64 = f
                    .votes
                    .iter()
                    .filter(|v| v.kind == VOTE_APPROVE)
                    .map(|v| v.stake)
                    .sum();
                total_approved += f.stake + approve;
            }
        }
        // The slashed pool = Σ proposer slashes + Σ rejected-fact (submitter stake
        // + floor approve-voter slash). On Resolved it is the distributable
        // `reward_pool`; on InvalidDeadend it is the amount the finalize site
        // BURNED back, so the vault is funded with `gross − slashed_pool` and the
        // counter is stamped on `bond_pool` (mirroring the on-chain post-burn
        // terminal state). This is the same formula on both phases.
        let mut slashed_pool: u64 = 0;
        for p in proposers {
            // ANY slashed_amount (disqualified OR flip-slashed-but-surviving).
            slashed_pool += p.slashed_amount;
        }
        for f in facts {
            if !f.agreed && !f.duplicate {
                // Rejected: submitter full forfeit + approve-voter floor slash.
                let approve: u64 = f
                    .votes
                    .iter()
                    .filter(|v| v.kind == VOTE_APPROVE)
                    .map(|v| v.stake)
                    .sum();
                slashed_pool += f.stake;
                slashed_pool +=
                    ((approve as u128) * (slash_num as u128) / (slash_den as u128)) as u64;
            }
        }
        let reward_pool: u64 = if resolved { slashed_pool } else { 0 };
        // On a dead-end the slashed pool was burned out of the vault at finalize.
        let burn_pool: u64 = if resolved { 0 } else { slashed_pool };

        let (proposer_bucket, fact_bucket) =
            reward::reward_buckets(reward_pool, SEED_PW, SEED_FW, total_correct, total_approved);

        // ----- vault balance: Σ all bonds + stakes, MINUS the dead-end burn -----
        let gross: u64 = proposers.iter().map(|p| p.bond).sum::<u64>()
            + facts
                .iter()
                .map(|f| f.stake + f.votes.iter().map(|v| v.stake).sum::<u64>())
                .sum::<u64>();
        let vault_initial: u64 = gross - burn_pool;
        let stake_vault = self.create_token_account(self.kass_mint, oracle_pda, vault_initial);
        // Back the vault KASS with real mint supply (a Burn elsewhere checks it).
        self.add_mint_supply(self.kass_mint, vault_initial);

        // ----- the Oracle account -------------------------------------------
        let now = self.now();
        let mut oracle = Oracle::zeroed();
        oracle.account_type = AccountType::Oracle.as_u8();
        oracle.creator = self.payer.pubkey().to_bytes().into();
        oracle.kass_mint = self.kass_mint.to_bytes().into();
        oracle.usdc_mint = self.usdc_mint.to_bytes().into();
        oracle.stake_vault = stake_vault.to_bytes().into();
        oracle.deadline = now;
        oracle.phase_ends_at = now;
        oracle.twap_window = TWAP_WINDOW;
        oracle.options_count = (resolved_option as u16 + 1).max(2) as u8;
        oracle.set_phase(phase);
        oracle.proposer_count = proposers.len() as u16;
        oracle.surviving_count = proposers.iter().filter(|p| !p.disqualified).count() as u16;
        // `total_oracle_stake` is the gross accumulator (never decremented by the
        // burn); the vault physically holds `gross − burn_pool`.
        oracle.total_oracle_stake = gross;
        oracle.dispute_bond_total = proposers.iter().map(|p| p.bond).sum();
        // On a dead-end the burned slashed pool is recorded on `bond_pool` (the
        // durable counter), matching the on-chain post-burn terminal state.
        oracle.bond_pool = burn_pool;
        oracle.bump = bump;
        oracle.resolved_option = if resolved {
            resolved_option
        } else {
            CLAIM_OPTION_NONE
        };
        // Reward config snapshot (the real defaults) + the chosen slash fraction.
        oracle.reward_proposer_weight = SEED_PW;
        oracle.reward_fact_weight = SEED_FW;
        oracle.fact_vote_slash_num = slash_num;
        oracle.fact_vote_slash_den = slash_den;
        // The resolution stamps the claims read.
        oracle.total_correct_proposer_stake = total_correct;
        oracle.total_approved_fact_stake = total_approved;
        oracle.reward_pool = reward_pool;
        self.set_program_account(oracle_pda, bytemuck::bytes_of(&oracle).to_vec());

        // ----- the Proposer accounts ----------------------------------------
        let mut seeded_proposers = Vec::with_capacity(proposers.len());
        for p in proposers {
            let authority = Keypair::new();
            self.svm
                .airdrop(&authority.pubkey(), 1_000_000_000)
                .unwrap();
            let dest_kass = self.create_token_account(self.kass_mint, authority.pubkey(), 0);

            let mut acct = Proposer::zeroed();
            acct.account_type = AccountType::Proposer.as_u8();
            acct.oracle = oracle_pda.to_bytes().into();
            acct.authority = authority.pubkey().to_bytes().into();
            acct.bond = p.bond;
            acct.original_option = p.claim_option;
            acct.claim_option = p.claim_option;
            acct.disqualified = p.disqualified as u8;
            acct.slashed = (p.slashed_amount > 0) as u8;
            acct.slashed_amount = p.slashed_amount;
            let account = self.seed_program_account(bytemuck::bytes_of(&acct).to_vec());

            // Mirrors on-chain `claim_proposer`: disqualified forfeits the whole
            // bond (base 0); survivor gets `bond − slashed_amount`; +reward iff
            // Resolved + surviving + correct.
            let base = if p.disqualified {
                0
            } else {
                p.bond.saturating_sub(p.slashed_amount)
            };
            let reward = if resolved && !p.disqualified && p.claim_option == resolved_option {
                reward::proposer_reward(p.bond, proposer_bucket, total_correct)
            } else {
                0
            };
            let expected = base + reward;

            seeded_proposers.push(SeededClaim {
                account,
                authority,
                dest_kass,
                expected,
                kind: 0,
            });
        }

        // ----- the Fact + FactVote accounts ---------------------------------
        let mut seeded_facts = Vec::with_capacity(facts.len());
        for f in facts {
            let approve_stake: u64 = f
                .votes
                .iter()
                .filter(|v| v.kind == VOTE_APPROVE)
                .map(|v| v.stake)
                .sum();
            let duplicate_stake: u64 = f
                .votes
                .iter()
                .filter(|v| v.kind == VOTE_DUPLICATE)
                .map(|v| v.stake)
                .sum();

            // Submitter.
            let submitter_auth = Keypair::new();
            self.svm
                .airdrop(&submitter_auth.pubkey(), 1_000_000_000)
                .unwrap();
            let submitter_dest =
                self.create_token_account(self.kass_mint, submitter_auth.pubkey(), 0);

            let mut fact = Fact::zeroed();
            fact.account_type = AccountType::Fact.as_u8();
            fact.oracle = oracle_pda.to_bytes().into();
            fact.proposer = submitter_auth.pubkey().to_bytes().into();
            fact.stake = f.stake;
            fact.approve_stake = approve_stake;
            fact.duplicate_stake = duplicate_stake;
            fact.agreed = f.agreed as u8;
            fact.duplicate = f.duplicate as u8;
            fact.settled = 1;
            let fact_account = self.seed_program_account(bytemuck::bytes_of(&fact).to_vec());

            // Disposition-based on BOTH terminal phases; reward only on Resolved.
            // A rejected submitter forfeits (0) on a dead-end too (its stake was
            // burned out of the vault at finalize).
            let submitter_expected = if f.agreed {
                f.stake
                    + if resolved {
                        reward::fact_reward(f.stake, fact_bucket, total_approved)
                    } else {
                        0
                    }
            } else if f.duplicate {
                f.stake
            } else {
                0
            };

            // Votes.
            let mut seeded_votes = Vec::with_capacity(f.votes.len());
            for v in &f.votes {
                let voter = Keypair::new();
                self.svm.airdrop(&voter.pubkey(), 1_000_000_000).unwrap();
                let voter_dest = self.create_token_account(self.kass_mint, voter.pubkey(), 0);

                let mut vote = FactVote::zeroed();
                vote.account_type = AccountType::FactVote.as_u8();
                vote.fact = fact_account.to_bytes().into();
                vote.voter = voter.pubkey().to_bytes().into();
                vote.stake = v.stake;
                vote.kind = v.kind;
                let vote_account = self.seed_program_account(bytemuck::bytes_of(&vote).to_vec());

                // Disposition-based on BOTH terminal phases; reward only on
                // Resolved. The rejected-fact approve-voter is slashed on a
                // dead-end too (its slashed fraction was burned at finalize).
                let approve = v.kind == VOTE_APPROVE;
                let expected = if approve && f.agreed {
                    // Approve-voter on an agreed fact earns the fact rate (Resolved
                    // only; 0 on InvalidDeadend since reward_pool == 0).
                    v.stake
                        + if resolved {
                            reward::fact_reward(v.stake, fact_bucket, total_approved)
                        } else {
                            0
                        }
                } else if approve && !f.duplicate {
                    // Approve-voter on a rejected fact is slashed CEIL(stake·num/den)
                    // (mirrors on-chain: ceil keeps the vault from running short
                    // against the floor-aggregate bond_pool credit).
                    let ceil =
                        ((v.stake as u128) * (slash_num as u128)).div_ceil(slash_den as u128);
                    v.stake - ceil as u64
                } else {
                    // Duplicate-voter, or approve-on-duplicate-dominant: full stake.
                    v.stake
                };

                seeded_votes.push(SeededClaim {
                    account: vote_account,
                    authority: voter,
                    dest_kass: voter_dest,
                    expected,
                    kind: v.kind,
                });
            }

            seeded_facts.push(SeededFactClaim {
                submitter: SeededClaim {
                    account: fact_account,
                    authority: submitter_auth,
                    dest_kass: submitter_dest,
                    expected: submitter_expected,
                    kind: 0,
                },
                votes: seeded_votes,
            });
        }

        TerminalSeed {
            oracle: oracle_pda,
            nonce,
            bump,
            stake_vault,
            vault_initial,
            reward_pool,
            total_correct_proposer_stake: total_correct,
            total_approved_fact_stake: total_approved,
            resolved_option,
            proposers: seeded_proposers,
            facts: seeded_facts,
        }
    }
}
