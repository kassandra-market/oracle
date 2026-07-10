/**
 * Offline unit tests for the CU1 v0.4 `Amm` decoder + price/TWAP/margin helpers
 * (`src/data/ammV04.ts`). `decodeAmmV04` runs against a HAND-BUILT `Amm` byte
 * blob — fields written at the exact little-endian offsets the on-chain reader
 * uses (`programs/oracles/src/cpi/metadao.rs:145-178`) — so it exercises the
 * genuine byte shape, not a hand-waved object. The math helpers run on known
 * inputs incl. the pre-start-delay div-guard (→ null) and a near-margin case.
 * No network.
 */
import { Address } from '@solana/web3.js'
import { describe, expect, it } from 'vitest'

import {
  AMM_ACCOUNT_DISCRIMINATOR,
  AMM_AGGREGATOR_OFFSET,
  AMM_BASE_AMOUNT_OFFSET,
  AMM_BASE_DECIMALS_OFFSET,
  AMM_BASE_MINT_OFFSET,
  AMM_CREATED_AT_SLOT_OFFSET,
  AMM_LAST_UPDATED_SLOT_OFFSET,
  AMM_MIN_LEN,
  AMM_QUOTE_AMOUNT_OFFSET,
  AMM_QUOTE_DECIMALS_OFFSET,
  AMM_QUOTE_MINT_OFFSET,
  AMM_START_DELAY_SLOTS_OFFSET,
  AmmDecodeError,
  PRICE_SCALE,
  decodeAmmV04,
  instantaneousPrice,
  marginProgress,
  twapPrice,
  willDisqualify,
  type AmmV04,
} from '../src/data/ammV04'

// --- byte-layout builder (disc + fields at their pinned offsets) -------------

interface AmmFields {
  createdAtSlot: bigint
  lastUpdatedSlot: bigint
  startDelaySlots: bigint
  aggregator: bigint
  baseAmount: bigint
  quoteAmount: bigint
  baseDecimals: number
  quoteDecimals: number
  baseMint: Uint8Array
  quoteMint: Uint8Array
}

function encodeAmm(f: AmmFields, len = AMM_MIN_LEN + 8): Uint8Array {
  const buf = new Uint8Array(len)
  buf.set(AMM_ACCOUNT_DISCRIMINATOR, 0)
  const dv = new DataView(buf.buffer)
  dv.setBigUint64(AMM_CREATED_AT_SLOT_OFFSET, f.createdAtSlot, true)
  buf.set(f.baseMint, AMM_BASE_MINT_OFFSET)
  buf.set(f.quoteMint, AMM_QUOTE_MINT_OFFSET)
  buf[AMM_BASE_DECIMALS_OFFSET] = f.baseDecimals
  buf[AMM_QUOTE_DECIMALS_OFFSET] = f.quoteDecimals
  dv.setBigUint64(AMM_BASE_AMOUNT_OFFSET, f.baseAmount, true)
  dv.setBigUint64(AMM_QUOTE_AMOUNT_OFFSET, f.quoteAmount, true)
  dv.setBigUint64(AMM_LAST_UPDATED_SLOT_OFFSET, f.lastUpdatedSlot, true)
  dv.setBigUint64(AMM_AGGREGATOR_OFFSET, f.aggregator & 0xffffffffffffffffn, true)
  dv.setBigUint64(AMM_AGGREGATOR_OFFSET + 8, f.aggregator >> 64n, true)
  dv.setBigUint64(AMM_START_DELAY_SLOTS_OFFSET, f.startDelaySlots, true)
  return buf
}

const seed = (n: number): Uint8Array => Uint8Array.from({ length: 32 }, (_, i) => (n + i) & 0xff)

const BASE_MINT = seed(10)
const QUOTE_MINT = seed(60)

