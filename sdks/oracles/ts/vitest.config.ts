import { configDefaults, defineConfig } from "vitest/config";

/**
 * The default `pnpm test` is fast + offline (litesvm + pure decoders). The
 * surfpool E2E suite under `test/surfpool/**` spawns a real validator and is
 * OPT-IN: it is only included when `KASSANDRA_E2E=1` (`pnpm test:e2e`). This
 * keeps the default suite green/offline and prevents it from ever spawning
 * surfpool.
 */
const e2e = process.env.KASSANDRA_E2E === "1";

export default defineConfig({
  test: {
    exclude: e2e
      ? configDefaults.exclude
      : [...configDefaults.exclude, "test/surfpool/**"],
    // The surfpool suite owns a single validator on a fixed port — never run
    // its files in parallel with each other.
    ...(e2e ? { fileParallelism: false } : {}),
  },
});
