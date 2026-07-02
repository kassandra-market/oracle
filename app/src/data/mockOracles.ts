/**
 * Offline mock fixtures for the oracle browse views — decoded-shaped
 * {@link OracleSummary} / {@link OracleDetail} objects so the list + detail
 * pages are visually reviewable with NO chain / RPC (headless render, design
 * review). This does NOT pollute the real data path: the pages call these ONLY
 * when {@link isMockMode} is true (`VITE_MOCK=1` build-time, or a `?mock` query
 * param at runtime); otherwise they go through `fetchOracles`/`fetchOracleDetail`
 * over the live {@link Connection}.
 */
import type { Address } from '@solana/web3.js'
import { Phase } from '@kassandra/sdk'
import type { AiClaim, Fact, Market, Oracle, Proposer } from '@kassandra/sdk'
import { OracleNotFoundError, type OracleDetail, type OracleSummary } from './oracles'

/**
 * Enable mock mode. `VITE_MOCK=1` at build time forces it; otherwise a `?mock`
 * query param flips a live build into fixtures for offline preview.
 */
export function isMockMode(): boolean {
  if (import.meta.env.VITE_MOCK === '1') return true
  if (typeof window !== 'undefined') {
    return new URLSearchParams(window.location.search).has('mock')
  }
  return false
}

// A readable stand-in for an Address. Mock display code only ever stringifies
// these (they are never fed to `new Address(...)` — that path is the live RPC),
// so a cast is safe and keeps the fixtures legible.
const A = (s: string): Address => s as unknown as Address

// Deterministic 32-byte "hash" from a seed, so previews render stable hex.
function hashBytes(seed: number): Uint8Array {
  return Uint8Array.from({ length: 32 }, (_, i) => (seed * 31 + i * 7) & 0xff)
}

const NOW = Math.floor(Date.now() / 1000)
const secs = (deltaDays: number): bigint => BigInt(NOW + Math.round(deltaDays * 86400))

const KASS_MINT = A('KassM1nt1111111111111111111111111111111111')
const USDC_MINT = A('UsdcM1nt1111111111111111111111111111111111')

/** Full-shape Oracle with sensible defaults; callers override the interesting fields. */
function makeOracle(over: Partial<Oracle>): Oracle {
  const base: Oracle = {
    accountType: 1 as Oracle['accountType'],
    creator: A('Creator11111111111111111111111111111111111'),
    kassMint: KASS_MINT,
    usdcMint: USDC_MINT,
    stakeVault: A('StakeVau1t111111111111111111111111111111111'),
    deadline: secs(-1),
    phaseEndsAt: secs(1),
    twapWindow: 3600n,
    optionsCount: 2,
    phaseRaw: Phase.Proposal,
    phase: Phase.Proposal,
    proposerCount: 0,
    survivingCount: 0,
    factCount: 0,
    totalOracleStake: 0n,
    bondPool: 0n,
    disputeBondTotal: 0n,
    settledCount: 0,
    aiFinalizedCount: 0,
    bump: 254,
    resolvedOption: 0xff,
    openChallengeCount: 0,
    promptHash: hashBytes(1),
    thresholdNum: 2n,
    thresholdDen: 3n,
    marketThresholdNum: 1n,
    marketThresholdDen: 10n,
    flipSlashNum: 1n,
    flipSlashDen: 2n,
    phaseWindow: 3600n,
    proposalWindow: 3600n,
    factVoteSlashNum: 1n,
    factVoteSlashDen: 2n,
    rewardProposerWeight: 1n,
    rewardFactWeight: 1n,
    challengeFailUsdcFeeNum: 1n,
    challengeFailUsdcFeeDen: 100n,
    challengeSuccessKassFeeNum: 1n,
    challengeSuccessKassFeeDen: 100n,
    totalCorrectProposerStake: 0n,
    totalApprovedFactStake: 0n,
    rewardPool: 0n,
    rewardEmission: 0n,
  }
  return { ...base, ...over }
}

// --- the fixture oracles (one per interesting phase) -------------------------

