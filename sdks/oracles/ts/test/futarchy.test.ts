/**
 * G2 — futarchy v0.6 + Squads v4 + conditional_vault builder byte/meta tests.
 *
 * For each builder we assert `data == [disc, ...borsh_args]` (the expected buffer
 * built INDEPENDENTLY here) and the account-meta order/roles, with PDA-derived
 * slots cross-checked against the documented seeds (see src/futarchy/NOTES.md).
 * The discriminators + seeds are the binary-validated values from the Rust CPI
 * modules; the account orders + arg layouts are from the authoritative
 * metaDAOproject/futarchy@v0.6.0 + Squads-Protocol/v4 source.
 *
 * Shared fixtures/helpers live in ./helpers/futarchy.ts. The Squads-v4 builders,
 * bootstrapGovernance composer, and collect_meteora_damm_fees tests live in
 * futarchy-squads.test.ts.
 */
import { Address } from "@solana/web3.js";
import { describe, expect, it } from "vitest";

import { futarchy } from "../src/index.js";
import {
  ACCOUNT_DISC,
  ADMIN,
  ATA_PROGRAM_ID,
  CONDITIONAL_VAULT_ID,
  DAO_CREATOR,
  DISC,
  FUTARCHY_ID,
  KASS_MINT,
  Market,
  PAYER,
  SOME,
  SQUADS_V4_ID,
  SwapType,
  TREASURY,
  USDC_MINT,
  ata,
  cat,
  enc,
  hex,
  pda,
  u128,
  u16,
  u32,
  u64,
} from "./helpers/futarchy.js";

describe("futarchy/Squads wire constants", () => {
  it("pins the binary-validated discriminators", () => {
    expect(hex(DISC.initializeDao)).toBe("80e2605a273818c4");
    expect(hex(DISC.initializeProposal)).toBe("32499c628195159e");
    expect(hex(DISC.launchProposal)).toBe("10d3bd77f54800e5");
    expect(hex(DISC.finalizeProposal)).toBe("174433a76dadbba4");
    expect(hex(DISC.conditionalSwap)).toBe("c288dc59f2a9829d");
    expect(hex(DISC.spotSwap)).toBe("a7610ce7ed4ea6fb");
    expect(hex(DISC.initializeQuestion)).toBe("f5976abc582c41d4");
    expect(hex(DISC.initializeConditionalVault)).toBe("2558fad436dae3af");
    expect(hex(DISC.splitTokens)).toBe("4fc374008cb049b3");
    expect(hex(DISC.mergeTokens)).toBe("e259fb79e182b40e");
    expect(hex(DISC.redeemTokens)).toBe("f662862998217845");
    expect(hex(DISC.resolveQuestion)).toBe("3420e0b3b40800f6");
    expect(hex(DISC.multisigCreateV2)).toBe("32ddc75d28f58be9");
    expect(hex(DISC.vaultTransactionCreate)).toBe("30fa4ea8d0e2dad3");
    expect(hex(DISC.vaultTransactionExecute)).toBe("c208a15799a419ab");
    expect(hex(DISC.proposalCreate)).toBe("dc3c49e01e6c4f9f");
    expect(hex(ACCOUNT_DISC.dao)).toBe("a3092f1f3455c531");
    expect(hex(ACCOUNT_DISC.proposal)).toBe("1a5ebdbb74883521");
    // F2a: pinned from metaDAOproject/programs@c1000ed + the on-chain v0.6.1 IDL.
    expect(hex(DISC.collectMeteoraDammFees)).toBe("8bd469767e36d68f");
  });

  it("Market/SwapType Borsh tags", () => {
    expect(Market.Spot).toBe(0);
    expect(Market.Pass).toBe(1);
    expect(Market.Fail).toBe(2);
    expect(SwapType.Buy).toBe(0);
    expect(SwapType.Sell).toBe(1);
  });
});

