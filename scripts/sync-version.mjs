#!/usr/bin/env node
/**
 * Single-source versioning: the Rust workspace `[workspace.package].version` in
 * the root Cargo.toml is the ONE source of truth. Every Rust crate inherits it
 * via `version.workspace = true`; this script stamps that same value into each
 * published TypeScript SDK's package.json (npm has no native workspace-version
 * inheritance).
 *
 * Usage:
 *   node scripts/sync-version.mjs            # write the workspace version into each TS package
 *   node scripts/sync-version.mjs --check    # verify only; exit 1 if any package is out of sync
 *
 * Bump the version in ONE place (Cargo [workspace.package].version), re-run this,
 * and commit — the publish workflow releases whatever isn't already on the registry.
 */
import { readFileSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = join(dirname(fileURLToPath(import.meta.url)), '..')
const check = process.argv.includes('--check')

// The TS packages that mirror the workspace version.
const TS_PACKAGES = ['sdks/oracles/ts/package.json', 'sdks/markets/ts/package.json']

/** Read `version` from the `[workspace.package]` table of the root Cargo.toml. */
function workspaceVersion() {
  const cargo = readFileSync(join(root, 'Cargo.toml'), 'utf8')
  // Isolate the [workspace.package] table (up to the next top-level table), then
  // read its `version = "…"`. Keeps us from matching a crate/dependency version.
  const table = cargo.match(/\[workspace\.package\]([\s\S]*?)(?:\n\[|$)/)
  const m = table && table[1].match(/\bversion\s*=\s*"([^"]+)"/)
  if (!m) {
    console.error('sync-version: could not find [workspace.package].version in Cargo.toml')
    process.exit(1)
  }
  return m[1]
}

const version = workspaceVersion()
let outOfSync = 0

for (const rel of TS_PACKAGES) {
  const path = join(root, rel)
  const pkg = JSON.parse(readFileSync(path, 'utf8'))
  if (pkg.version === version) {
    console.log(`  ${pkg.name}: ${version} (ok)`)
    continue
  }
  outOfSync++
  if (check) {
    console.error(`  ${pkg.name}: ${pkg.version} != ${version} (out of sync)`)
  } else {
    const prev = pkg.version
    pkg.version = version
    writeFileSync(path, JSON.stringify(pkg, null, 2) + '\n')
    console.log(`  ${pkg.name}: ${prev} → ${version}`)
  }
}

// The internal [workspace.dependencies] path deps carry an explicit `version` (so
// the SDK crates can be published to crates.io). Keep those pinned to the same
// workspace version — a `{ path = "…", version = "X" }` entry.
const cargoPath = join(root, 'Cargo.toml')
let cargo = readFileSync(cargoPath, 'utf8')
const depVersionRe = /(path = "[^"]*", version = ")([^"]+)(")/g
let cargoChanged = false
cargo = cargo.replace(depVersionRe, (m, pre, cur, post) => {
  if (cur === version) return m
  outOfSync++
  cargoChanged = true
  if (check) {
    console.error(`  Cargo path-dep version: ${cur} != ${version} (out of sync)`)
    return m
  }
  return pre + version + post
})
if (!check && cargoChanged) {
  writeFileSync(cargoPath, cargo)
  console.log(`  Cargo [workspace.dependencies] path-dep versions → ${version}`)
}

if (check && outOfSync > 0) {
  console.error(
    `sync-version: ${outOfSync} package(s) out of sync with the workspace version ${version}. Run \`node scripts/sync-version.mjs\`.`,
  )
  process.exit(1)
}
console.log(`sync-version: workspace version ${version}`)