const ORACLES: OracleSummary[] = [
  {
    pubkey: 'OracLeChaLLenged11111111111111111111111111111',
    oracle: makeOracle({
      deadline: secs(-2),
      phaseEndsAt: secs(0.5),
      phaseRaw: Phase.Challenge,
      phase: Phase.Challenge,
      optionsCount: 3,
      proposerCount: 4,
      survivingCount: 3,
      factCount: 2,
      settledCount: 2,
      bondPool: 12_500_000_000n,
      disputeBondTotal: 40_000_000_000n,
      openChallengeCount: 1,
      promptHash: hashBytes(11),
    }),
  },
  {
    pubkey: 'OracLeProposaL111111111111111111111111111111',
    oracle: makeOracle({
      deadline: secs(3),
      phaseEndsAt: secs(3),
      phaseRaw: Phase.Proposal,
      phase: Phase.Proposal,
      optionsCount: 2,
      proposerCount: 1,
      survivingCount: 1,
      factCount: 0,
      promptHash: hashBytes(22),
    }),
  },
  {
    pubkey: 'OracLeAiCLaim11111111111111111111111111111111',
    oracle: makeOracle({
      deadline: secs(-1),
      phaseEndsAt: secs(0.25),
      phaseRaw: Phase.AiClaim,
      phase: Phase.AiClaim,
      optionsCount: 2,
      proposerCount: 2,
      survivingCount: 2,
      factCount: 1,
      settledCount: 1,
      bondPool: 2_000_000_000n,
      disputeBondTotal: 20_000_000_000n,
      promptHash: hashBytes(33),
    }),
  },
  {
    pubkey: 'OracLeReso1ved1111111111111111111111111111111',
    oracle: makeOracle({
      deadline: secs(-6),
      phaseEndsAt: secs(-3),
      phaseRaw: Phase.Resolved,
      phase: Phase.Resolved,
      optionsCount: 2,
      proposerCount: 2,
      survivingCount: 2,
      factCount: 1,
      settledCount: 1,
      resolvedOption: 1,
      bondPool: 5_000_000_000n,
      totalCorrectProposerStake: 30_000_000_000n,
      totalApprovedFactStake: 15_000_000_000n,
      rewardPool: 6_000_000_000n,
      promptHash: hashBytes(44),
    }),
  },
  {
    pubkey: 'OracLeDeadend11111111111111111111111111111111',
    oracle: makeOracle({
      deadline: secs(-9),
      phaseEndsAt: secs(-8),
      phaseRaw: Phase.InvalidDeadend,
      phase: Phase.InvalidDeadend,
      optionsCount: 4,
      proposerCount: 3,
      survivingCount: 0,
      factCount: 2,
      resolvedOption: 0xff,
      bondPool: 40_000_000_000n,
      promptHash: hashBytes(55),
    }),
  },
]

// --- children for the rich detail (the challenged oracle) --------------------

