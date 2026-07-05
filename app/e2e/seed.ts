/**
 * Reusable on-chain seeding for the browser E2E — drives oracles into each phase
 * with REAL instructions over surfpool (mirroring the gated surfpool vitest E2Es
 * in `app/test/*.e2e.test.ts`), so each Playwright spec can perform ONE app UI
 * action against an oracle already in the right phase.
 *
 * IMPORTANT: every pubkey handed to an `@kassandra/sdk` builder is passed as a
 * base58 STRING (`.toString()`), never a web3.js `Address` object — under
 * Playwright's loader the app and the SDK resolve separate copies of
 * `@solana/web3.js`, so a foreign `Address` fails the SDK's `instanceof` check.
 */
import { Address, Keypair, Transaction, type TransactionInstruction } from '@solana/web3.js'
import { buildDaoBlob } from '../../sdk/test/surfpool/futarchy-dao.ts'
import {
  TOKEN_PROGRAM_ID,
  VOTE_APPROVE,
  advancePhase,
  createOracle,
  decodeOracle,
  finalizeAiClaims,
  finalizeFacts,
  finalizeProposals,
  pda,
  propose,
  submitAiClaim,
  submitFact,
  voteFact,
} from '@kassandra/sdk'

import { SurfpoolHarness, mintBytes, toHex, tokenAccountBytes } from '../../sdk/test/surfpool/harness.ts'
import { MockAnthropic } from '../../sdk/test/surfpool/mock-anthropic.ts'
import {
  runRunner,
  runnerAvailable,
  writeRunnerConfig,
  type RunOutput,
} from '../../sdk/test/surfpool/run-runner.ts'

export interface SeedCtx {
  harness: SurfpoolHarness
  payer: Keypair
  kassMint: Keypair
  usdcMint: Keypair
}

async function sha256(s: string): Promise<Uint8Array> {
  return new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(s)))
}

/** 64-char hex → the 32-byte array the SDK builders expect. */
function hex32(h: string): Uint8Array {
  const b = new Uint8Array(32)
  for (let i = 0; i < 32; i++) b[i] = parseInt(h.slice(i * 2, i * 2 + 2), 16)
  return b
}

/** The AI-claim hashes the app's "Paste runner output" form accepts. */
export interface RunnerClaim {
  modelId: Uint8Array
  paramsHash: Uint8Array
  ioHash: Uint8Array
  option: number
  /** JSON the app's SubmitAiClaimForm paste-mode consumes verbatim. */
  formPayload: { model_id: string; params_hash: string; io_hash: string; option: number }
}

/**
 * Run the REAL `kassandra-runner` binary — its genuine AnthropicProvider
 * HTTP+parse path — against a LOCAL MOCK Anthropic server (no API key, no
 * network) to produce a real AI-claim payload for `option`. The e2e uses THIS
 * instead of fabricated hashes so the runner is actually exercised end to end.
 *
 * Uses zero facts (the runner accepts an empty fact set), so nothing is fetched
 * over the network; the mock supplies the model's verdict.
 */
export async function runnerClaim(option: number, optionsCount = 2): Promise<RunnerClaim> {
  if (!runnerAvailable()) {
    throw new Error(
      'kassandra-runner binary missing — build it first: `cargo build -p kassandra-runner`',
    )
  }
  const mock = await MockAnthropic.start()
  try {
    mock.setOption(option, 'claude-opus-4-8')
    const configPath = writeRunnerConfig({
      interpretation: 'E2E: resolve the disputed oracle to the AI-selected option.',
      options_count: optionsCount,
      option_labels: Array.from({ length: optionsCount }, (_, i) => ({
        index: i,
        label: `Option ${i}`,
      })),
      facts: [],
    })
    const { code, stdout, stderr } = await runRunner(configPath, mock.baseUrl)
    if (code !== 0) throw new Error(`kassandra-runner exited ${code}: ${stderr}`)
    const out = JSON.parse(stdout) as RunOutput
    return {
      modelId: hex32(out.model_id_hex),
      paramsHash: hex32(out.params_hash_hex),
      ioHash: hex32(out.io_hash_hex),
      option: out.option_index,
      formPayload: {
        model_id: out.model_id_hex,
        params_hash: out.params_hash_hex,
        io_hash: out.io_hash_hex,
        option: out.option_index,
      },
    }
  } finally {
    await mock.stop()
  }
}