describe("PDA derivers (CONFIRMED: multisig.create_key == Dao)", () => {
  it("squads multisig/vault derive from the Dao PDA per the documented seeds", async () => {
    const dao = (await pda.dao(DAO_CREATOR, 7n)).address;

    // independent: dao = [b"dao", creator, nonce_le] under FUTARCHY_ID
    const [daoIndep] = await Address.findProgramAddress(
      [enc.encode("dao"), new Address(DAO_CREATOR).toBytes(), u64(7n)],
      FUTARCHY_ID,
    );
    expect(dao.toString()).toBe(daoIndep.toString());

    const multisig = (await pda.squadsMultisig(dao)).address;
    const [msIndep] = await Address.findProgramAddress(
      [enc.encode("multisig"), enc.encode("multisig"), dao.toBytes()],
      SQUADS_V4_ID,
    );
    expect(multisig.toString()).toBe(msIndep.toString());

    const vault = (await pda.squadsVault(multisig, 0)).address;
    const [vIndep] = await Address.findProgramAddress(
      [enc.encode("multisig"), multisig.toBytes(), enc.encode("vault"), Uint8Array.from([0])],
      SQUADS_V4_ID,
    );
    expect(vault.toString()).toBe(vIndep.toString());
  });

  it("squads transaction/proposal/program_config/spending_limit seeds", async () => {
    const dao = (await pda.dao(DAO_CREATOR, 1n)).address;
    const multisig = (await pda.squadsMultisig(dao)).address;

    const [txIndep] = await Address.findProgramAddress(
      [enc.encode("multisig"), multisig.toBytes(), enc.encode("transaction"), u64(1n)],
      SQUADS_V4_ID,
    );
    expect((await pda.squadsTransaction(multisig, 1n)).address.toString()).toBe(txIndep.toString());

    const [propIndep] = await Address.findProgramAddress(
      [enc.encode("multisig"), multisig.toBytes(), enc.encode("transaction"), u64(1n), enc.encode("proposal")],
      SQUADS_V4_ID,
    );
    expect((await pda.squadsProposal(multisig, 1n)).address.toString()).toBe(propIndep.toString());

    const [pcIndep] = await Address.findProgramAddress(
      [enc.encode("multisig"), enc.encode("program_config")],
      SQUADS_V4_ID,
    );
    expect((await pda.squadsProgramConfig()).address.toString()).toBe(pcIndep.toString());

    const [slIndep] = await Address.findProgramAddress(
      [enc.encode("multisig"), multisig.toBytes(), enc.encode("spending_limit"), dao.toBytes()],
      SQUADS_V4_ID,
    );
    expect((await pda.squadsSpendingLimit(multisig, dao)).address.toString()).toBe(slIndep.toString());
  });
});

describe("conditional_vault builders", () => {
  it("initialize_question: disc ++ id[32] ++ oracle[32] ++ n:u8 + accounts", async () => {
    const id = new Uint8Array(32).fill(0x5a);
    const ix = await futarchy.initializeQuestion({ questionId: id, oracle: SOME, numOutcomes: 2, payer: PAYER });
    expect(hex(ix.data)).toBe(hex(cat(DISC.initializeQuestion, id, new Address(SOME).toBytes(), Uint8Array.from([2]))));

    const question = (await pda.question(id, SOME, 2)).address;
    const ea = (await pda.vaultEventAuthority()).address;
    expect(ix.programId.toString()).toBe(CONDITIONAL_VAULT_ID.toString());
    expect(ix.keys.map((k) => [k.pubkey.toString(), k.isSigner, k.isWritable])).toEqual([
      [question.toString(), false, true],
      [PAYER, true, true],
      ["11111111111111111111111111111111", false, false],
      [ea.toString(), false, false],
      [CONDITIONAL_VAULT_ID.toString(), false, false],
    ]);
  });

  it("split_tokens: disc ++ amount:u64 + InteractWithVault metas", async () => {
    const ix = await futarchy.splitTokens({
      question: SOME,
      vault: SOME,
      vaultUnderlying: SOME,
      authority: ADMIN,
      userUnderlying: SOME,
      conditionalMints: [SOME, USDC_MINT],
      userConditionalAccounts: [KASS_MINT, USDC_MINT],
      amount: 2_000_000_000n,
    });
    expect(hex(ix.data)).toBe(hex(cat(DISC.splitTokens, u64(2_000_000_000n))));
    // 8 fixed metas + 2 cond mints + 2 user cond accounts = 12
    expect(ix.keys.length).toBe(12);
    expect(ix.keys[3].isSigner).toBe(true); // authority
    expect(ix.keys[3].pubkey.toString()).toBe(ADMIN);
  });

  it("resolve_question: disc ++ Vec<u32>{2, n0, n1}", async () => {
    const ix = await futarchy.resolveQuestion({ question: SOME, oracle: ADMIN, payoutNumerators: [1, 0] });
    expect(hex(ix.data)).toBe(hex(cat(DISC.resolveQuestion, u32(2), u32(1), u32(0))));
    expect(ix.keys[1].isSigner).toBe(true);
  });
});