function childrenFor(pubkey: string): Pick<OracleDetail, 'facts' | 'proposers' | 'aiClaims' | 'market'> {
  const oracle = A(pubkey)
  const facts: OracleDetail['facts'] = [
    {
      pubkey: 'FactAgreed1111111111111111111111111111111111',
      fact: {
        accountType: 3 as Fact['accountType'],
        oracle,
        proposer: A('Proposer0ne11111111111111111111111111111111'),
        contentHash: hashBytes(101),
        stake: 5_000_000_000n,
        approveStake: 18_000_000_000n,
        duplicateStake: 1_000_000_000n,
        uriLen: 21,
        agreed: true,
        duplicate: false,
        settled: true,
        bump: 255,
        uri: 'ipfs://seeded-fact-01',
        uriRaw: new Uint8Array(200),
      },
    },
    {
      pubkey: 'FactRejected111111111111111111111111111111111',
      fact: {
        accountType: 3 as Fact['accountType'],
        oracle,
        proposer: A('ProposerTwo11111111111111111111111111111111'),
        contentHash: hashBytes(102),
        stake: 3_000_000_000n,
        approveStake: 2_000_000_000n,
        duplicateStake: 9_000_000_000n,
        uriLen: 0,
        agreed: false,
        duplicate: true,
        settled: true,
        bump: 255,
        uri: '',
        uriRaw: new Uint8Array(200),
      },
    },
  ]
  const proposers: OracleDetail['proposers'] = [
    {
      pubkey: 'Proposer0ne11111111111111111111111111111111',
      proposer: {
        accountType: 2 as Proposer['accountType'],
        oracle,
        authority: A('AuthAdaL0ve1ace1111111111111111111111111111'),
        bond: 20_000_000_000n,
        originalOption: 1,
        claimOption: 1,
        disqualified: false,
        slashed: false,
        flipped: false,
        bump: 255,
        aiFinalized: true,
        slashedAmount: 0n,
      },
    },
    {
      pubkey: 'ProposerTwo11111111111111111111111111111111',
      proposer: {
        accountType: 2 as Proposer['accountType'],
        oracle,
        authority: A('AuthGraceHopper11111111111111111111111111111'),
        bond: 20_000_000_000n,
        originalOption: 0,
        claimOption: 2,
        disqualified: false,
        slashed: true,
        flipped: true,
        bump: 255,
        aiFinalized: true,
        slashedAmount: 10_000_000_000n,
      },
    },
  ]
  const aiClaims: OracleDetail['aiClaims'] = [
    {
      pubkey: 'AiC1aim1111111111111111111111111111111111111',
      aiClaim: {
        accountType: 5 as AiClaim['accountType'],
        oracle,
        proposer: A('Proposer0ne11111111111111111111111111111111'),
        modelId: hashBytes(201),
        paramsHash: hashBytes(202),
        ioHash: hashBytes(203),
        option: 1,
        challenged: true,
        bump: 255,
        authority: A('AuthAdaL0ve1ace1111111111111111111111111111'),
      },
    },
  ]
  const market: OracleDetail['market'] = {
    pubkey: 'Market11111111111111111111111111111111111111',
    market: {
      accountType: 6 as Market['accountType'],
      oracle,
      aiClaim: A('AiC1aim1111111111111111111111111111111111111'),
      proposer: A('Proposer0ne11111111111111111111111111111111'),
      challenger: A('Cha11enger5atoshi11111111111111111111111111'),
      question: A('Question1111111111111111111111111111111111111'),
      kassVault: A('KassVau1t111111111111111111111111111111111'),
      usdcVault: A('UsdcVau1t111111111111111111111111111111111'),
      passAmm: A('PassAmm11111111111111111111111111111111111'),
      failAmm: A('Fai1Amm11111111111111111111111111111111111'),
      oraclePassKass: A('OraclePassKass11111111111111111111111111111'),
      oracleFailKass: A('OracleFai1Kass11111111111111111111111111111'),
      challengerUsdcVault: A('Cha11engerUsdcVau1t1111111111111111111111'),
      twapEnd: secs(0.5),
      challengerUsdc: 500_000_000n,
      settled: false,
      bump: 255,
    },
  }
  return { facts, proposers, aiClaims, market }
}

/** All mock oracle summaries, sorted by deadline desc (mirrors the live path). */
export function mockOracles(): OracleSummary[] {
  return [...ORACLES].sort((a, b) =>
    b.oracle.deadline > a.oracle.deadline ? 1 : b.oracle.deadline < a.oracle.deadline ? -1 : 0,
  )
}

/**
 * Mock detail for a pubkey. The challenged oracle gets a full child set (facts,
 * proposers, AI claims, market); the others are empty (exercising the empty
 * section states). An unknown pubkey rejects with {@link OracleNotFoundError}
 * so the not-found state is reviewable via `?mock` on a bogus id.
 */
export function mockOracleDetail(pubkey: string): Promise<OracleDetail> {
  const summary = ORACLES.find((o) => o.pubkey === pubkey)
  if (!summary) return Promise.reject(new OracleNotFoundError(pubkey))
  const rich = pubkey === 'OracLeChaLLenged11111111111111111111111111111'
  const kids = rich
    ? childrenFor(pubkey)
    : { facts: [], proposers: [], aiClaims: [], market: undefined }
  return Promise.resolve({ pubkey, oracle: summary.oracle, ...kids })
}
