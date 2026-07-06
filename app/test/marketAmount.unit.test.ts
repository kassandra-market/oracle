/**
 * Offline unit tests for `src/market/data/amount.ts` — human-decimal KASS parsing
 * into raw base units (scale 10^9, rejecting malformed / over-precise / non-positive
 * input) and the additive balance gate. Pure, no React / chain.
 */
import { describe, expect, it } from 'vitest'

import { kassBalanceGateError, parseKassAmount } from '../src/market/data/amount'

describe('parseKassAmount', () => {
  it('scales human decimals into base units (10^9)', () => {
    expect(parseKassAmount('1')).toEqual({ value: 1_000_000_000n })
    expect(parseKassAmount('1.5')).toEqual({ value: 1_500_000_000n })
    expect(parseKassAmount('.25')).toEqual({ value: 250_000_000n }) // leading dot
    expect(parseKassAmount('1000')).toEqual({ value: 1_000_000_000_000n })
    expect(parseKassAmount('  2  ')).toEqual({ value: 2_000_000_000n }) // trimmed
    expect(parseKassAmount('0.123456789')).toEqual({ value: 123_456_789n }) // max precision
  })

  it('rejects empty / non-numeric input', () => {
    expect(parseKassAmount('').error).toMatch(/enter a kass amount/i)
    expect(parseKassAmount('   ').error).toMatch(/enter a kass amount/i)
    expect(parseKassAmount('abc').error).toMatch(/must be a number/i)
    expect(parseKassAmount('-1').error).toMatch(/must be a number/i) // sign not allowed
    expect(parseKassAmount('1e9').error).toMatch(/must be a number/i)
  })

  it('rejects more than 9 fractional digits', () => {
    expect(parseKassAmount('1.1234567890').error).toMatch(/at most 9 decimal/i)
  })

  it('rejects zero / effectively-zero amounts', () => {
    expect(parseKassAmount('0').error).toMatch(/greater than zero/i)
    expect(parseKassAmount('0.0').error).toMatch(/greater than zero/i)
    expect(parseKassAmount('0.000000000').error).toMatch(/greater than zero/i)
  })
})

describe('kassBalanceGateError', () => {
  it('never blocks on a null balance (disconnected / loading)', () => {
    expect(kassBalanceGateError(5n, null)).toBeUndefined()
    expect(kassBalanceGateError(undefined, null)).toBeUndefined()
  })

  it('blocks a zero balance outright', () => {
    expect(kassBalanceGateError(undefined, 0n)).toMatch(/no kass/i)
    expect(kassBalanceGateError(1n, 0n)).toMatch(/no kass/i)
  })

  it('blocks only when the amount exceeds the balance', () => {
    expect(kassBalanceGateError(150n, 100n)).toMatch(/exceeds your kass balance/i)
    expect(kassBalanceGateError(100n, 100n)).toBeUndefined() // equal is fine
    expect(kassBalanceGateError(50n, 100n)).toBeUndefined()
    expect(kassBalanceGateError(undefined, 100n)).toBeUndefined() // nothing entered yet
  })
})
