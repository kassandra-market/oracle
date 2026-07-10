/**
 * KASS balance read helper (pure — takes a {@link Connection}, NO React).
 *
 * {@link fetchKassBalance} derives the owner's `ATA(owner, kassMint)` (the same
 * deriver the WF1 action layer uses) and reads its token-account balance,
 * returning the raw base-unit amount as a `bigint`.
 *
 * An ABSENT/uninitialised KASS ATA (the RPC throws "could not find account" or
 * returns no value) legitimately means a zero balance → **0n** is returned, not
 * thrown. A genuinely transient/unexpected RPC error propagates so the caller
 * (the hook) can treat it softly and NOT hard-block the form on a flaky fetch.
 */
import type { Connection } from '@solana/web3.js'
import { associatedTokenAccount } from '@kassandra-market/oracles'
import type { AddressInput } from '../data/actions'

/**
 * The owner's KASS balance in raw base units, or `0n` when the ATA is absent.
 * @throws only on a genuinely unexpected/transient RPC failure (never for a
 *   not-found ATA, which is caught and reported as `0n`).
 */
export async function fetchKassBalance(
  connection: Connection,
  owner: AddressInput,
  kassMint: AddressInput,
): Promise<bigint> {
  const ata = (await associatedTokenAccount(owner, kassMint)).address
  try {
    const res = await connection.getTokenAccountBalance(ata)
    // A present-but-empty response (no value) is treated as zero.
    if (!res?.value) return 0n
    return BigInt(res.value.amount)
  } catch (err) {
    if (isAccountNotFound(err)) return 0n
    throw err
  }
}

/** Whether an RPC error signals a missing/uninitialised account (→ zero balance). */
function isAccountNotFound(err: unknown): boolean {
  const msg = err instanceof Error ? err.message : String(err)
  return /could not find account|not found|invalid param|account does not exist/i.test(msg)
}
