---
name: kassandra-ts-client
description: "Use when integrating with the Kassandra optimistic-oracle Solana program from TypeScript or a dApp - building an instruction (propose, submit a fact, vote, submit an AI claim, open or settle a challenge, finalize, claim, close), decoding an on-chain account (oracle, proposer, fact, market), or deriving a Kassandra PDA. Reach for it before hand-writing account metas, discriminants, or PDA seeds."
---

# Integrating Kassandra from TypeScript

The `@kassandra-market/oracles` package is the client for the Kassandra dispute-oracle program
(`KASSANDRA_PROGRAM_ID` = `KassVxvXUEPr5apSr2MqiGva4VFtJXyYLLDFS3f83nY`). It is ESM, built on
`@solana/web3.js` (v3) + `@solana/kit`. Never hand-roll a Kassandra instruction — every one
has a builder here, and the discriminants/seeds/layouts are the SDK's job.

## Instruction builders

Each is `async` and returns a web3.js `TransactionInstruction`. The builder **derives its own
PDAs** (proposer, stake vault, etc.) — you pass wallets + token accounts, not PDAs.

`propose`, `createOracle`, `submitFact`, `voteFact`, `submitAiClaim`, `openChallenge`,
`settleChallenge`, `finalizeProposals`, `finalizeFacts`, `finalizeOracle`, `finalizeAiClaims`,
`advancePhase`, `claimProposer`, `claimFact`, `claimFactVote`, `closeAiClaim`, `closeMarket`,
`sweepOracle`, `initProtocol`, `setGovernance`, `setConfig`, `resolveDeadend`, `kassPrice`.

Each takes one args object; a `programId?` override is always accepted. Import the `*Args`
type (e.g. `ProposeArgs`) for the exact fields.

## Decoders, PDAs, enums

- Decoders (bytes to typed struct): `decodeOracle`, `decodeProposer`, `decodeFact`,
  `decodeFactVote`, `decodeAiClaim`, `decodeMarket`, `decodeProtocol`. An oracle exposes
  `phase` (a `Phase`), `optionsCount`, `phaseEndsAt`, `kassMint`, `stakeVault`, `resolvedOption`.
- PDAs (`pda` namespace, async, return `{ address, bump }`): `pda.oracle(nonce)`,
  `pda.proposer(oracle, authority)`, `pda.fact(oracle, contentHash)`, `pda.factVote(fact, voter)`,
  `pda.aiClaim(oracle, proposer)`, `pda.market(aiClaim)`, `pda.stakeVault(oracle)`,
  `pda.challengeUsdcVault(market)`, `pda.protocol()`, `pda.mintAuthority()`.
- Enums/constants: `Phase`, `Ix`, `CLAIM_OPTION_NONE`, `VOTE_APPROVE`, `VOTE_DUPLICATE`,
  `KASSANDRA_PROGRAM_ID`, `EXTERNAL_PROGRAM_IDS`.

## Example

```ts
import { propose, decodeOracle, Phase } from "@kassandra-market/oracles";

// Build the propose instruction. `authorityKass` is the proposer's KASS token account
// (the bond source); the proposer PDA + stake vault are derived inside the builder.
async function buildProposeIx(oracle, authority, authorityKass, option, bond) {
  return propose({ oracle, authority, authorityKass, option, bond });
}

// Read an oracle's current phase from chain.
async function readOraclePhase(accountData: Uint8Array) {
  const oracle = decodeOracle(accountData);
  return { phase: oracle.phase, phaseEndsAt: oracle.phaseEndsAt, options: oracle.optionsCount };
  // oracle.phase === Phase.Proposal, Phase.FactVoting, Phase.Challenge, Phase.Resolved, ...
}
```

## Notes

- Proposals only land once the oracle is in `Phase.Proposal` and past its deadline; check
  `decodeOracle(...).phase` first.
- The app wraps builders in a `data/actions/*` layer returning `TransactionInstruction[]` — a
  good pattern to copy, but the builders above are the real primitives.
- Cross-language parity: the Rust client is `kassandra-oracles-sdk` (see the `kassandra-rust-client`
  skill); both mirror the same program, kept in lockstep by a byte-parity test.
