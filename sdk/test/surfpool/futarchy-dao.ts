/**
 * Shared test helper: fabricate a minimal futarchy `Dao` account blob carrying an
 * embedded spot AMM `TwapOracle` at the fixed byte offsets the program reads
 * (`aggregator`@9, `last_updated`@25, `created_at`@33, `start_delay`@105). Used by
 * every surfpool/e2e suite that needs a governance-blessed `kass_dao` (so
 * `open_challenge` sizing + `kass_price` have a readable TWAP), instead of each
 * copy-pasting the same builder.
 */

/** The v0.6 futarchy `Dao` Anchor account discriminator (first 8 bytes). */
export const FUTARCHY_DAO_DISC = Uint8Array.from([
  0xa3, 0x09, 0x2f, 0x1f, 0x34, 0x55, 0xc5, 0x31,
]);

/**
 * Build a 141-byte futarchy `Dao` blob: the DAO discriminator, `PoolState::Spot`
 * (`data[8] = 0`), a u128 `aggregator`, and the i64 `last_updated` / `created_at`
 * / u32 `start_delay` TWAP fields. The embedded spot TWAP resolves to
 * `aggregator / (last_updated - created_at - start_delay)`.
 */
export function buildDaoBlob(
  aggregator: bigint,
  lastUpdated: bigint,
  createdAt: bigint,
  startDelay = 0,
): Uint8Array {
  const data = new Uint8Array(141);
  data.set(FUTARCHY_DAO_DISC, 0);
  data[8] = 0; // PoolState::Spot
  const dv = new DataView(data.buffer);
  dv.setBigUint64(9, aggregator & 0xffffffffffffffffn, true);
  dv.setBigUint64(17, aggregator >> 64n, true);
  dv.setBigInt64(25, lastUpdated, true);
  dv.setBigInt64(33, createdAt, true);
  dv.setUint32(105, startDelay, true);
  return data;
}
