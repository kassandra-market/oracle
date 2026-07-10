/**
 * G2 — futarchy v0.6 + Squads v4 + conditional_vault builder byte/meta tests.
 *
 * For each builder we assert `data == [disc, ...borsh_args]` (the expected buffer
 * built INDEPENDENTLY here) and the account-meta order/roles, with PDA-derived
 * slots cross-checked against the documented seeds (see src/futarchy/NOTES.md).
 * The discriminators + seeds are the binary-validated values from the Rust CPI
 * modules; the account orders + arg layouts are from the authoritative
 * metaDAOproject/futarchy@v0.6.0 + Squads-Protocol/v4 source.
 */
import { Address } from "@solana/web3.js";
import { describe, expect, it } from "vitest";

import { futarchy } from "../src/index.js";
import { setGovernance } from "../src/instructions/index.js";

const {
  DISC,
  ACCOUNT_DISC,
  Market,
  SwapType,
  FUTARCHY_ID,
  CONDITIONAL_VAULT_ID,
  SQUADS_V4_ID,
  SQUADS_PERMISSIONLESS_MEMBER,
  METADAO_ADMIN,
  METADAO_MULTISIG_VAULT,
  METEORA_DAMM_V2_ID,
  DAMM_V2_POOL_AUTHORITY,
  ATA_PROGRAM_ID,
  collectMeteoraDammFees,
  pda,
} = futarchy;

const SYSTEM_ID = "11111111111111111111111111111111";
const TOKEN_ID = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";

// Deterministic valid base58 stand-ins.
const PAYER = "rqRMW2HFJsi1FE1jb8Rvaz4Qz3xHzNkZDb8am1pqEHE";
const DAO_CREATOR = "84yVtdReAJ8GiR7Erqj7jyxoJurYWzQ6n9eaBGYBDNqM";
const KASS_MINT = "So11111111111111111111111111111111111111112";
const USDC_MINT = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
const TREASURY = "7bQEwuq9ybNyjjFcbtHBfDPxdH3TuGAsZKVRZdihVN4d";
const ADMIN = "7WCvk98KGRqi2o8D7EWTGrZQuFtikidP8A2D7CDVXwWJ";
const SOME = "GuBhyNi5GFo9K5YXGKfPMDryWK8GwS5oXe9CJGrzo2sk";

const enc = new TextEncoder();
const hex = (b: Uint8Array) => Buffer.from(b).toString("hex");
const u64 = (v: bigint) => {
  const o = new Uint8Array(8);
  new DataView(o.buffer).setBigUint64(0, v, true);
  return o;
};
const u32 = (v: number) => {
  const o = new Uint8Array(4);
  new DataView(o.buffer).setUint32(0, v, true);
  return o;
};
const u16 = (v: number) => {
  const o = new Uint8Array(2);
  new DataView(o.buffer).setUint16(0, v, true);
  return o;
};
const u128 = (v: bigint) => {
  const o = new Uint8Array(16);
  const dv = new DataView(o.buffer);
  dv.setBigUint64(0, v & 0xffffffffffffffffn, true);
  dv.setBigUint64(8, v >> 64n, true);
  return o;
};
const cat = (...ps: Uint8Array[]) => {
  const out = new Uint8Array(ps.reduce((n, p) => n + p.length, 0));
  let o = 0;
  for (const p of ps) {
    out.set(p, o);
    o += p.length;
  }
  return out;
};
const ata = async (owner: string | Address, mint: string | Address) =>
  (
    await Address.findProgramAddress(
      [new Address(owner as string).toBytes(), new Address("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA").toBytes(), new Address(mint as string).toBytes()],
      ATA_PROGRAM_ID,
    )
  )[0];

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

