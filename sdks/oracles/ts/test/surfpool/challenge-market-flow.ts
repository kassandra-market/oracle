/**
 * T4 surfpool CHALLENGE-MARKET E2E — high-level flows (question/market
 * composition, front-door-to-Challenge, open/settle, and the v0.4 AMM pool
 * build/swap/crank). Extracted from `challenge-market-e2e.test.ts`; bodies are
 * verbatim.
 */
import { Address, Keypair, TransactionInstruction } from "@solana/web3.js";

import { decodeMarket } from "../../src/accounts/index.js";
import { ammV04 } from "../../src/index.js";
import { VOTE_APPROVE, TOKEN_PROGRAM_ID } from "../../src/constants.js";
import {
  advancePhase,
  finalizeAiClaims,
  finalizeFacts,
  finalizeProposals,
  openChallenge,
  settleChallenge,
  submitAiClaim,
  submitFact,
  voteFact,
} from "../../src/instructions/index.js";
import * as pda from "../../src/pda.js";

import {
  enc,
  VLTX,
  AMM_ID,
  BOND,
  MAX_PRICE,
  SYSTEM_PROGRAM_ID,
  ATA_PROGRAM_ID,
  INITIALIZE_QUESTION,
  INITIALIZE_CONDITIONAL_VAULT,
  sendIx,
  fetchAccount,
  fundKass,
  ata,
  composeVault,
  fabricateTokenAccountMint,
  createOracleReal,
  openProposals,
  advancePastPhaseEnd,
  proposeRealWithAuthority,
  setTokenAccountAt,
  advanceSlots,
  type Fixture,
  type Challenged,
  type MarketComposition,
  type VaultAccounts,
  type Payouts,
} from "./challenge-market-harness.js";

/** Real `initialize_question` CPI: a binary question whose resolver == `oracle`. */
export async function composeQuestion(
  f: Fixture,
  resolver: Address,
  questionId: Uint8Array,
  numOutcomes: number,
): Promise<{ question: Address }> {
  const [question] = await Address.findProgramAddress(
    [enc.encode("question"), questionId, resolver.toBytes(), Uint8Array.from([numOutcomes])],
    VLTX,
  );
  const [eventAuthority] = await Address.findProgramAddress([enc.encode("__event_authority")], VLTX);

  const data = new Uint8Array(73);
  data.set(INITIALIZE_QUESTION, 0);
  data.set(questionId, 8);
  data.set(resolver.toBytes(), 40);
  data[72] = numOutcomes;

  await sendIx(
    f,
    new TransactionInstruction({
      programId: VLTX,
      keys: [
        { pubkey: question, isSigner: false, isWritable: true },
        { pubkey: f.payer.publicKey, isSigner: true, isWritable: true },
        { pubkey: SYSTEM_PROGRAM_ID, isSigner: false, isWritable: false },
        { pubkey: eventAuthority, isSigner: false, isWritable: false },
        { pubkey: VLTX, isSigner: false, isWritable: false },
      ],
      data,
    }),
    [],
    400_000,
  );
  return { question };
}

/**
 * Drive the REAL dispute core (clock advanced via `surfnet_timeTravel`) to
 * `Phase::Challenge`. The returned proposer is the option-0 proposer who claims
 * option 0 (no flip) → surviving, `slashed_amount == 0`: a clean bond to
 * challenge. Mirrors `challenge_e2e.rs::front_door_to_challenge`.
 */
