import { configDefaults, defineConfig } from "vitest/config";

/**
 * The default `pnpm test` is fast + offline (litesvm + pure decoders). The
 * surfpool E2E suite under `test/surfpool/**` spawns a real validator and is
 * OPT-IN: it is only included when `KASSANDRA_MARKET_E2E=1` (`pnpm test:e2e`).
 * This keeps the default suite green/offline and prevents it from ever spawning
 * surfpool.
 */
const e2e = process.env.KASSANDRA_MARKET_E2E === "1";

// The in-process LiteSVM suites (each boots a native SVM + loads the ~1 MB
// MetaDAO BPF fixtures). The `litesvm` napi prebuilt crashes with a native
// `std::bad_alloc` on GitHub's hosted linux runner (the Rust `litesvm` crate
// loads the identical fixtures fine there). These are effectively local-node
// e2e tests; their coverage is fully duplicated on CI by the Rust program
// LiteSVM suite AND the offline surfpool e2e (both run the same flows against
// the real program + MetaDAO programs). `SKIP_LITESVM=1` excludes them on the
// hosted runner; they run in full locally + are covered by those green lanes.
const skipLitesvm = process.env.SKIP_LITESVM === "1";
const litesvmSuites = [
  "test/lifecycle.e2e.test.ts",
  "test/lifecycle-active.e2e.test.ts",
  "test/categorical.e2e.test.ts",
];

export default defineConfig({
  test: {
    exclude: [
      ...configDefaults.exclude,
      ...(e2e ? [] : ["test/surfpool/**"]),
      ...(skipLitesvm ? litesvmSuites : []),
    ],
    // Several suites each boot a heavy native LiteSVM instance. Run the test
    // files SERIALLY (one at a time) in isolated forks so only one native SVM is
    // live at a time and each file's fork frees its native memory on exit —
    // avoiding the SVM allocator crash from concurrent instances on CI runners.
    // (The surfpool suite likewise owns a single validator on a fixed port.)
    fileParallelism: false,
  },
});
