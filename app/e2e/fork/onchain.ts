/**
 * Read + patch on-chain accounts from the FORKED surfpool (port 8940) in the
 * Playwright test process — asserting each browser write by its persistent
 * on-chain effect, and fabricating the few balances/timestamps the forked
 * compose→settle flow needs (extra conditional tokens to swap; a past
 * `Market.twap_end` so the settle gate opens without waiting the TWAP window).
 */
import { decodeMarket } from '@kassandra-market/oracles'

import { tokenAccountAmount, tokenAccountBytes } from '../../../sdks/oracles/ts/test/surfpool/harness.ts'
import { KASSANDRA_PROGRAM as PROGRAM, TOKEN_PROGRAM, poll, surfpoolRpc } from '../rpc'

const { getAccountData, setAccountRaw } = surfpoolRpc('http://127.0.0.1:8940')
export { getAccountData, poll }

export async function marketAt(address: string): Promise<ReturnType<typeof decodeMarket>> {
  const data = await getAccountData(address)
  if (!data) throw new Error(`market ${address} not found`)
  return decodeMarket(data)
}

export async function tokenBalance(address: string): Promise<bigint> {
  const data = await getAccountData(address)
  if (!data) return 0n
  return tokenAccountAmount(data)
}

/** Fabricate a token account at `address` holding `amount` of `mint` for `owner`. */
export async function fabricateTokenAccount(
  address: string,
  mintBytesB58: Uint8Array,
  ownerBytes: Uint8Array,
  amount: bigint,
): Promise<void> {
  await setAccountRaw(address, tokenAccountBytes(mintBytesB58, ownerBytes, amount), TOKEN_PROGRAM)
}

/** Decode a v0.4 AMM's spot TWAP (created_at@9, last_updated@131, aggregator@171). */
export async function ammTwap(amm: string): Promise<bigint> {
  const data = await getAccountData(amm)
  if (!data) return 0n
  const dv = new DataView(data.buffer, data.byteOffset, data.length)
  const u128 = (off: number): bigint =>
    dv.getBigUint64(off, true) | (dv.getBigUint64(off + 8, true) << 64n)
  const createdAt = dv.getBigUint64(9, true)
  const lastUpdated = dv.getBigUint64(131, true)
  const aggregator = u128(171)
  const startDelay = dv.getBigUint64(219, true)
  const slots = lastUpdated - (createdAt + startDelay)
  return slots > 0n && aggregator > 0n ? aggregator / slots : 0n
}

/** Rewind `Market.twap_end` (i64 @ 392) to the past so settle opens immediately. */
export async function backdateMarketTwapEnd(market: string): Promise<void> {
  const data = await getAccountData(market)
  if (!data) throw new Error('market not found')
  const d = Uint8Array.from(data)
  const past = BigInt(Math.floor(Date.now() / 1000) - 3600)
  new DataView(d.buffer).setBigInt64(392, past, true)
  await setAccountRaw(market, d, PROGRAM)
}

/** Rewind the oracle's `phase_ends_at` (i64 @ 144) so the Challenge window has elapsed. */
export async function backdateOraclePhaseEnd(oracle: string): Promise<void> {
  const data = await getAccountData(oracle)
  if (!data) throw new Error('oracle not found')
  const d = Uint8Array.from(data)
  const past = BigInt(Math.floor(Date.now() / 1000) - 3600)
  new DataView(d.buffer).setBigInt64(144, past, true)
  await setAccountRaw(oracle, d, PROGRAM)
}

/** The oracle's phase discriminant byte (@161): 6=Challenge, 7=Resolved, 8=InvalidDeadend. */
export async function oraclePhaseByte(oracle: string): Promise<number | null> {
  const data = await getAccountData(oracle)
  return data ? data[161] : null
}