/** Boot surfpool, deploy the program, mint KASS/USDC, and init the protocol. */
export async function bootAndInit(
  port: number,
  harnessOpts: Record<string, unknown> = {},
): Promise<SeedCtx> {
  const harness = await SurfpoolHarness.start({ port, ...harnessOpts })
  const payer = await Keypair.generate()
  await harness.airdrop(payer.publicKey.toString(), 1_000_000_000_000)

  const { mintAuthority, initProtocol } = await import('@kassandra/sdk')
  const mintAuth = await mintAuthority()
  const kassMint = await Keypair.generate()
  const usdcMint = await Keypair.generate()
  await harness.setAccount(kassMint.publicKey.toString(), {
    lamports: 1_000_000_000,
    owner: TOKEN_PROGRAM_ID.toString(),
    executable: false,
    data: toHex(mintBytes(mintAuth.address.toBytes(), 10n ** 18n, 9)),
  })
  await harness.setAccount(usdcMint.publicKey.toString(), {
    lamports: 1_000_000_000,
    owner: TOKEN_PROGRAM_ID.toString(),
    executable: false,
    data: toHex(mintBytes(payer.publicKey.toBytes(), 0n, 6)),
  })

  const ctx: SeedCtx = { harness, payer, kassMint, usdcMint }
  await sendIx(
    ctx,
    await initProtocol({
      admin: payer.publicKey.toString(),
      kassMint: kassMint.publicKey.toString(),
      usdcMint: usdcMint.publicKey.toString(),
    }),
  )
  return ctx
}

/** Send one ix signed by the payer (+ extra signers). */
export async function sendIx(
  ctx: SeedCtx,
  ix: TransactionInstruction,
  signers: Keypair[] = [],
): Promise<void> {
  const conn = ctx.harness.connection
  const tx = new Transaction()
  tx.feePayer = ctx.payer.publicKey
  tx.recentBlockhash = (await conn.getLatestBlockhash()).blockhash
  tx.add(ix)
  await tx.sign(ctx.payer, ...signers)
  const sig = await conn.sendRawTransaction(await tx.serialize(), { skipPreflight: false })
  await ctx.harness.confirmSignature(sig)
}

async function fetchAccount(ctx: SeedCtx, address: Address): Promise<Uint8Array> {
  const deadline = Date.now() + 15_000
  while (Date.now() < deadline) {
    const info = await ctx.harness.connection.getAccountInfo(address)
    if (info && info.data.length > 0) return info.data
    await new Promise((r) => setTimeout(r, 150))
  }
  throw new Error(`account ${address} did not appear`)
}

/** Fabricate a KASS token account owned by `owner` (base58) holding `amount`. */
export async function fundKass(ctx: SeedCtx, owner: string, amount: bigint): Promise<string> {
  const ownerBytes = new Address(owner).toBytes()
  const acct = await Keypair.generate()
  await ctx.harness.setAccount(acct.publicKey.toString(), {
    lamports: 5_000_000,
    owner: TOKEN_PROGRAM_ID.toString(),
    executable: false,
    data: toHex(tokenAccountBytes(ctx.kassMint.publicKey.toBytes(), ownerBytes, amount)),
  })
  return acct.publicKey.toString()
}