const SAMPLE: AmmFields = {
  createdAtSlot: 1000n,
  lastUpdatedSlot: 2150n, // 1000 + 150 start-delay + 1000 accumulating
  startDelaySlots: 150n,
  aggregator: PRICE_SCALE * 1000n, // twap 1.0 * 1000 slots
  baseAmount: 1_000_000_000_000n, // 1000 * 1e9
  quoteAmount: 1_000_000_000n, // 1000 * 1e6
  baseDecimals: 9,
  quoteDecimals: 6,
  baseMint: BASE_MINT,
  quoteMint: QUOTE_MINT,
}

describe('decodeAmmV04', () => {
  it('reads every field at its pinned offset', () => {
    const amm = decodeAmmV04(encodeAmm(SAMPLE))
    expect(amm.createdAtSlot).toBe(1000n)
    expect(amm.lastUpdatedSlot).toBe(2150n)
    expect(amm.startDelaySlots).toBe(150n)
    expect(amm.aggregator).toBe(PRICE_SCALE * 1000n)
    expect(amm.baseAmount).toBe(1_000_000_000_000n)
    expect(amm.quoteAmount).toBe(1_000_000_000n)
    expect(amm.baseDecimals).toBe(9)
    expect(amm.quoteDecimals).toBe(6)
    expect(amm.baseMint).toEqual(new Address(BASE_MINT))
    expect(amm.quoteMint).toEqual(new Address(QUOTE_MINT))
  })

  it('decodes a full u128 aggregator (both 64-bit halves)', () => {
    const big = (12345n << 64n) | 6789n
    const amm = decodeAmmV04(encodeAmm({ ...SAMPLE, aggregator: big }))
    expect(amm.aggregator).toBe(big)
  })

  it('throws on a wrong discriminator', () => {
    const buf = encodeAmm(SAMPLE)
    buf[0] = 0x00
    expect(() => decodeAmmV04(buf)).toThrow(AmmDecodeError)
  })

  it('throws on a too-short buffer', () => {
    const short = encodeAmm(SAMPLE).slice(0, AMM_MIN_LEN - 1)
    expect(() => decodeAmmV04(short)).toThrow(AmmDecodeError)
  })
})

const decode = (f: Partial<AmmFields> = {}): AmmV04 => decodeAmmV04(encodeAmm({ ...SAMPLE, ...f }))

describe('instantaneousPrice', () => {
  it('is decimals-aware quote/base', () => {
    // 1000 USDC (6dec) / 1000 KASS (9dec) => spot 1.0
    expect(instantaneousPrice(decode())).toBeCloseTo(1.0, 9)
  })

  it('reflects an imbalanced pool', () => {
    // 1045 USDC / 1000 KASS => 1.045
    expect(instantaneousPrice(decode({ quoteAmount: 1_045_000_000n }))).toBeCloseTo(1.045, 9)
  })

  it('returns null on an empty base reserve', () => {
    expect(instantaneousPrice(decode({ baseAmount: 0n }))).toBeNull()
  })
})

describe('twapPrice', () => {
  it('mirrors get_twap: aggregator / elapsed slots', () => {
    // aggregator = 1.0*PRICE_SCALE*1000, slots = 2150 - (1000+150) = 1000 => 1.0*PRICE_SCALE
    expect(twapPrice(decode())).toBe(PRICE_SCALE)
  })

  it('returns null pre-start-delay (div-by-zero guard)', () => {
    // last_updated == created + start_delay => 0 elapsed slots
    expect(twapPrice(decode({ lastUpdatedSlot: 1150n }))).toBeNull()
  })

  it('returns null when last_updated precedes the start slot', () => {
    expect(twapPrice(decode({ lastUpdatedSlot: 1100n }))).toBeNull()
  })

  it('returns null on a zero aggregator (no observations)', () => {
    expect(twapPrice(decode({ aggregator: 0n }))).toBeNull()
  })
})

