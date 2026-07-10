/**
 * Back-compat shim: the harness was split into the `harness/` folder module.
 * This file preserves the original `surfpool/harness.ts` import path for
 * consumers outside `test/` (e.g. `scripts/local-stack.mts`, which imports it by
 * explicit `.ts` path). It simply re-exports the folder module's public surface.
 */
export * from "./harness/index.js";