/** Create an oracle with `optionsCount` options, creator = payer, deadline in the near future. */
export async function createOracleReal(
  ctx: SeedCtx,
  nonce: bigint,
  optionsCount: number,
  question: string,
): Promise<Address> {
  const creatorKass = await fundKass(ctx, ctx.payer.publicKey.toString(), 10n ** 15n)
  const now = await ctx.harness.clockUnixTimestamp()
  await sendIx(
    ctx,
    await createOracle({
      nonce,
      promptHash: await sha256(question),
      optionsCount,
      deadline: now + 1_000n + nonce * 100n,
      twapWindow: 600n,
      creator: ctx.payer.publicKey.toString(),
      creatorKassToken: creatorKass,
      kassMint: ctx.kassMint.publicKey.toString(),
      usdcMint: ctx.usdcMint.publicKey.toString(),
    }),
  )
  return (await pda.oracle(nonce)).address
}

/**
 * How far ahead of the current chain clock to push a kept-open window's
 * `phase_ends_at`. It must clear every clock advance for the rest of seeding AND
 * the ensuing browse/test session — but seeding only elapses a handful of ~1h
 * phase windows (a few hours of chain time in practice), so a WEEK is ample
 * headroom. It must NOT be absurdly large (the old value was 1e9 s ≈ 31 years),
 * because the app renders the remaining time literally ("ends in …"): a 1e9-s
 * window shows as "ends in 11574d", which reads as broken. A week shows a sane
 * "ends in 7d".
 */
const KEEP_OPEN_AHEAD_SECS = 7n * 24n * 3600n

/**
 * Push an oracle's `phase_ends_at` (i64 at byte offset 144) into the near future
 * (see {@link KEEP_OPEN_AHEAD_SECS}) so its CURRENT phase window stays OPEN at
 * test/browse time — regardless of how far later seeding advances the shared
 * surfpool clock. surfpool's time-travel is forward-only, so we cannot rewind
 * the clock into a closed window; instead we move the window's end past every
 * clock position seeding will reach. The phase itself is unchanged, so the
 * phase-gated action (submit fact / vote / submit AI claim) is still legal.
 */
export async function keepWindowOpen(ctx: SeedCtx, oracle: Address): Promise<void> {
  const { KASSANDRA_PROGRAM_ID } = await import('@kassandra/sdk')
  const info = await ctx.harness.connection.getAccountInfo(oracle)
  if (!info) throw new Error(`oracle ${oracle} not found for window patch`)
  const data = Uint8Array.from(info.data as Uint8Array)
  const now = await ctx.harness.clockUnixTimestamp()
  new DataView(data.buffer).setBigInt64(144, now + KEEP_OPEN_AHEAD_SECS, true)
  await ctx.harness.setAccount(oracle.toString(), {
    lamports: Number((info as { lamports?: bigint | number }).lamports ?? 5_000_000),
    owner: KASSANDRA_PROGRAM_ID.toString(),
    executable: false,
    data: toHex(data),
  })
}

/**
 * Fabricate DAO governance: patch the Protocol singleton so `governance_set = 1`
 * and `dao_authority = daoAuthority` (offsets 121 / 128), and create the DAO
 * treasury (`ATA(daoAuthority, kass_mint)`) as an empty KASS token account. The
 * real set_governance is hardened (dao_authority must equal a Squads vault PDA no
 * keypair can sign), so tests fabricate the linkage directly — exactly as the
 * gated `claims.e2e` surfpool test documents.
 */
export async function fabricateGovernance(ctx: SeedCtx, daoAuthority: string): Promise<void> {
  const { KASSANDRA_PROGRAM_ID, associatedTokenAccount } = await import('@kassandra/sdk')
  const p = (await pda.protocol()).address
  const info = await ctx.harness.connection.getAccountInfo(p)
  if (!info) throw new Error('protocol not found')
  const data = Uint8Array.from(info.data as Uint8Array)
  data[121] = 1 // governance_set
  data.set(new Address(daoAuthority).toBytes(), 128) // dao_authority
  await ctx.harness.setAccount(p.toString(), {
    lamports: Number((info as { lamports?: bigint | number }).lamports ?? 5_000_000),
    owner: KASSANDRA_PROGRAM_ID.toString(),
    executable: false,
    data: toHex(data),
  })
  // DAO treasury = ATA(dao_authority, kass_mint), empty.
  const treasury = (await associatedTokenAccount(daoAuthority, ctx.kassMint.publicKey.toString()))
    .address
  await ctx.harness.setAccount(treasury.toString(), {
    lamports: 5_000_000,
    owner: TOKEN_PROGRAM_ID.toString(),
    executable: false,
    data: toHex(
      tokenAccountBytes(ctx.kassMint.publicKey.toBytes(), new Address(daoAuthority).toBytes(), 0n),
    ),
  })
}