describe("Squads v4 builders", () => {
  it("vault_transaction_create: disc ++ vault_idx ++ eph ++ Vec<u8>(msg) ++ None memo", async () => {
    const msg = new Uint8Array([1, 2, 3, 4]);
    const dao = (await pda.dao(DAO_CREATOR, 1n)).address;
    const multisig = (await pda.squadsMultisig(dao)).address;
    const ix = await futarchy.vaultTransactionCreate({
      multisig, creator: dao, rentPayer: PAYER, transactionIndex: 1n, transactionMessage: msg,
    });
    expect(hex(ix.data)).toBe(
      hex(cat(DISC.vaultTransactionCreate, Uint8Array.from([0, 0]), u32(4), msg, Uint8Array.from([0]))),
    );
    const tx = (await pda.squadsTransaction(multisig, 1n)).address;
    expect(ix.keys[1].pubkey.toString()).toBe(tx.toString());
    expect(ix.keys[1].isWritable).toBe(true);
  });

  it("proposal_create: disc ++ index:u64 ++ draft:bool", async () => {
    const dao = (await pda.dao(DAO_CREATOR, 1n)).address;
    const multisig = (await pda.squadsMultisig(dao)).address;
    const ix = await futarchy.proposalCreate({ multisig, creator: dao, rentPayer: PAYER, transactionIndex: 1n });
    expect(hex(ix.data)).toBe(hex(cat(DISC.proposalCreate, u64(1n), Uint8Array.from([0]))));
  });

  it("vault_transaction_execute: disc-only, fixed metas + member signer", async () => {
    const dao = (await pda.dao(DAO_CREATOR, 1n)).address;
    const multisig = (await pda.squadsMultisig(dao)).address;
    const ix = await futarchy.vaultTransactionExecute({ multisig, transactionIndex: 1n, member: dao });
    expect(hex(ix.data)).toBe(hex(DISC.vaultTransactionExecute));
    const prop = (await pda.squadsProposal(multisig, 1n)).address;
    const tx = (await pda.squadsTransaction(multisig, 1n)).address;
    expect(ix.keys.map((k) => [k.pubkey.toString(), k.isSigner, k.isWritable])).toEqual([
      [multisig.toString(), false, false],
      [prop.toString(), false, true],
      [tx.toString(), false, false],
      [dao.toString(), true, false],
    ]);
  });
});

describe("bootstrapGovernance composer", () => {
  it("composes [initialize_dao, set_governance] with vault==dao_authority, kass_dao==dao", async () => {
    const r = await futarchy.bootstrapGovernance({
      payer: PAYER,
      daoCreator: DAO_CREATOR,
      kassMint: KASS_MINT,
      usdcMint: USDC_MINT,
      squadsProgramConfigTreasury: TREASURY,
      nonce: 42n,
      twapInitialObservation: 500_000_000n,
      twapMaxObservationChangePerUpdate: 1_000_000_000n,
      twapStartDelaySeconds: 60,
      minQuoteFutarchicLiquidity: 1_000_000n,
      minBaseFutarchicLiquidity: 1_000_000n,
      baseToStake: 1_000_000n,
      passThresholdBps: 300,
      secondsPerProposal: 86_400,
      admin: ADMIN,
    });

    const dao = (await pda.dao(DAO_CREATOR, 42n)).address;
    const multisig = (await pda.squadsMultisig(dao)).address;
    const vault = (await pda.squadsVault(multisig, 0)).address;

    expect(r.dao.toString()).toBe(dao.toString());
    expect(r.multisig.toString()).toBe(multisig.toString());
    expect(r.vault.toString()).toBe(vault.toString());
    expect(r.instructions.length).toBe(2);

    // ix[0] = initialize_dao on the futarchy program, dao slot 0.
    expect(r.instructions[0].programId.toString()).toBe(FUTARCHY_ID.toString());
    expect(r.instructions[0].keys[0].pubkey.toString()).toBe(dao.toString());

    // ix[1] = set_governance whose payload dao_authority==vault, kass_dao==dao.
    const handoff = await setGovernance({ authority: ADMIN, daoAuthority: vault, kassDao: dao });
    expect(hex(r.instructions[1].data)).toBe(hex(handoff.data));
    // payload = [disc, dao_authority[32], kass_dao[32]]
    expect(hex(r.instructions[1].data.slice(1, 33))).toBe(hex(vault.toBytes()));
    expect(hex(r.instructions[1].data.slice(33, 65))).toBe(hex(dao.toBytes()));
    expect(r.instructions[1].keys[2].pubkey.toString()).toBe(dao.toString()); // kass_dao account
  });

  it("the permissionless multisig member id is pinned", () => {
    expect(SQUADS_PERMISSIONLESS_MEMBER.toString()).toBe("EP3SoC2SvR3d4c2eXVBvhEMWSr2j3YtoCY3UMiQV7BPD");
  });
});

