/**
 * Canonical SPL `Mint` / token-`Account` + Kassandra `Oracle` byte-layout
 * fabrication, shared by the LiteSVM harness (`harness.ts`) and the surfpool
 * harness (`surfpool/harness.ts`). Both stand up accounts by writing these exact
 * packed byte layouts (no `InitializeMint` CPI). All inputs are raw `Uint8Array`
 * pubkey bytes — an `Address` caller passes `.toBytes()`.
 */

// --- SPL Mint (82 bytes) ------------------------------------------------------
const MINT_LEN = 82;

/** Pack an 82-byte SPL `Mint` (COption authority tag 1 = Some, freeze = None). */
export function mintBytes(authority: Uint8Array, supply: bigint, decimals: number): Uint8Array {
  const data = new Uint8Array(MINT_LEN);
  const dv = new DataView(data.buffer);
  dv.setUint32(0, 1, true); // mint_authority COption tag = Some
  data.set(authority, 4);
  dv.setBigUint64(36, supply, true); // supply
  data[44] = decimals; // decimals
  data[45] = 1; // is_initialized
  return data;
}

// --- SPL token Account (165 bytes) --------------------------------------------
const TOKEN_ACCOUNT_LEN = 165;

/** Pack a 165-byte SPL token `Account` holding `amount` of `mint`, owned by `owner`. */
export function tokenAccountBytes(mint: Uint8Array, owner: Uint8Array, amount: bigint): Uint8Array {
  const data = new Uint8Array(TOKEN_ACCOUNT_LEN);
  const dv = new DataView(data.buffer);
  data.set(mint, 0); // mint
  data.set(owner, 32); // owner
  dv.setBigUint64(64, amount, true); // amount
  data[108] = 1; // state = Initialized
  return data;
}

/** Read the `amount` (u64 @ offset 64) out of SPL token-account bytes. */
export function tokenAccountAmount(data: Uint8Array): bigint {
  return new DataView(data.buffer, data.byteOffset, data.length).getBigUint64(64, true);
}

// --- Kassandra Oracle (392 bytes) — mirrors kassandra `state.rs` --------------
const ORACLE_LEN = 392; // Oracle::LEN — `load_kassandra_oracle` requires ≥ this.
const ORACLE_ACCOUNT_TYPE_OFFSET = 0;
const ORACLE_OPTIONS_COUNT_OFFSET = 160;
const ORACLE_PHASE_OFFSET = 161;
const ORACLE_RESOLVED_OPTION_OFFSET = 197;
const KASS_ACCOUNT_TYPE_ORACLE = 1; // kassandra `AccountType::Oracle`.

/** A 392-byte Kassandra `Oracle` carrying just the fields the market reads. */
export function oracleBytes(optionsCount: number, phase: number, resolvedOption: number): Uint8Array {
  const data = new Uint8Array(ORACLE_LEN);
  data[ORACLE_ACCOUNT_TYPE_OFFSET] = KASS_ACCOUNT_TYPE_ORACLE;
  data[ORACLE_OPTIONS_COUNT_OFFSET] = optionsCount;
  data[ORACLE_PHASE_OFFSET] = phase;
  data[ORACLE_RESOLVED_OPTION_OFFSET] = resolvedOption;
  return data;
}