/**
 * Back-date an oracle's `phase_ends_at` (offset 144) to ~40 days in the PAST
 * (real time), so the sweep's 30-day grace is elapsed for BOTH the browser gate
 * (SweepControl compares `Date.now()` against `phase_ends_at + grace`) and the
 * program gate (the surfpool clock is well past a real-time-past timestamp).
 */
export async function backdateForSweep(ctx: SeedCtx, oracle: Address): Promise<void> {
  const { KASSANDRA_PROGRAM_ID } = await import('@kassandra/sdk')
  const info = await ctx.harness.connection.getAccountInfo(oracle)
  if (!info) throw new Error('oracle not found for backdate')
  const data = Uint8Array.from(info.data as Uint8Array)
  const past = BigInt(Math.floor(Date.now() / 1000) - 40 * 24 * 3600)
  new DataView(data.buffer).setBigInt64(144, past, true)
  await ctx.harness.setAccount(oracle.toString(), {
    lamports: Number((info as { lamports?: bigint | number }).lamports ?? 5_000_000),
    owner: KASSANDRA_PROGRAM_ID.toString(),
    executable: false,
    data: toHex(data),
  })
}
/** Patch the Protocol singleton bytes in place (for governance fabrication). */
async function patchProtocolBytes(ctx: SeedCtx, mutate: (d: Uint8Array) => void): Promise<void> {
  const { KASSANDRA_PROGRAM_ID } = await import('@kassandra/sdk')
  const p = (await pda.protocol()).address
  const info = await ctx.harness.connection.getAccountInfo(p)
  if (!info) throw new Error('protocol not found')
  const data = Uint8Array.from(info.data as Uint8Array)
  mutate(data)
  await ctx.harness.setAccount(p.toString(), {
    lamports: Number((info as { lamports?: bigint | number }).lamports ?? 5_000_000),
    owner: KASSANDRA_PROGRAM_ID.toString(),
    executable: false,
    data: toHex(data),
  })
}

/**
 * Fabricate a futarchy-owned `Dao` account carrying a spot TWAP and record it as
 * `Protocol.kass_dao` (offset 160) — the account `kass_price` reads and the
 * linkage `set_governance` needs. Returns the DAO address.
 */
export async function fabricateKassDao(ctx: SeedCtx): Promise<string> {
  const { EXTERNAL_PROGRAM_IDS } = await import('@kassandra/sdk')
  const dao = await Keypair.generate()
  await ctx.harness.setAccount(dao.publicKey.toString(), {
    lamports: 5_000_000,
    owner: EXTERNAL_PROGRAM_IDS.futarchyV06.toString(),
    executable: false,
    data: toHex(buildDaoBlob(500_000_000n * 1_000_000n, 1_000_000n, 0n)),
  })
  await patchProtocolBytes(ctx, (d) => d.set(dao.publicKey.toBytes(), 160))
  return dao.publicKey.toString()
}

/** Seed an oracle stuck in InvalidDeadend (phaseRaw byte 161 = 8) for resolve_deadend. */
export async function seedDeadendOracle(ctx: SeedCtx, nonce: bigint): Promise<Address> {
  const { KASSANDRA_PROGRAM_ID } = await import('@kassandra/sdk')
  const o = await createOracleReal(ctx, nonce, 2, 'E2E dead-end')
  await driveToResolvedUncontested(ctx, o, 0)
  const info = await ctx.harness.connection.getAccountInfo(o)
  if (!info) throw new Error('deadend oracle not found')
  const data = Uint8Array.from(info.data as Uint8Array)
  data[161] = 8 // Phase::InvalidDeadend
  await ctx.harness.setAccount(o.toString(), {
    lamports: Number((info as { lamports?: bigint | number }).lamports ?? 5_000_000),
    owner: KASSANDRA_PROGRAM_ID.toString(),
    executable: false,
    data: toHex(data),
  })
  return o
}

