/**
 * Meteora **DAMM v2** (cp-amm, `cpamd…`) SDK surface (Task M1).
 *
 * Re-exports the 6 spot-path instruction builders (`initializePool`,
 * `createPosition`, `addLiquidity`, `removeLiquidity`, `swap`,
 * `claimPositionFee`), the `Pool`/`Position` zero-copy decoders, the wire
 * constants (discriminators, seeds, sizes), and the PDA derivers under the `pda`
 * namespace (e.g. `meteora.pda.pool(config, a, b)`).
 *
 * cp-amm is the DAO's SPOT-liquidity venue — POSITION-based, no built-in oracle.
 * Kassandra does NOT CPI it; these builders/decoders are for the treasury side.
 * All wire formats are byte-sourced from MeteoraAg/damm-v2 @ commit
 * `bdd8a1e355f484b3cff131578a662c560b97b72f` (see `./constants.ts`).
 */
export * from "./constants.js";
export * from "./instructions.js";
export * from "./accounts.js";
export * as pda from "./pda.js";