describe('marginProgress', () => {
  // margin 1/10 => disqualify when fail > pass*1.1; the divergence progress is
  // (fail-pass)*DEN/(pass*NUM): 0 at no divergence, 1 exactly at the margin.
  const NUM = 1n
  const DEN = 10n

  it('is ~0.90 for a near-margin fail (1.09 vs 1.0)', () => {
    const pass = PRICE_SCALE // 1.0
    const fail = (PRICE_SCALE * 109n) / 100n // 1.09
    expect(marginProgress(fail, pass, NUM, DEN)).toBeCloseTo(0.9, 3)
  })

  it('is exactly 1 at the margin (fail == pass*1.1)', () => {
    const pass = PRICE_SCALE
    const fail = (PRICE_SCALE * 110n) / 100n // 1.10x == margin
    expect(marginProgress(fail, pass, NUM, DEN)).toBeCloseTo(1, 6)
  })

  it('reaches > 1 once fail clears the margin (1.11x pass)', () => {
    const pass = PRICE_SCALE
    const fail = (PRICE_SCALE * 111n) / 100n // 1.11x > 1.10x margin
    expect(marginProgress(fail, pass, NUM, DEN)).toBeGreaterThan(1)
  })

  it('is 0 when fail == pass (no divergence)', () => {
    expect(marginProgress(PRICE_SCALE, PRICE_SCALE, NUM, DEN)).toBe(0)
  })

  it('is 0 when fail is below pass', () => {
    expect(marginProgress(PRICE_SCALE / 2n, PRICE_SCALE, NUM, DEN)).toBe(0)
  })

  it('returns 0 on a null TWAP', () => {
    expect(marginProgress(null, PRICE_SCALE, NUM, DEN)).toBe(0)
    expect(marginProgress(PRICE_SCALE, null, NUM, DEN)).toBe(0)
  })

  it('returns 0 when pass TWAP is zero (always survives)', () => {
    expect(marginProgress(PRICE_SCALE, 0n, NUM, DEN)).toBe(0)
  })
})

describe('willDisqualify (exact on-chain boundary: fail*DEN > pass*(DEN+NUM))', () => {
  const NUM = 1n
  const DEN = 10n

  it('SURVIVES at exact equality (fail == pass*1.1) — strict `>`, measure-zero boundary', () => {
    const pass = PRICE_SCALE // 1.0
    const fail = (PRICE_SCALE * 11n) / 10n // exactly pass*(DEN+NUM)/DEN == 1.1x
    // marginProgress reads ~1 here, but the on-chain strict `>` does NOT disqualify.
    expect(marginProgress(fail, pass, NUM, DEN)).toBeCloseTo(1, 6)
    expect(willDisqualify(fail, pass, NUM, DEN)).toBe(false)
  })

  it('DISQUALIFIES one tick past the boundary', () => {
    const pass = PRICE_SCALE
    const fail = (PRICE_SCALE * 11n) / 10n + 1n // exactly one PRICE_SCALE unit over
    expect(willDisqualify(fail, pass, NUM, DEN)).toBe(true)
  })

  it('SURVIVES one tick below the boundary', () => {
    const pass = PRICE_SCALE
    const fail = (PRICE_SCALE * 11n) / 10n - 1n
    expect(willDisqualify(fail, pass, NUM, DEN)).toBe(false)
  })

  it('SURVIVES a near-margin fail (1.09 vs 1.0)', () => {
    const pass = PRICE_SCALE
    const fail = (PRICE_SCALE * 109n) / 100n // 1.09 < 1.10 margin
    expect(willDisqualify(fail, pass, NUM, DEN)).toBe(false)
  })

  it('SURVIVES with no divergence / below-pass fail', () => {
    expect(willDisqualify(PRICE_SCALE, PRICE_SCALE, NUM, DEN)).toBe(false)
    expect(willDisqualify(PRICE_SCALE / 2n, PRICE_SCALE, NUM, DEN)).toBe(false)
  })

  it('SURVIVES on a null TWAP or zero pass price (settle guard)', () => {
    expect(willDisqualify(null, PRICE_SCALE, NUM, DEN)).toBe(false)
    expect(willDisqualify(PRICE_SCALE, null, NUM, DEN)).toBe(false)
    expect(willDisqualify(PRICE_SCALE * 2n, 0n, NUM, DEN)).toBe(false)
  })
})