/** Resolve an oracle uncontested (all proposers agree) → Resolved(option). */
export async function driveToResolvedUncontested(
  ctx: SeedCtx,
  oracle: Address,
  option: number,
): Promise<void> {
  await openProposals(ctx, oracle)
  const p: string[] = []
  for (let i = 0; i < 3; i++) {
    p.push((await proposeAs(ctx, oracle, await Keypair.generate(), option, 5_000n)).toString())
  }
  await advancePastPhaseEnd(ctx, oracle)
  await sendIx(ctx, await finalizeProposals({ oracle: oracle.toString(), proposers: p }))
}

export async function openProposals(ctx: SeedCtx, oracle: Address): Promise<void> {
  const o = decodeOracle(await fetchAccount(ctx, oracle))
  await ctx.harness.advanceToUnix(o.deadline + 60n)
}

export async function advancePastPhaseEnd(ctx: SeedCtx, oracle: Address): Promise<void> {
  const o = decodeOracle(await fetchAccount(ctx, oracle))
  await ctx.harness.advanceToUnix(o.phaseEndsAt + 120n)
}

/**
 * Propose `option` with `bond` from `authority` (a caller-supplied keypair —
 * pass the funded browser wallet to make it a locked-in proposer). Funds the
 * authority's KASS bond source.
 */
export async function proposeAs(
  ctx: SeedCtx,
  oracle: Address,
  authority: Keypair,
  option: number,
  bond: bigint,
): Promise<Address> {
  await ctx.harness.airdrop(authority.publicKey.toString(), 2_000_000_000)
  const authorityKass = await fundKass(ctx, authority.publicKey.toString(), bond * 10n)
  await sendIx(
    ctx,
    await propose({
      oracle: oracle.toString(),
      authority: authority.publicKey.toString(),
      authorityKass,
      option,
      bond,
    }),
    [authority],
  )
  return (await pda.proposer(oracle.toString(), authority.publicKey.toString())).address
}

/** Drive an oracle into FactProposal via a 2-proposer dispute. Returns the proposer PDAs. */
export async function driveToFactProposal(
  ctx: SeedCtx,
  oracle: Address,
  walletProposer?: Keypair,
): Promise<Address[]> {
  await openProposals(ctx, oracle)
  const proposers: Address[] = []
  const a0 = walletProposer ?? (await Keypair.generate())
  proposers.push(await proposeAs(ctx, oracle, a0, 0, 1_000n))
  const a1 = await Keypair.generate()
  proposers.push(await proposeAs(ctx, oracle, a1, 1, 1_000n))
  await advancePastPhaseEnd(ctx, oracle)
  await sendIx(ctx, await finalizeProposals({ oracle: oracle.toString(), proposers: proposers.map(String) }))
  return proposers
}

/** Submit one fact (from a fresh submitter) into a FactProposal oracle. Returns the Fact PDA. */
export async function submitOneFact(ctx: SeedCtx, oracle: Address): Promise<Address> {
  const contentHash = new Uint8Array(32).fill(0x07)
  const submitter = await Keypair.generate()
  await ctx.harness.airdrop(submitter.publicKey.toString(), 2_000_000_000)
  const submitterKass = await fundKass(ctx, submitter.publicKey.toString(), 1_000_000n)
  await sendIx(
    ctx,
    await submitFact({
      oracle: oracle.toString(),
      submitter: submitter.publicKey.toString(),
      submitterKass,
      contentHash,
      stake: 100n,
      uri: 'ipfs://seeded-fact',
    }),
    [submitter],
  )
  return (await pda.fact(oracle.toString(), contentHash)).address
}

