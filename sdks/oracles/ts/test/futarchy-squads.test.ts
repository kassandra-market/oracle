/**
 * G2 (cont.) — Squads-v4 builders, the bootstrapGovernance composer, and the
 * F2a collect_meteora_damm_fees wire-format tests. Split out of futarchy.test.ts;
 * shared fixtures/helpers live in ./helpers/futarchy.ts.
 */
import { Address } from "@solana/web3.js";
import { describe, expect, it } from "vitest";

import { futarchy } from "../src/index.js";
import { setGovernance } from "../src/instructions/index.js";
import {
  ADMIN,
  DAMM_V2_POOL_AUTHORITY,
  DAO_CREATOR,
  DISC,
  FUTARCHY_ID,
  KASS_MINT,
  METADAO_ADMIN,
  METADAO_MULTISIG_VAULT,
  METEORA_DAMM_V2_ID,
  PAYER,
  SQUADS_PERMISSIONLESS_MEMBER,
  SQUADS_V4_ID,
  SYSTEM_ID,
  TOKEN_ID,
  TREASURY,
  USDC_MINT,
  ata,
  cat,
  collectMeteoraDammFees,
  enc,
  hex,
  pda,
  u32,
  u64,
} from "./helpers/futarchy.js";

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
