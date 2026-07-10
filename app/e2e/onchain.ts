/**
 * Read + decode on-chain accounts from surfpool in the Playwright test process,
 * so a browser write can be asserted by its PERSISTENT on-chain effect (the UI
 * success line is transient — it is cleared by the post-write refetch).
 *
 * The low-level JSON-RPC / surfnet plumbing lives in `./rpc.ts` (shared with the
 * forked project); this module binds it to the default :8899 endpoint and adds
 * the Kassandra account decoders + clock helpers.
 */
import { Address } from '@solana/web3.js'
import { decodeFact, decodeOracle, decodeProposer, decodeProtocol } from '@kassandra-market/oracles'

import { KASSANDRA_PROGRAM, poll, surfpoolRpc } from './rpc'

const { rpc, getAccountData, setAccountRaw } = surfpoolRpc('http://127.0.0.1:8899')
export { getAccountData, poll, setAccountRaw }

/** The on-chain Clock sysvar `unix_timestamp` — what the program's `now()` reads. */
export async function clockUnix(): Promise<number> {
  const data = await getAccountData('SysvarC1ock11111111111111111111111111111111')
  if (!data) throw new Error('Clock sysvar not readable')
  return Number(Buffer.from(data).readBigInt64LE(32))
}

/**
 * Set the surfpool clock so the program's `now()` lands at ~`targetUnix`, by
 * jumping the absolute slot (surfpool moves unix at ~0.4 s/slot). Because the
 * shared clock is advanced far forward by seeding, each spec resets it into its
 * oracle's phase window right before acting (tests run serially, one oracle each).
 */
export async function setClockTo(targetUnix: number): Promise<void> {
  const cur = await clockUnix()
  const slot = await rpc<number>('getSlot', [])
  const delta = Math.round((targetUnix - cur) / 0.4)
  await rpc('surfnet_timeTravel', [{ absoluteSlot: Math.max(1, slot + delta) }])
  await new Promise((r) => setTimeout(r, 250))
}

export async function oracleAt(address: string): Promise<ReturnType<typeof decodeOracle>> {
  const data = await getAccountData(address)
  if (!data) throw new Error(`oracle ${address} not found`)
  return decodeOracle(data)
}

export async function factAt(address: string): Promise<ReturnType<typeof decodeFact>> {
  const data = await getAccountData(address)
  if (!data) throw new Error(`fact ${address} not found`)
  return decodeFact(data)
}

export async function proposerAt(address: string): Promise<ReturnType<typeof decodeProposer>> {
  const data = await getAccountData(address)
  if (!data) throw new Error(`proposer ${address} not found`)
  return decodeProposer(data)
}

export async function protocolAt(address: string): Promise<ReturnType<typeof decodeProtocol>> {
  const data = await getAccountData(address)
  if (!data) throw new Error(`protocol ${address} not found`)
  return decodeProtocol(data)
}

/**
 * Fabricate governance fields on the Protocol singleton (admin @8, governance_set
 * @121, dao_authority @128, kass_dao @160) so the connected wallet can drive each
 * DAO-gated op — the real set_governance requires a Squads vault PDA no keypair
 * can sign, so admin/DAO tests fabricate the linkage directly.
 */
export async function patchProtocol(
  protocol: string,
  fields: { admin?: string; daoAuthority?: string; governanceSet?: boolean; kassDao?: string },
): Promise<void> {
  const cur = await getAccountData(protocol)
  if (!cur) throw new Error('protocol not found')
  const d = Uint8Array.from(cur)
  if (fields.admin) d.set(new Address(fields.admin).toBytes(), 8)
  if (fields.governanceSet !== undefined) d[121] = fields.governanceSet ? 1 : 0
  if (fields.daoAuthority) d.set(new Address(fields.daoAuthority).toBytes(), 128)
  if (fields.kassDao) d.set(new Address(fields.kassDao).toBytes(), 160)
  await setAccountRaw(protocol, d, KASSANDRA_PROGRAM)
}
