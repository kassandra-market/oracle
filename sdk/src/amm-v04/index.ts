/**
 * MetaDAO **v0.4 standalone AMM** (`AMMyu265…`) SDK surface (Task CS1).
 *
 * Re-exports the 4 instruction builders (`createAmm`, `addLiquidity`, `swap`,
 * `crankThatTwap`), the wire constants (discriminators, seeds, `SwapType`), and
 * the PDA/ATA derivers under the `pda` namespace (e.g. `ammV04.pda.amm(b, q)`).
 *
 * This is the standalone AMM whose built-in TWAP oracle `settle_challenge`
 * reads — a DIFFERENT program from the v0.6 futarchy embedded AMM
 * (`src/futarchy`). The two are not interchangeable.
 */
export * from "./constants.js";
export * from "./instructions.js";
export * as pda from "./pda.js";