export async function frontDoorToChallenge(f: Fixture, nonce: bigint): Promise<Challenged> {
  const oracle = (await pda.oracle(nonce)).address;
  const aiOption = 0;

  await createOracleReal(f, nonce, 2);
  await openProposals(f, oracle);

  const authorities: Keypair[] = [];
  const proposerPdas: Address[] = [];
  for (const option of [0, 1]) {
    const { authority, proposer } = await proposeRealWithAuthority(f, oracle, option, BOND);
    authorities.push(authority);
    proposerPdas.push(proposer);
  }

  await advancePastPhaseEnd(f, oracle);
  await sendIx(f, await finalizeProposals({ oracle, proposers: proposerPdas }));

  const contentHash = new Uint8Array(32).fill(0x07);
  const submitter = await Keypair.generate();
  await f.harness.airdrop(submitter.publicKey.toString(), 2_000_000_000);
  const submitterKass = await fundKass(f, submitter.publicKey, 1_000_000n);
  await sendIx(
    f,
    await submitFact({ oracle, submitter: submitter.publicKey, submitterKass, contentHash, stake: 100n, uri: "ipfs://fact" }),
    [submitter],
  );
  const fact = (await pda.fact(oracle, contentHash)).address;

  await advancePastPhaseEnd(f, oracle);
  await sendIx(f, await advancePhase({ oracle }));

  const voter = await Keypair.generate();
  await f.harness.airdrop(voter.publicKey.toString(), 2_000_000_000);
  const voterKass = await fundKass(f, voter.publicKey, 10n * BOND);
  await sendIx(
    f,
    await voteFact({ oracle, fact, voter: voter.publicKey, voterKass, kind: VOTE_APPROVE, stake: 2n * BOND }),
    [voter],
  );

  await advancePastPhaseEnd(f, oracle);
  await sendIx(f, await finalizeFacts({ nonce, kassMint: f.kassMint.publicKey, tail: [fact] }));

  for (let i = 0; i < proposerPdas.length; i++) {
    await sendIx(
      f,
      await submitAiClaim({
        oracle,
        proposer: proposerPdas[i],
        authority: authorities[i].publicKey,
        modelId: new Uint8Array(32).fill(0xa1),
        paramsHash: new Uint8Array(32).fill(0xb2),
        ioHash: new Uint8Array(32).fill(0xc3),
        option: aiOption,
      }),
      [authorities[i]],
    );
  }

  await advancePastPhaseEnd(f, oracle);
  await sendIx(f, await finalizeAiClaims({ oracle, proposers: proposerPdas }));

  const proposer = proposerPdas[0];
  const aiClaim = (await pda.aiClaim(oracle, proposer)).address;
  return {
    oracle,
    proposer,
    proposerAuthority: authorities[0].publicKey,
    aiClaim,
    proposerPdas,
    authorities,
  };
}

/** Compose the binary question + KASS/USDC conditional vaults (resolver == oracle)
 * + the oracle-PDA-owned pass/fail conditional-KASS holders. */
export async function composeMarket(f: Fixture, oracle: Address): Promise<MarketComposition> {
  const questionId = new Uint8Array(32).fill(0x07);
  const { question } = await composeQuestion(f, oracle, questionId, 2);
  const kass = await composeVault(f, question, f.kassMint.publicKey);
  const usdc = await composeVault(f, question, f.usdcMint.publicKey);
  const oraclePassKass = await fabricateTokenAccountMint(f, kass.passMint, oracle, 0n);
  const oracleFailKass = await fabricateTokenAccountMint(f, kass.failMint, oracle, 0n);
  return { question, kass, usdc, oraclePassKass, oracleFailKass };
}

/** Send the Kassandra `open_challenge` (program-signed `split_tokens` CPI →
 * forked vault). Returns the fresh challenger + the Market PDA. */
export async function openChallengeReal(
  f: Fixture,
  nonce: bigint,
  c: Challenged,
  m: MarketComposition,
  passAmm: Address,
  failAmm: Address,
): Promise<{ challenger: Keypair; market: Address }> {
  const challenger = await Keypair.generate();
  await f.harness.airdrop(challenger.publicKey.toString(), 2_000_000_000);
  const challengerUsdcSrc = await fabricateTokenAccountMint(f, f.usdcMint.publicKey, challenger.publicKey, 5_000_000n);
  const cvEventAuthority = (await Address.findProgramAddress([enc.encode("__event_authority")], VLTX))[0];

  await sendIx(
    f,
    await openChallenge({
      nonce,
      proposer: c.proposer,
      challenger: challenger.publicKey,
      question: m.question,
      kassVault: m.kass.vault,
      usdcVault: m.usdc.vault,
      passAmm,
      failAmm,
      kassVaultUnderlying: m.kass.underlying,
      passKassMint: m.kass.passMint,
      failKassMint: m.kass.failMint,
      oraclePassKass: m.oraclePassKass,
      oracleFailKass: m.oracleFailKass,
      cvEventAuthority,
      kassDao: f.kassDao,
      usdcMint: f.usdcMint.publicKey,
      challengerUsdcSrc,
    }),
    [challenger],
    1_400_000,
  );
  const market = (await pda.market(c.aiClaim)).address;
  return { challenger, market };
}

/** Fabricate the (empty) settle payout destinations, advance past `twap_end`, and
 * send the Kassandra `settle_challenge`. Returns the payout accounts to assert on. */
