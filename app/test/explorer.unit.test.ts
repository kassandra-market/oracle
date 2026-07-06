/**
 * Offline unit tests for `src/lib/explorer.ts` — the Solana Explorer tx-URL
 * builder (null on localnet, `?cluster=devnet` on devnet, bare on mainnet) and the
 * short-signature helper. Pure, no chain / React.
 */
import { describe, expect, it } from 'vitest'

import { explorerTxUrl, shortSig } from '../src/lib/explorer'

const SIG = '5xY5Q1abcdefghijkLMNOPqrstuvwxyz1234567890ABCDEF'

describe('explorerTxUrl', () => {
  it('returns null on localnet (nothing public to link to)', () => {
    expect(explorerTxUrl('localnet', SIG)).toBeNull()
  })

  it('appends ?cluster=devnet on devnet', () => {
    expect(explorerTxUrl('devnet', SIG)).toBe(
      `https://explorer.solana.com/tx/${SIG}?cluster=devnet`,
    )
  })

  it('uses the bare tx URL on mainnet-beta (no cluster param)', () => {
    expect(explorerTxUrl('mainnet-beta', SIG)).toBe(`https://explorer.solana.com/tx/${SIG}`)
  })
})

describe('shortSig', () => {
  it('returns short signatures unchanged (<= head + tail + 1)', () => {
    expect(shortSig('abcdefghi')).toBe('abcdefghi') // 9 == 4+4+1
    expect(shortSig('tiny')).toBe('tiny')
  })

  it('truncates the middle keeping head + tail', () => {
    expect(shortSig(SIG)).toBe(`${SIG.slice(0, 4)}…${SIG.slice(-4)}`)
    expect(shortSig(SIG, 6, 6)).toBe(`${SIG.slice(0, 6)}…${SIG.slice(-6)}`)
  })
})
