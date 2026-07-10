#!/usr/bin/env bash
#
# fetch-metadao.sh — dump MetaDAO's on-chain program binaries into the test
# fixtures directory so the LiteSVM CPI tests are fully hermetic (no network at
# test time).
#
# ─────────────────────────────────────────────────────────────────────────────
# RESOLVED PROGRAM IDS (authoritatively sourced — do NOT edit from memory)
# ─────────────────────────────────────────────────────────────────────────────
#
#   conditional_vault  VLTX1ishMBbcX3rdBWGssxawAo1Q2X2qxYFYqiGodVg   (v0.4.0)
#   amm                AMMyu265tkBpRW21iGQxKGLaves3gKm2JcMUqfXNSpqD   (v0.4)
#
# SOURCE OF TRUTH:
#   * github.com/metaDAOproject/programs  (the futarchy repo moved here from
#     metaDAOproject/futarchy).
#   * conditional_vault: declare_id! in programs/conditional_vault/src/lib.rs on
#     `main` == VLTX1ishMBbcX3rdBWGssxawAo1Q2X2qxYFYqiGodVg, Cargo.toml
#     version = "0.4.0", security_txt source_release = "v0.4". Still the current
#     vault program — `main`'s Anchor.toml [programs.localnet].conditional_vault
#     matches.
#   * amm: declare_id! in programs/amm/src/lib.rs @ tag v0.4 (and v0.4
#     Anchor.toml [programs.localnet].amm) == AMMyu265tkBpRW21iGQxKGLaves3gKm2JcMUqfXNSpqD.
#     NOTE: a web search "guessed" AMMJdEiCCa8mdugg6JPF7gFirmmxisTfDJoSNSUi5zDJ
#     — that is NOT MetaDAO's declared amm id; the authoritative declare_id! is
#     AMMyu... (verified on-chain, see below).
#
# LATEST-VERSION NOTE:
#   MetaDAO governance v0.5/v0.6/v0.7 migrated market liquidity to Meteora DAMM
#   v2 (see programs/damm_v2_cpi in `main`), so there is no NEWER first-party
#   MetaDAO `amm` program than v0.4 — AMMyu... is the last standalone MetaDAO
#   AMM and the one whose built-in TWAP oracle matches our decision-market
#   design (§6). The conditional_vault (VLTX1...) remains current on `main`.
#
# ON-CHAIN VERIFICATION (mainnet-beta, captured 2026-06-29):
#   conditional_vault VLTX1ishMBbcX3rdBWGssxawAo1Q2X2qxYFYqiGodVg
#       owner BPFLoaderUpgradeab1e..., authority 6awyHMshBGVjJ3ozdSJdyyDE1CTAXUwrpNMaRGMsb4sf
#       last deployed slot 399213625, data length 424952 bytes
#   amm AMMyu265tkBpRW21iGQxKGLaves3gKm2JcMUqfXNSpqD
#       owner BPFLoaderUpgradeab1e..., authority 6awyHMshBGVjJ3ozdSJdyyDE1CTAXUwrpNMaRGMsb4sf
#       last deployed slot 326427490, data length 385416 bytes
#   (Both share upgrade authority 6awyHMshBGVjJ3ozdSJdyyDE1CTAXUwrpNMaRGMsb4sf,
#   confirming common MetaDAO provenance.)
#
# Idempotent: re-running overwrites the fixtures with a fresh dump from
# mainnet-beta. `solana program dump` always fetches the program account's
# current bytes; pinning is by program-id + the slots documented above.
#
set -euo pipefail

CONDITIONAL_VAULT_ID="VLTX1ishMBbcX3rdBWGssxawAo1Q2X2qxYFYqiGodVg"
AMM_ID="AMMyu265tkBpRW21iGQxKGLaves3gKm2JcMUqfXNSpqD"

# mainnet-beta RPC.
URL="${SOLANA_MAINNET_URL:-https://api.mainnet-beta.solana.com}"

# Fixtures live next to the LiteSVM tests so `include_bytes!`/runtime loads find
# them relative to programs/oracles/tests/.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
FIXTURE_DIR="${SCRIPT_DIR}/../programs/oracles/tests/fixtures"
mkdir -p "${FIXTURE_DIR}"

dump() {
    local id="$1" out="$2"
    echo "Dumping ${id} -> ${out}"
    solana program dump -u "${URL}" "${id}" "${out}"
}

dump "${CONDITIONAL_VAULT_ID}" "${FIXTURE_DIR}/metadao_conditional_vault.so"
dump "${AMM_ID}"               "${FIXTURE_DIR}/metadao_amm.so"

echo "Done. Fixtures in ${FIXTURE_DIR}:"
ls -l "${FIXTURE_DIR}"/*.so

# Pinned fixture hashes (captured 2026-06-29). `solana program dump` always
# fetches the program's CURRENT bytes, so if MetaDAO upgrades a program a re-run
# would silently change a fixture. Verify against these pins so drift is loud,
# not silent. If a mismatch is expected (intentional version bump), update both
# the hash here AND the documented slot/version above in the same commit.
EXPECTED_VAULT_SHA="bd19fac056e9d777ec0a4eb93d658293a31a7f4a2a701cda6eafb515009a1b89"
EXPECTED_AMM_SHA="c19026b4748c6f9d6dafd4c5ed46712150f9227b46858d16d543ebdb8b0dda1d"

sha_of() { shasum -a 256 "$1" | awk '{print $1}'; }
check() {
    local out="$1" expected="$2" got
    got="$(sha_of "${out}")"
    if [[ "${got}" != "${expected}" ]]; then
        echo "WARNING: sha256 drift for ${out}" >&2
        echo "  expected ${expected}" >&2
        echo "  got      ${got}" >&2
        echo "  MetaDAO may have upgraded this program; review before committing." >&2
        return 1
    fi
    echo "OK ${out} sha256=${got}"
}
echo "Verifying pinned hashes:"
check "${FIXTURE_DIR}/metadao_conditional_vault.so" "${EXPECTED_VAULT_SHA}"
check "${FIXTURE_DIR}/metadao_amm.so"               "${EXPECTED_AMM_SHA}"