/** Advance FactProposal → FactVoting. */
export async function advanceToFactVoting(ctx: SeedCtx, oracle: Address): Promise<void> {
  await advancePastPhaseEnd(ctx, oracle)
  await sendIx(ctx, await advancePhase({ oracle: oracle.toString() }))
}

/** Approve-vote a fact (fresh voter, clears quorum). */
export async function approveVote(ctx: SeedCtx, oracle: Address, fact: Address): Promise<void> {
  const voter = await Keypair.generate()
  await ctx.harness.airdrop(voter.publicKey.toString(), 2_000_000_000)
  const voterKass = await fundKass(ctx, voter.publicKey.toString(), 10_000n)
  await sendIx(
    ctx,
    await voteFact({
      oracle: oracle.toString(),
      fact: fact.toString(),
      voter: voter.publicKey.toString(),
      voterKass,
      kind: VOTE_APPROVE,
      stake: 2_000n,
    }),
    [voter],
  )
}

/** FactVoting → AiClaim (finalize the fact set). */
export async function advanceToAiClaim(ctx: SeedCtx, oracle: Address, nonce: bigint, fact: Address): Promise<void> {
  await advancePastPhaseEnd(ctx, oracle)
  await sendIx(
    ctx,
    await finalizeFacts({ nonce, kassMint: ctx.kassMint.publicKey.toString(), tail: [fact.toString()] }),
  )
}

/** AiClaim → Challenge (finalize the AI-claim round over the proposer tail). */
export async function advanceToChallenge(ctx: SeedCtx, oracle: Address, proposers: Address[]): Promise<void> {
  await advancePastPhaseEnd(ctx, oracle)
  await sendIx(ctx, await finalizeAiClaims({ oracle: oracle.toString(), proposers: proposers.map(String) }))
}

/**
 * Drive an oracle all the way to the Challenge phase with `wallet` surviving:
 * dispute (wallet = proposer 0) → a fact → FactVoting → approve → AiClaim →
 * the wallet stamps its AI claim (so it is NOT slashed) → finalize_ai_claims →
 * Challenge (open_challenge_count == 0). Returns the wallet's Proposer PDA + the
 * fact + the wallet's AiClaim PDA.
 */
export async function driveToChallengeSurviving(
  ctx: SeedCtx,
  oracle: Address,
  nonce: bigint,
  wallet: Keypair,
): Promise<{ proposers: Address[]; fact: Address; aiClaim: Address }> {
  const proposers = await driveToFactProposal(ctx, oracle, wallet)
  const fact = await submitOneFact(ctx, oracle)
  await advanceToFactVoting(ctx, oracle)
  await approveVote(ctx, oracle, fact)
  await advanceToAiClaim(ctx, oracle, nonce, fact)
  await submitAiClaimAs(ctx, oracle, proposers[0], wallet, 0)
  await advanceToChallenge(ctx, oracle, proposers)
  const aiClaim = (await pda.aiClaim(oracle.toString(), proposers[0].toString())).address
  return { proposers, fact, aiClaim }
}

/** Finalize a Challenge-phase oracle (no open challenges) → terminal (Resolved). */
export async function finalizeToTerminal(
  ctx: SeedCtx,
  oracle: Address,
  nonce: bigint,
  proposers: Address[],
): Promise<void> {
  const { finalizeOracle } = await import('@kassandra/sdk')
  await advancePastPhaseEnd(ctx, oracle)
  await sendIx(
    ctx,
    await finalizeOracle({
      nonce,
      kassMint: ctx.kassMint.publicKey.toString(),
      proposers: proposers.map(String),
    }),
  )
}

