/**
 * Futarchy v0.6 + Squads v4 + conditional_vault SDK surface (Task G2).
 *
 * Re-exports the instruction builders, the wire constants (discriminators,
 * seeds, `Market`/`SwapType` enums), and the governance bootstrap. PDA derivers
 * are under the `pda` namespace (e.g. `futarchy.pda.dao(creator, nonce)`).
 *
 * See `./NOTES.md` for the authoritative layout map + the CONFIRMED
 * `create_key == Dao PDA` finding. The Meteora DAMM v2 builders live in the
 * sibling `../meteora` module, not here.
 */
export * from "./constants.js";
export * from "./instructions.js";
export * from "./bootstrap.js";
export * as pda from "./pda.js";