// F2a — collect_meteora_damm_fees. Wire format PINNED from TWO agreeing sources:
// (a) metaDAOproject/programs@c1000ed84ef6d084203ad2a9c13940fd14feb53c
//     programs/futarchy/src/instructions/collect_meteora_damm_fees.rs + lib.rs:158
//     (declare_id == FUTAREL…, Cargo.toml v0.6.1), and
// (b) the on-chain Anchor IDL of FUTAREL… (v0.6.1), instruction collectMeteoraDammFees.
// Both give the SAME 27 accounts (incl. the #[event_cpi] tail) and NO args.
describe("collect_meteora_damm_fees (F2a — pinned v0.6.1 wire format)", () => {
  // Meteora-side stand-ins (deterministic valid base58).
  const POOL = "Az8Fho8xdVcxX9qoUqfW2rcau84VCBbTPjYtdaBZS9te";
  const POSITION = "CVtSMpnbcnLiFvT4b5KqczNw4kT8iYxMDvxu5Ef6VWb1";
  const TOKEN_A_VAULT = "A9kSdAC4B5wgsc6t4ZgekoFeoE4rci6h2q3L84SmrqYP";
  const TOKEN_B_VAULT = "32aPQRSwF6vTRWdmxzEUqqdD32s4bLZqAiz2nJfy9eAK";
  const POSITION_NFT_ACCOUNT = "33Bxc7zrtjhXHwXSinSFDvwLfniH22dpE9LRRoQdtoBm";
  const OWNER = "MtLmU4aQUHGE5PPt37fJhoUth6RWAr35aUHbnwtiPC3";

  it("data == disc only (no args)", async () => {
    const ix = await collectMeteoraDammFees({
      dao: (await pda.dao(DAO_CREATOR, 9n)).address,
      transactionIndex: 3n,
      pool: POOL,
      position: POSITION,
      tokenAVault: TOKEN_A_VAULT,
      tokenBVault: TOKEN_B_VAULT,
      tokenAMint: KASS_MINT,
      tokenBMint: USDC_MINT,
      positionNftAccount: POSITION_NFT_ACCOUNT,
      owner: OWNER,
    });
    expect(ix.programId.toString()).toBe(FUTARCHY_ID.toString());
    expect(hex(ix.data)).toBe(hex(DISC.collectMeteoraDammFees));
    expect(ix.data.length).toBe(8);
  });

  it("account metas match the IDL order + roles + PDAs", async () => {
    const dao = (await pda.dao(DAO_CREATOR, 9n)).address;
    const multisig = (await pda.squadsMultisig(dao)).address;
    const squadsVault = (await pda.squadsVault(multisig, 0)).address;
    const txIndex = 3n;
    const squadsTx = (await pda.squadsTransaction(multisig, txIndex)).address;
    const squadsProp = (await pda.squadsProposal(multisig, txIndex)).address;
    const futEventAuth = (await pda.futarchyEventAuthority()).address;
    const [dammEventAuth] = await Address.findProgramAddress(
      [enc.encode("__event_authority")],
      METEORA_DAMM_V2_ID,
    );
    const tokenAAccount = await ata(METADAO_MULTISIG_VAULT, KASS_MINT);
    const tokenBAccount = await ata(METADAO_MULTISIG_VAULT, USDC_MINT);

    const ix = await collectMeteoraDammFees({
      dao,
      transactionIndex: txIndex,
      pool: POOL,
      position: POSITION,
      tokenAVault: TOKEN_A_VAULT,
      tokenBVault: TOKEN_B_VAULT,
      tokenAMint: KASS_MINT,
      tokenBMint: USDC_MINT,
      positionNftAccount: POSITION_NFT_ACCOUNT,
      owner: OWNER,
    });

    // [pubkey, isSigner, isWritable] in EXACT IDL order (27 accounts).
    const expected: Array<[string, boolean, boolean]> = [
      [dao.toString(), false, true],
      [METADAO_ADMIN.toString(), true, true],
      [multisig.toString(), false, true],
      [squadsVault.toString(), false, true],
      [squadsTx.toString(), false, true],
      [squadsProp.toString(), false, true],
      [SQUADS_PERMISSIONLESS_MEMBER.toString(), true, false],
      [METEORA_DAMM_V2_ID.toString(), false, false],
      [dammEventAuth.toString(), false, false],
      [DAMM_V2_POOL_AUTHORITY.toString(), false, false],
      [POOL, false, false],
      [POSITION, false, true],
      [tokenAAccount.toString(), false, true],
      [tokenBAccount.toString(), false, true],
      [TOKEN_A_VAULT, false, true],
      [TOKEN_B_VAULT, false, true],
      [KASS_MINT, false, false],
      [USDC_MINT, false, false],
      [POSITION_NFT_ACCOUNT, false, false],
      [OWNER, false, false],
      [TOKEN_ID, false, false],
      [TOKEN_ID, false, false],
      [SYSTEM_ID, false, false],
      [TOKEN_ID, false, false],
      [SQUADS_V4_ID.toString(), false, false],
      [futEventAuth.toString(), false, false],
      [FUTARCHY_ID.toString(), false, false],
    ];

    expect(ix.keys.length).toBe(27);
    ix.keys.forEach((k, i) => {
      expect([k.pubkey.toString(), k.isSigner, k.isWritable]).toEqual(expected[i]);
    });
  });

  it("the hard-coded pool_authority equals the derived cp-amm PDA", async () => {
    const [derived] = await Address.findProgramAddress(
      [enc.encode("pool_authority")],
      METEORA_DAMM_V2_ID,
    );
    expect(derived.toString()).toBe(DAMM_V2_POOL_AUTHORITY.toString());
    expect(DAMM_V2_POOL_AUTHORITY.toString()).toBe("HLnpSz9h2S4hiLQ43rnSD9XkcUThA7B8hQMKmDaiTLcC");
  });
});