/** Submit a fact as a SPECIFIC keypair (e.g. the browser wallet). Returns the Fact PDA. */
export async function submitFactAs(
  ctx: SeedCtx,
  oracle: Address,
  submitter: Keypair,
  stake: bigint,
): Promise<Address> {
  const contentHash = new Uint8Array(32).fill(0x5a)
  await ctx.harness.airdrop(submitter.publicKey.toString(), 2_000_000_000)
  const submitterKass = await fundKass(ctx, submitter.publicKey.toString(), stake * 10n)
  await sendIx(
    ctx,
    await submitFact({
      oracle: oracle.toString(),
      submitter: submitter.publicKey.toString(),
      submitterKass,
      contentHash,
      stake,
      uri: 'ipfs://wallet-fact',
    }),
    [submitter],
  )
  return (await pda.fact(oracle.toString(), contentHash)).address
}

/** Approve-vote a fact as a SPECIFIC keypair (e.g. the browser wallet). */
export async function voteFactAs(
  ctx: SeedCtx,
  oracle: Address,
  fact: Address,
  voter: Keypair,
  stake: bigint,
): Promise<void> {
  await ctx.harness.airdrop(voter.publicKey.toString(), 2_000_000_000)
  const voterKass = await fundKass(ctx, voter.publicKey.toString(), stake * 10n)
  await sendIx(
    ctx,
    await voteFact({
      oracle: oracle.toString(),
      fact: fact.toString(),
      voter: voter.publicKey.toString(),
      voterKass,
      kind: VOTE_APPROVE,
      stake,
    }),
    [voter],
  )
}

/**
 * Drive an oracle all the way to Resolved with `wallet` in EVERY claimable role:
 * winning proposer (option 0), agreed-fact submitter, approve-voter, and AI
 * claimant. The second proposer (option 1) submits no AI claim, so it is slashed
 * and the wallet's option resolves. Returns the wallet's claimable child PDAs.
 */
export async function driveToResolvedFull(
  ctx: SeedCtx,
  oracle: Address,
  nonce: bigint,
  wallet: Keypair,
): Promise<{ proposer: Address; fact: Address; factVote: Address; aiClaim: Address }> {
  await openProposals(ctx, oracle)
  const walletProposer = await proposeAs(ctx, oracle, wallet, 0, 5_000n)
  const other = await proposeAs(ctx, oracle, await Keypair.generate(), 1, 1_000n)
  await advancePastPhaseEnd(ctx, oracle)
  await sendIx(ctx, await finalizeProposals({ oracle: oracle.toString(), proposers: [walletProposer.toString(), other.toString()] }))

  const fact = await submitFactAs(ctx, oracle, wallet, 2_000n)
  await advanceToFactVoting(ctx, oracle)
  await voteFactAs(ctx, oracle, fact, wallet, 8_000n) // clears quorum vs the 6000 bond weight
  await advanceToAiClaim(ctx, oracle, nonce, fact)
  await submitAiClaimAs(ctx, oracle, walletProposer, wallet, 0)
  await advanceToChallenge(ctx, oracle, [walletProposer, other])
  await finalizeToTerminal(ctx, oracle, nonce, [walletProposer, other])

  const factVote = (await pda.factVote(fact.toString(), wallet.publicKey.toString())).address
  const aiClaim = (await pda.aiClaim(oracle.toString(), walletProposer.toString())).address
  return { proposer: walletProposer, fact, factVote, aiClaim }
}

/**
 * Submit an AI claim as `authority` for its `proposer`, using hashes produced by
 * the REAL runner (mock Anthropic — see {@link runnerClaim}), not fabricated ones.
 * Returns the runner claim so callers can reuse its payload (e.g. the browser
 * paste-mode test).
 */
export async function submitAiClaimAs(
  ctx: SeedCtx,
  oracle: Address,
  proposer: Address,
  authority: Keypair,
  option: number,
): Promise<RunnerClaim> {
  const claim = await runnerClaim(option)
  await sendIx(
    ctx,
    await submitAiClaim({
      oracle: oracle.toString(),
      proposer: proposer.toString(),
      authority: authority.publicKey.toString(),
      modelId: claim.modelId,
      paramsHash: claim.paramsHash,
      ioHash: claim.ioHash,
      option: claim.option,
    }),
    [authority],
  )
  return claim
}
