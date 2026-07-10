/**
 * DAO / admin instruction builders — the governance ops the participant flows
 * don't cover: `set_governance`, `set_config`, `resolve_deadend`, `kass_price`.
 * These are gated to the Protocol admin / DAO authority on-chain; the app exposes
 * them on the /admin page for completeness (and to make them e2e-drivable).
 */
import { Address, type TransactionInstruction } from "@solana/web3.js";
import {
  type SetConfigParams,
  futarchy,
  kassPrice,
  resolveDeadend,
  setConfig,
  setGovernance,
} from "@kassandra-market/oracles";

import { ValidationError, type AddressInput } from "../actions";

function addr(name: string, v: AddressInput): Address {
  try {
    return v instanceof Address ? v : new Address(v);
  } catch {
    throw new ValidationError(name, `${name} is not a valid address.`);
  }
}

/** A valid baseline `set_config` payload; `phaseWindow` is distinct so a write is observable. */
export const DEFAULT_CONFIG: SetConfigParams = {
  emissionNum: 0n,
  emissionDen: 1n,
  totalSupplyCap: 0n,
  feeEmaHalflife: 86_400n,
  feePerEmaUnit: 0n,
  feeEmaIncrement: 0n,
  thresholdNum: 2n,
  thresholdDen: 3n,
  marketThresholdNum: 1n,
  marketThresholdDen: 10n,
  flipSlashNum: 1n,
  flipSlashDen: 2n,
  phaseWindow: 7_201n,
  proposalWindow: 3_600n,
  factVoteSlashNum: 0n,
  factVoteSlashDen: 1n,
  rewardProposerWeight: 1n,
  rewardFactWeight: 1n,
  challengeFailUsdcFeeNum: 1n,
  challengeFailUsdcFeeDen: 100n,
  challengeSuccessKassFeeNum: 1n,
  challengeSuccessKassFeeDen: 100n,
  // Bootstrapping stake floor: curve pre-set to ~10/day → ~1000/day, magnitude
  // disabled (0) so participation stays free until governance activates it.
  stakeFloorEmaThreshold: 15_000_000_000n,
  stakeFloorEmaCap: 1_443_000_000_000n,
  stakeFloorMax: 0n,
};

/** `resolve_deadend` — DAO-gated final outcome for an Invalid-dead-end oracle. */
export async function buildResolveDeadendIxs(args: {
  oracle: AddressInput;
  authority: AddressInput;
  option: number;
}): Promise<TransactionInstruction[]> {
  return [
    await resolveDeadend({
      oracle: addr("oracle", args.oracle),
      authority: addr("authority", args.authority),
      option: args.option,
    }),
  ];
}

/** `set_config` — DAO-gated retune of the governable protocol params. */
export async function buildSetConfigIxs(args: {
  authority: AddressInput;
  params?: SetConfigParams;
}): Promise<TransactionInstruction[]> {
  return [await setConfig({ authority: addr("authority", args.authority), params: args.params ?? DEFAULT_CONFIG })]
}

/**
 * `set_governance` — the one-time DAO-linkage handoff. Records `dao_authority`
 * (derived as the Squads v4 vault of `kassDao`) + `kassDao` into the Protocol.
 */
export async function buildSetGovernanceIxs(args: {
  authority: AddressInput;
  kassDao: AddressInput;
}): Promise<TransactionInstruction[]> {
  const kassDao = addr("kassDao", args.kassDao);
  const multisig = (await futarchy.pda.squadsMultisig(kassDao)).address;
  const daoAuthority = (await futarchy.pda.squadsVault(multisig, 0)).address;
  return [await setGovernance({ authority: addr("authority", args.authority), daoAuthority, kassDao })]
}

/** `kass_price` — read the governance-anchored KASS/USDC spot TWAP. */
export async function buildKassPriceIxs(args: {
  kassDao: AddressInput;
}): Promise<TransactionInstruction[]> {
  return [await kassPrice({ kassDao: addr("kassDao", args.kassDao) })]
}
