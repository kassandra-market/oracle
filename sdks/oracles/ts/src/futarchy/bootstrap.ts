/**
 * Off-chain governance bootstrap (Task G2).
 *
 * Composes the instruction sequence that stands up a real futarchy KASS DAO and
 * hands Kassandra's governance to it:
 *   1. futarchy `initialize_dao` — creates the `Dao` AND (via an internal CPI)
 *      the Squads v4 multisig with `create_key == Dao` + vault index 0
 *      (see ./NOTES.md — CONFIRMED). No separate multisig-create step is needed.
 *   2. derive the multisig + vault PDAs from the Dao PDA.
 *   3. Kassandra G1-hardened `set_governance` with `dao_authority = vault`,
 *      `kass_dao = Dao` (the on-chain handoff re-derives + validates this exact
 *      linkage).
 *
 * Returns the created/derived addresses + the composed `TransactionInstruction`s.
 * The LIVE submission against surfpool is G3; here it is compose + typecheck +
 * offline byte/sequence assertions.
 */
import type { Address, TransactionInstruction } from "@solana/web3.js";

import { setGovernance } from "../instructions/lifecycle.js";
import type { AddressInput } from "../pda.js";
import { initializeDao } from "./instructions.js";
import * as fpda from "./pda.js";

export interface BootstrapGovernanceArgs {
  /** Reserved for the G3 live run (e.g. fetching the Squads treasury). Unused offline. */
  connection?: unknown;
  /** Rent payer + signer for both instructions. */
  payer: AddressInput;
  /** Signer that seeds the Dao PDA and becomes its config authority. */
  daoCreator: AddressInput;
  /** DAO base mint (KASS) — recorded as `Dao.base_mint`. */
  kassMint: AddressInput;
  /** DAO quote mint (USDC, 6-decimal) — recorded as `Dao.quote_mint`. */
  usdcMint: AddressInput;
  /** Squads `ProgramConfig.treasury` (read from the on-chain ProgramConfig in G3). */
  squadsProgramConfigTreasury: AddressInput;
  /** DAO nonce — seeds the Dao PDA `[b"dao", dao_creator, nonce]`. */
  nonce: bigint | number;
  // InitializeDaoParams (sensible defaults provided where omitted)
  twapInitialObservation: bigint | number;
  twapMaxObservationChangePerUpdate: bigint | number;
  twapStartDelaySeconds: number;
  minQuoteFutarchicLiquidity: bigint | number;
  minBaseFutarchicLiquidity: bigint | number;
  baseToStake: bigint | number;
  passThresholdBps: number;
  secondsPerProposal: number;
  /** Kassandra admin authority (signer) for the pre-handoff `set_governance`. */
  admin: AddressInput;
  /** Override the Kassandra program id. */
  kassandraProgramId?: Address;
}

export interface BootstrapGovernanceResult {
  /** The futarchy `Dao` PDA (== Squads multisig `create_key` == Kassandra `kass_dao`). */
  dao: Address;
  /** The Squads multisig PDA (`create_key == dao`). */
  multisig: Address;
  /** The Squads vault PDA at index 0 (== Kassandra `Protocol.dao_authority`). */
  vault: Address;
  /** The Squads program-config PDA. */
  programConfig: Address;
  /** The Squads spending-limit PDA (created None-empty by `initialize_dao`). */
  spendingLimit: Address;
  /** `[initialize_dao, set_governance]` — submit in order (G3). */
  instructions: TransactionInstruction[];
}

export async function bootstrapGovernance(
  a: BootstrapGovernanceArgs,
): Promise<BootstrapGovernanceResult> {
  const dao = (await fpda.dao(a.daoCreator, a.nonce)).address;
  const multisig = (await fpda.squadsMultisig(dao)).address;
  const vault = (await fpda.squadsVault(multisig, 0)).address;
  const programConfig = (await fpda.squadsProgramConfig()).address;
  const spendingLimit = (await fpda.squadsSpendingLimit(multisig, dao)).address;

  const initIx = await initializeDao({
    daoCreator: a.daoCreator,
    payer: a.payer,
    baseMint: a.kassMint,
    quoteMint: a.usdcMint,
    squadsProgramConfigTreasury: a.squadsProgramConfigTreasury,
    twapInitialObservation: a.twapInitialObservation,
    twapMaxObservationChangePerUpdate: a.twapMaxObservationChangePerUpdate,
    twapStartDelaySeconds: a.twapStartDelaySeconds,
    minQuoteFutarchicLiquidity: a.minQuoteFutarchicLiquidity,
    minBaseFutarchicLiquidity: a.minBaseFutarchicLiquidity,
    baseToStake: a.baseToStake,
    passThresholdBps: a.passThresholdBps,
    secondsPerProposal: a.secondsPerProposal,
    nonce: a.nonce,
  });

  const handoffIx = await setGovernance({
    authority: a.admin,
    daoAuthority: vault,
    kassDao: dao,
    programId: a.kassandraProgramId,
  });

  return {
    dao,
    multisig,
    vault,
    programConfig,
    spendingLimit,
    instructions: [initIx, handoffIx],
  };
}