export async function settleChallengeReal(
  f: Fixture,
  nonce: bigint,
  c: Challenged,
  m: MarketComposition,
  market: Address,
  challenger: Keypair,
  passAmm: Address,
  failAmm: Address,
): Promise<Payouts> {
  const proposerUsdc = await fabricateTokenAccountMint(f, f.usdcMint.publicKey, c.proposerAuthority, 0n);
  const challengerUsdcDest = await fabricateTokenAccountMint(f, f.usdcMint.publicKey, challenger.publicKey, 0n);
  const challengerKass = await fabricateTokenAccountMint(f, f.kassMint.publicKey, challenger.publicKey, 0n);
  const escrowVault = (await pda.challengeUsdcVault(market)).address;
  const cvEventAuthority = (await Address.findProgramAddress([enc.encode("__event_authority")], VLTX))[0];

  // Gate: settle is allowed only after market.twap_end (now + oracle.twap_window).
  const twapEnd = decodeMarket(await fetchAccount(f, market)).twapEnd;
  await f.harness.advanceToUnix(twapEnd + 120n);

  await sendIx(
    f,
    await settleChallenge({
      nonce,
      aiClaim: c.aiClaim,
      proposer: c.proposer,
      question: m.question,
      passAmm,
      failAmm,
      cvEventAuthority,
      kassVault: m.kass.vault,
      kassVaultUnderlying: m.kass.underlying,
      passKassMint: m.kass.passMint,
      failKassMint: m.kass.failMint,
      oraclePassKass: m.oraclePassKass,
      oracleFailKass: m.oracleFailKass,
      proposerUsdc,
      challengerUsdcDest,
      challengerKass,
    }),
    [],
    1_400_000,
  );
  return { escrowVault, proposerUsdc, challengerUsdcDest, challengerKass };
}

/** `create_amm` + `add_liquidity` for one (base, quote) conditional pair. Funds
 * the payer's base/quote ATAs (4× reserve) so a later swap has headroom. Returns
 * the `Amm` PDA. Mirrors `challenge_e2e.rs::build_pool`. */
export async function buildPool(
  f: Fixture,
  baseMint: Address,
  quoteMint: Address,
  baseReserve: bigint,
  quoteReserve: bigint,
): Promise<Address> {
  const ammAddr = (await ammV04.pda.amm(baseMint, quoteMint)).address;
  const lp = (await ammV04.pda.lpMint(ammAddr)).address;
  const userBase = await ammV04.pda.ata(f.payer.publicKey, baseMint);
  const userQuote = await ammV04.pda.ata(f.payer.publicKey, quoteMint);
  await setTokenAccountAt(f, userBase, baseMint, f.payer.publicKey, baseReserve * 4n);
  await setTokenAccountAt(f, userQuote, quoteMint, f.payer.publicKey, quoteReserve * 4n);

  const initialObs = (quoteReserve * 1_000_000_000_000n) / baseReserve;
  await sendIx(
    f,
    await ammV04.createAmm({
      payer: f.payer.publicKey,
      baseMint,
      quoteMint,
      twapInitialObservation: initialObs,
      twapMaxObservationChangePerUpdate: MAX_PRICE,
      twapStartDelaySlots: 0n,
    }),
    [],
    1_400_000,
  );

  const userLp = await ammV04.pda.ata(f.payer.publicKey, lp);
  await setTokenAccountAt(f, userLp, lp, f.payer.publicKey, 0n);
  await sendIx(
    f,
    await ammV04.addLiquidity({
      payer: f.payer.publicKey,
      baseMint,
      quoteMint,
      quoteAmount: quoteReserve,
      maxBaseAmount: baseReserve,
      minLpTokens: 0n,
    }),
    [],
    1_400_000,
  );
  return ammAddr;
}

/** A genuine BUY (quote in, base out) that pushes the pool's price UP. Warps 5
 * slots first (mirror `swap_buy`'s `warp_slots(0, 5)`). */
export async function swapBuy(f: Fixture, baseMint: Address, quoteMint: Address, amountIn: bigint): Promise<void> {
  // A generous forward jump (surfnet_timeTravel rejects tiny increments that
  // land at/under its internal slot with "Internal error"); the cranks below
  // weight the post-swap price into the slot-weighted TWAP regardless.
  await advanceSlots(f, 200);
  await sendIx(
    f,
    await ammV04.swap({
      payer: f.payer.publicKey,
      baseMint,
      quoteMint,
      swapType: ammV04.SwapType.Buy,
      inputAmount: amountIn,
      minOutputAmount: 0n,
    }),
    [],
    1_400_000,
  );
}

/** Advance ≥ ONE_MINUTE_IN_SLOTS (150) slots, then `crank_that_twap` once
 * (mirror `crank_pool`'s `warp_slots(0, 300)`). */
export async function crankPool(f: Fixture, amm: Address): Promise<void> {
  await advanceSlots(f, 300);
  await sendIx(f, await ammV04.crankThatTwap({ amm }), [], 400_000);
}
