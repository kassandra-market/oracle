/**
 * Offline unit tests for the KASS balance read helper (default suite — no
 * network). A mock {@link Connection} records the queried address and either
 * resolves a token-account balance or throws an absent-ATA error; we assert:
 *   - a present ATA → `fetchKassBalance` returns the raw `bigint` amount, and
 *     the address queried is exactly `ATA(owner, kassMint)`;
 *   - an absent ATA (the RPC throws "could not find account") → `0n`, NOT thrown.
 */
import { Keypair, type Connection } from '@solana/web3.js'
import { associatedTokenAccount } from '@kassandra-market/oracles'
import { describe, expect, it } from 'vitest'

import { fetchKassBalance } from '../src/data/balance.ts'

async function fixture() {
  const owner = (await Keypair.generate()).publicKey
  const kassMint = (await Keypair.generate()).publicKey
  const ata = (await associatedTokenAccount(owner, kassMint)).address
  return { owner, kassMint, ata }
}

describe('fetchKassBalance', () => {
  it('returns the balance bigint and queries the derived ATA', async () => {
    const { owner, kassMint, ata } = await fixture()
    let queried: string | undefined
    const connection = {
      getTokenAccountBalance: async (address: { toString(): string }) => {
        queried = address.toString()
        return { value: { amount: '123', decimals: 9 } }
      },
    } as unknown as Connection

    const balance = await fetchKassBalance(connection, owner, kassMint)
    expect(balance).toBe(123n)
    expect(queried).toBe(ata.toString())
  })

  it('returns 0n when the ATA is absent (the RPC throws) instead of throwing', async () => {
    const { owner, kassMint } = await fixture()
    const connection = {
      getTokenAccountBalance: async () => {
        throw new Error(
          'failed to get token account balance: Invalid param: could not find account',
        )
      },
    } as unknown as Connection

    await expect(fetchKassBalance(connection, owner, kassMint)).resolves.toBe(0n)
  })
})
