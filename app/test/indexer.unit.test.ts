import { afterEach, describe, expect, it, vi } from 'vitest'

import {
  fetchAccountEvents,
  fetchOracleAccounts,
  fetchOracleDetailAccounts,
  indexerBaseUrl,
  isIndexerConfigured,
  type IndexedEvent,
} from '../src/data/indexer'

afterEach(() => {
  vi.unstubAllEnvs()
  vi.unstubAllGlobals()
})

describe('indexer client config', () => {
  it('is unconfigured when VITE_INDEXER_URL is unset/blank', () => {
    vi.stubEnv('VITE_INDEXER_URL', '')
    expect(indexerBaseUrl()).toBeUndefined()
    expect(isIndexerConfigured()).toBe(false)
  })

  it('normalizes a trailing slash', () => {
    vi.stubEnv('VITE_INDEXER_URL', 'https://idx.example.com/')
    expect(indexerBaseUrl()).toBe('https://idx.example.com')
    expect(isIndexerConfigured()).toBe(true)
  })
})

describe('fetchOracleAccounts (indexed account mirror)', () => {
  it('decodes the base64 account bytes from /oracles/accounts', async () => {
    vi.stubEnv('VITE_INDEXER_URL', 'https://idx.example.com')
    const fetchMock = vi.fn(async (url: string) => {
      expect(url).toBe('https://idx.example.com/oracles/accounts')
      // "AQID" == base64([1,2,3])
      return new Response(JSON.stringify({ count: 1, accounts: [{ pubkey: 'OraA', data: 'AQID' }] }), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      })
    })
    vi.stubGlobal('fetch', fetchMock)

    const accts = await fetchOracleAccounts()
    expect(accts).not.toBeNull()
    expect(accts!).toHaveLength(1)
    expect(accts![0].pubkey).toBe('OraA')
    expect(Array.from(accts![0].data)).toEqual([1, 2, 3])
  })

  it('returns null (→ caller falls back to getProgramAccounts) when unconfigured or on error', async () => {
    vi.stubEnv('VITE_INDEXER_URL', '')
    expect(await fetchOracleAccounts()).toBeNull()

    vi.stubEnv('VITE_INDEXER_URL', 'https://idx.example.com')
    vi.stubGlobal('fetch', vi.fn(async () => new Response('nope', { status: 503 })))
    expect(await fetchOracleAccounts()).toBeNull()
  })

  it('fetchOracleDetailAccounts carries the account_type tags + decodes data', async () => {
    vi.stubEnv('VITE_INDEXER_URL', 'https://idx.example.com')
    vi.stubGlobal(
      'fetch',
      vi.fn(async (url: string) => {
        expect(url).toBe('https://idx.example.com/oracles/OraA/accounts')
        return new Response(
          JSON.stringify({
            count: 2,
            accounts: [
              { pubkey: 'OraA', accountType: 1, data: 'AQID' },
              { pubkey: 'FactA1', accountType: 3, data: 'AQID' },
            ],
          }),
          { status: 200, headers: { 'content-type': 'application/json' } },
        )
      }),
    )
    const accts = await fetchOracleDetailAccounts('OraA')
    expect(accts!.map((a) => a.accountType)).toEqual([1, 3])
    expect(Array.from(accts![0].data)).toEqual([1, 2, 3])
  })
})

describe('fetchAccountEvents', () => {
  it('hits the account-events route and returns the events array', async () => {
    vi.stubEnv('VITE_INDEXER_URL', 'https://idx.example.com')
    const sample: IndexedEvent = {
      signature: 'Sig1',
      ixIndex: 0,
      ixType: 'propose',
      discriminant: 11,
      slot: 42,
      blockTime: 1_700_000_000,
      account0: 'OracleA',
      accounts: ['OracleA'],
      dataBase64: 'Cw==',
    }
    const fetchMock = vi.fn(async (url: string) => {
      expect(url).toBe('https://idx.example.com/accounts/OracleA/events?limit=25')
      return new Response(JSON.stringify({ count: 1, events: [sample] }), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      })
    })
    vi.stubGlobal('fetch', fetchMock)

    const events = await fetchAccountEvents('OracleA', { limit: 25 })
    expect(events).toEqual([sample])
    expect(fetchMock).toHaveBeenCalledOnce()
  })

  it('throws a clear error on a non-2xx response', async () => {
    vi.stubEnv('VITE_INDEXER_URL', 'https://idx.example.com')
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => new Response('nope', { status: 503 })),
    )
    await expect(fetchAccountEvents('OracleA')).rejects.toThrow(/503/)
  })

  it('throws when the indexer is not configured', async () => {
    vi.stubEnv('VITE_INDEXER_URL', '')
    await expect(fetchAccountEvents('OracleA')).rejects.toThrow(/not configured/)
  })
})
