/**
 * Higher-level futarchy-governance fixture flows shared by the split T-G3 E2E
 * test files: boot surfpool FORKING MAINNET + init the Kassandra protocol
 * (`setupFixture`), and the real `initialize_dao` + Squads multisig + G1
 * `set_governance` handoff every downstream governance arm needs as a
 * precondition (`bootstrapDao`). The bodies mirror the original E2E `beforeAll`
 * and BOOTSTRAP setup verbatim (minus the BOOTSTRAP arm's own assertions).
 */
import { Address, Keypair } from "@solana/web3.js";

import { TOKEN_PROGRAM_ID } from "../../src/constants.js";
import { initProtocol } from "../../src/instructions/index.js";
import * as pda from "../../src/pda.js";
import * as futarchy from "../../src/futarchy/index.js";

import { SurfpoolHarness, mintBytes, toHex } from "./harness.js";
import { type Fixture, sendIx } from "./futarchy-governance-harness.js";

/**
 * Boot surfpool FORKING MAINNET (so MetaDAO's deployed programs execute over
 * RPC), fund the payer, materialise real KASS (9dp) + USDC (6dp) mints, and run
 * the real `initialize_protocol`. Returns the fixture with `dao/multisig/vault`
 * still unset (call `bootstrapDao` to fill them).
 */
export async function setupFixture(port: number): Promise<Fixture> {
  const harness = await SurfpoolHarness.start({
    port,
    fork: "mainnet",
    readyTimeoutMs: 60_000,
  });
  const payer = await Keypair.generate();
  await harness.airdrop(payer.publicKey.toString(), 1_000_000_000_000);

  // Real KASS (9dp) + USDC (MUST be 6dp — initialize_dao `mint::decimals = 6`).
  const mintAuth = await pda.mintAuthority();
  const kassMint = await Keypair.generate();
  const usdcMint = await Keypair.generate();
  await harness.setAccount(kassMint.publicKey.toString(), {
    lamports: 1_000_000_000,
    owner: TOKEN_PROGRAM_ID.toString(),
    executable: false,
    data: toHex(mintBytes(mintAuth.address.toBytes(), 0n, 9)),
  });
  await harness.setAccount(usdcMint.publicKey.toString(), {
    lamports: 1_000_000_000,
    owner: TOKEN_PROGRAM_ID.toString(),
    executable: false,
    data: toHex(mintBytes(payer.publicKey.toBytes(), 0n, 6)),
  });

  const f: Fixture = {
    harness,
    payer,
    kassMint,
    usdcMint,
    dao: undefined as unknown as Address,
    multisig: undefined as unknown as Address,
    vault: undefined as unknown as Address,
  };

  await sendIx(
    f,
    await initProtocol({
      admin: payer.publicKey,
      kassMint: kassMint.publicKey,
      usdcMint: usdcMint.publicKey,
    }),
  );
  return f;
}

/**
 * Real `initialize_dao` (creates the Dao + the Squads multisig with
 * create_key==Dao + vault atomically; treasury fetched live) → the G1-hardened
 * `set_governance(kass_dao=Dao, dao_authority=vault)`. Mutates `f` in place with
 * the resulting `dao`, `multisig`, and `vault`.
 */
export async function bootstrapDao(f: Fixture): Promise<void> {
  const nonce = 1n;
  // Squads ProgramConfig.treasury is NOT a PDA — fetch it live from the
  // on-chain ProgramConfig account (treasury @ offset 8+32+8 = 48).
  const programConfig = (await futarchy.pda.squadsProgramConfig()).address;
  const pcInfo = await f.harness.connection.getAccountInfo(programConfig);
  const treasury = new Address(pcInfo!.data.slice(48, 80));

  const boot = await futarchy.bootstrapGovernance({
    payer: f.payer.publicKey,
    daoCreator: f.payer.publicKey,
    kassMint: f.kassMint.publicKey,
    usdcMint: f.usdcMint.publicKey,
    squadsProgramConfigTreasury: treasury,
    nonce,
    admin: f.payer.publicKey,
    // TWAP params: observable immediately (start_delay 0); tiny windows so the
    // verdict arm can timeTravel past them.
    twapInitialObservation: 1_000_000_000_000n, // 1.0 (PRICE_SCALE 1e12) quote/base
    twapMaxObservationChangePerUpdate: 1_000_000_000_000n, // > 0 (invariant); allows big moves
    twapStartDelaySeconds: 0,
    // invariant: min_base/min_quote futarchic liquidity must be > 0.
    minQuoteFutarchicLiquidity: 1n,
    minBaseFutarchicLiquidity: 1n,
    baseToStake: 0n,
    passThresholdBps: 0,
    // DAO invariant: seconds_per_proposal >= 86400 (1 day) and >= 2×start_delay.
    secondsPerProposal: 86_400,
  });

  f.dao = boot.dao;
  f.multisig = boot.multisig;
  f.vault = boot.vault;

  // initialize_dao CPIs into Squads (multisig create) + creates ATAs → heavy.
  await sendIx(f, boot.instructions[0], [], 1_400_000);
  await sendIx(f, boot.instructions[1]);
}