describe("futarchy builders", () => {
  it("initialize_dao: 117-byte data (v0.6.1 +team_* args) + the full 18-account order (event_cpi tail)", async () => {
    const args = {
      daoCreator: DAO_CREATOR,
      payer: PAYER,
      baseMint: KASS_MINT,
      quoteMint: USDC_MINT,
      squadsProgramConfigTreasury: TREASURY,
      twapInitialObservation: 500_000_000n,
      twapMaxObservationChangePerUpdate: 1_000_000_000n,
      twapStartDelaySeconds: 60,
      minQuoteFutarchicLiquidity: 1_000_000n,
      minBaseFutarchicLiquidity: 2_000_000n,
      baseToStake: 3_000_000n,
      passThresholdBps: 500,
      secondsPerProposal: 86_400,
      nonce: 9n,
    };
    const ix = await futarchy.initializeDao(args);

    const expected = cat(
      DISC.initializeDao,
      u128(500_000_000n),
      u128(1_000_000_000n),
      u32(60),
      u64(1_000_000n),
      u64(2_000_000n),
      u64(3_000_000n),
      u16(500),
      u32(86_400),
      u64(9n),
      Uint8Array.from([0]),
      Uint8Array.from([0, 0]), // v0.6.1 team_sponsored_pass_threshold_bps: i16 = 0
      new Uint8Array(32), // v0.6.1 team_address: Pubkey = zero/system key (default)
    );
    expect(ix.data.length).toBe(117);
    expect(hex(ix.data)).toBe(hex(expected));

    const dao = (await pda.dao(DAO_CREATOR, 9n)).address;
    const multisig = (await pda.squadsMultisig(dao)).address;
    const vault = (await pda.squadsVault(multisig, 0)).address;
    const pc = (await pda.squadsProgramConfig()).address;
    const sl = (await pda.squadsSpendingLimit(multisig, dao)).address;
    const baseVault = await ata(dao, KASS_MINT);
    const quoteVault = await ata(dao, USDC_MINT);
    const ea = (await pda.futarchyEventAuthority()).address;
    const SYS = "11111111111111111111111111111111";
    const TOKEN = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";

    expect(ix.programId.toString()).toBe(FUTARCHY_ID.toString());
    expect(ix.keys.map((k) => [k.pubkey.toString(), k.isSigner, k.isWritable])).toEqual([
      [dao.toString(), false, true],
      [DAO_CREATOR, true, false],
      [PAYER, true, true],
      [SYS, false, false],
      [KASS_MINT, false, false],
      [USDC_MINT, false, false],
      [multisig.toString(), false, true],
      [vault.toString(), false, false],
      [SQUADS_V4_ID.toString(), false, false],
      [pc.toString(), false, false],
      [TREASURY, false, true],
      [sl.toString(), false, true],
      [baseVault.toString(), false, true],
      [quoteVault.toString(), false, true],
      [TOKEN, false, false],
      [ATA_PROGRAM_ID.toString(), false, false],
      [ea.toString(), false, false],
      [FUTARCHY_ID.toString(), false, false],
    ]);
  });

  it("conditional_swap: disc ++ market ++ swap_type ++ in:u64 ++ minOut:u64", async () => {
    const common = {
      dao: SOME, ammBaseVault: SOME, ammQuoteVault: SOME, proposal: SOME,
      ammPassBaseVault: SOME, ammPassQuoteVault: SOME, ammFailBaseVault: SOME, ammFailQuoteVault: SOME,
      trader: ADMIN, userInputAccount: SOME, userOutputAccount: SOME,
      baseVault: SOME, baseVaultUnderlying: SOME, quoteVault: SOME, quoteVaultUnderlying: SOME,
      passBaseMint: SOME, failBaseMint: SOME, passQuoteMint: SOME, failQuoteMint: SOME, question: SOME,
    };
    const ix = await futarchy.conditionalSwap({
      ...common,
      market: Market.Pass,
      swapType: SwapType.Buy,
      inputAmount: 1_000n,
      minOutputAmount: 900n,
    });
    expect(hex(ix.data)).toBe(
      hex(cat(DISC.conditionalSwap, Uint8Array.from([1]), Uint8Array.from([0]), u64(1_000n), u64(900n))),
    );
    expect(ix.keys.length).toBe(25); // 23 declared + event_authority + program
    expect(ix.keys[8].isSigner).toBe(true); // trader
  });

  it("spot_swap: disc ++ in:u64 ++ swap_type ++ minOut:u64", async () => {
    const ix = await futarchy.spotSwap({
      dao: SOME, userBaseAccount: SOME, userQuoteAccount: SOME, ammBaseVault: SOME, ammQuoteVault: SOME,
      user: ADMIN, inputAmount: 50n, swapType: SwapType.Sell, minOutputAmount: 10n,
    });
    expect(hex(ix.data)).toBe(hex(cat(DISC.spotSwap, u64(50n), Uint8Array.from([1]), u64(10n))));
    expect(ix.keys.length).toBe(9);
    expect(ix.keys[5].isSigner).toBe(true); // user
  });

  it("initialize_proposal/finalize_proposal: disc-only data + event_cpi tail", async () => {
    const ip = await futarchy.initializeProposal({
      squadsProposal: SOME, squadsMultisig: SOME, dao: SOME, question: SOME, quoteVault: SOME, baseVault: SOME, proposer: ADMIN, payer: PAYER,
    });
    expect(hex(ip.data)).toBe(hex(DISC.initializeProposal));
    expect(ip.keys.length).toBe(12);

    const fp = await futarchy.finalizeProposal({
      proposal: SOME, dao: SOME, question: SOME, squadsProposal: SOME, squadsMultisig: SOME,
      ammPassBaseVault: SOME, ammPassQuoteVault: SOME, ammFailBaseVault: SOME, ammFailQuoteVault: SOME,
      ammBaseVault: SOME, ammQuoteVault: SOME, quoteVault: SOME, quoteVaultUnderlying: SOME,
      passQuoteMint: SOME, failQuoteMint: SOME, passBaseMint: SOME, failBaseMint: SOME,
      baseVault: SOME, baseVaultUnderlying: SOME,
    });
    expect(hex(fp.data)).toBe(hex(DISC.finalizeProposal));
    expect(fp.keys.length).toBe(25);
  });
});
