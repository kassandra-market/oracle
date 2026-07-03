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

export interface SeedCtx {
  harness: SurfpoolHarness
  payer: Keypair
  kassMint: Keypair
  usdcMint: Keypair
}

async function sha256(s: string): Promise<Uint8Array> {
  return new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(s)))
}

/** Boot surfpool, deploy the program, mint KASS/USDC, and init the protocol. */
export async function bootAndInit(port: number): Promise<SeedCtx> {
  const harness = await SurfpoolHarness.start({ port })
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
 * Push an oracle's `phase_ends_at` (i64 at byte offset 144) to the far future so
 * its CURRENT phase window stays OPEN at test time — regardless of how far later
 * seeding advances the shared surfpool clock. surfpool's time-travel is
 * forward-only, so we cannot rewind the clock into a closed window; instead we
 * move the window's end past every future clock position. The phase itself is
 * unchanged, so the phase-gated action (submit fact / vote / submit AI claim) is
 * still legal.
 */
export async function keepWindowOpen(ctx: SeedCtx, oracle: Address): Promise<void> {
  const { KASSANDRA_PROGRAM_ID } = await import('@kassandra/sdk')
  const info = await ctx.harness.connection.getAccountInfo(oracle)
  if (!info) throw new Error(`oracle ${oracle} not found for window patch`)
  const data = Uint8Array.from(info.data as Uint8Array)
  const now = await ctx.harness.clockUnixTimestamp()
  new DataView(data.buffer).setBigInt64(144, now + 1_000_000_000n, true)
  await ctx.harness.setAccount(oracle.toString(), {
    lamports: Number((info as { lamports?: bigint | number }).lamports ?? 5_000_000),
    owner: KASSANDRA_PROGRAM_ID.toString(),
    executable: false,
    data: toHex(data),
  })
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

/** Submit a (fabricated-metadata) AI claim as `authority` for its `proposer`. */
export async function submitAiClaimAs(
  ctx: SeedCtx,
  oracle: Address,
  proposer: Address,
  authority: Keypair,
  option: number,
): Promise<void> {
  await sendIx(
    ctx,
    await submitAiClaim({
      oracle: oracle.toString(),
      proposer: proposer.toString(),
      authority: authority.publicKey.toString(),
      modelId: new Uint8Array(32).fill(0x11),
      paramsHash: new Uint8Array(32).fill(0x22),
      ioHash: new Uint8Array(32).fill(0x33),
      option,
    }),
    [authority],
  )
}
