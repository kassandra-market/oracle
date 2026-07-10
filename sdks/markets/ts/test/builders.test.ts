/**
 * Builder + decoder unit tests (no chain).
 *
 * For each instruction builder we assert: the discriminant byte, the byte-exact
 * payload (for the fixed-layout ones), the account COUNT + each meta's
 * isSigner/isWritable role, and that internally-derived-PDA slots carry the
 * `pda.*` address. The expectations are hardcoded copies of `sdks/oracles/rust/src/ix.rs`
 * (the verified Rust builders) — a drift there fails here.
 *
 * The decoder tests build zeroed Config/Market/Contribution buffers with the tag
 * + a couple of fields set at the pinned offsets and assert the decoder reads
 * them back, plus that `assertAccount` rejects a wrong size/tag.
 */
import { Address } from "@solana/web3.js";
import type { AccountMeta } from "@solana/web3.js";
import { describe, expect, it } from "vitest";

import {
  AccountType,
  ACCOUNT_SIZES,
  EXTERNAL_PROGRAM_IDS,
  Ix,
  MarketStatus,
  SYSTEM_PROGRAM_ID,
  TOKEN_PROGRAM_ID,
} from "../src/constants.js";
import * as pda from "../src/pda.js";
import {
  activate,
  cancel,
  claimLp,
  closeMarket,
  collectFee,
  contribute,
  createMarket,
  initConfig,
  refund,
  resolveMarket,
  updateConfig,
} from "../src/instructions/market.js";
import { decodeConfig } from "../src/accounts/config.js";
import { decodeContribution } from "../src/accounts/contribution.js";
import { decodeMarket } from "../src/accounts/market.js";
import { assertAccount } from "../src/accounts/common.js";

// Deterministic non-PDA test addresses.
const A = (n: number): Address =>
  new Address(new Uint8Array(32).fill(n));
const PAYER = A(1);
const KASS_MINT = A(2);
const AUTHORITY = A(3);
const ORACLE = A(4);
const CREATOR = A(5);
const CREATOR_ATA = A(6);
const CONTRIBUTOR = A(7);
const CONTRIB_ATA = A(8);
const LP_ATA = A(9);
const QUESTION = A(10);
const FEE_DEST = A(11);

/** Compact "S"/"W"/"w"/"r" flag string per key, in order. */
function flags(keys: ReadonlyArray<AccountMeta>): string {
  return keys
    .map((k) => (k.isSigner ? (k.isWritable ? "S" : "s") : k.isWritable ? "W" : "r"))
    .join("");
}
const b58 = (a: Address): string => a.toString();
const addrsOf = (keys: ReadonlyArray<AccountMeta>): string[] => keys.map((k) => b58(k.pubkey));

describe("initConfig (Ix 0)", () => {
  it("disc, payload = authority(32) ++ u64(minLiquidity) ++ u16(feeBps) ++ feeDestination(32), accounts", async () => {
    const ix = await initConfig({
      payer: PAYER,
      kassMint: KASS_MINT,
      authority: AUTHORITY,
      minLiquidity: 5n,
      feeBps: 250,
      feeDestination: FEE_DEST,
    });
    expect(ix.data[0]).toBe(Ix.InitConfig);
    // payload: authority 32 ++ u64 LE minLiquidity ++ u16 LE feeBps ++ feeDestination 32
    const expected = new Uint8Array([
      ...AUTHORITY.toBytes(),
      5, 0, 0, 0, 0, 0, 0, 0,
      0xfa, 0x00, // 250 LE
      ...FEE_DEST.toBytes(),
    ]);
    expect(ix.data.slice(1)).toEqual(expected);
    expect(ix.data.length).toBe(1 + 32 + 8 + 2 + 32);

    const config = await pda.config();
    const programData = await pda.programData();
    expect(ix.keys.length).toBe(6);
    expect(flags(ix.keys)).toBe("WSrrrr");
    expect(addrsOf(ix.keys)).toEqual([
      b58(config.address),
      b58(PAYER),
      b58(KASS_MINT),
      b58(FEE_DEST),
      b58(SYSTEM_PROGRAM_ID),
      b58(programData.address),
    ]);
  });
});

describe("updateConfig (Ix 1)", () => {
  it("disc, payload = u64(minLiquidity) ++ u16(feeBps) ++ feeDestination(32), accounts [config(w), authority(ro,signer), feeDestination(ro)]", async () => {
    const ix = await updateConfig({
      authority: AUTHORITY,
      minLiquidity: 42n,
      feeBps: 300,
      feeDestination: FEE_DEST,
    });
    expect(ix.data[0]).toBe(Ix.UpdateConfig);
    expect(ix.data.slice(1)).toEqual(
      new Uint8Array([42, 0, 0, 0, 0, 0, 0, 0, 0x2c, 0x01, ...FEE_DEST.toBytes()]),
    );
    const config = await pda.config();
    expect(ix.keys.length).toBe(3);
    expect(flags(ix.keys)).toBe("Wsr");
    expect(addrsOf(ix.keys)).toEqual([b58(config.address), b58(AUTHORITY), b58(FEE_DEST)]);
  });
});

describe("createMarket (Ix 2)", () => {
  it("disc, payload = u64(seedAmount) ++ u8(outcomeIndex), 10 accounts w/ derived PDAs", async () => {
    const outcomeIndex = 2;
    const ix = await createMarket({
      creator: CREATOR,
      oracle: ORACLE,
      kassMint: KASS_MINT,
      creatorKassAta: CREATOR_ATA,
      seedAmount: 1000n,
      outcomeIndex,
    });
    expect(ix.data[0]).toBe(Ix.CreateMarket);
    // 1000 = 0x3E8 -> LE 8 bytes; outcome_index u8 @8.
    // Payload is 9 bytes: seedAmount(8) ++ outcome_index(1).
    expect(ix.data.slice(1)).toEqual(
      new Uint8Array([0xe8, 0x03, 0, 0, 0, 0, 0, 0, 0x02]),
    );
    expect(ix.data.length).toBe(1 + 8 + 1);
    expect(ix.data[9]).toBe(outcomeIndex); // outcome_index @8 within payload (byte 9 incl. disc)

    const config = await pda.config();
    const market = await pda.market(ORACLE, outcomeIndex);
    const escrow = await pda.escrow(market.address);
    const contribution = await pda.contribution(market.address, CREATOR);

    // The market PDA is keyed by the outcome: a different index → a different PDA.
    const marketOutcome0 = await pda.market(ORACLE, 0);
    expect(market.address.toString()).not.toBe(marketOutcome0.address.toString());

    expect(ix.keys.length).toBe(10);
    expect(flags(ix.keys)).toBe("rrWWrSWWrr");
    expect(addrsOf(ix.keys)).toEqual([
      b58(config.address),
      b58(ORACLE),
      b58(market.address),
      b58(escrow.address),
      b58(KASS_MINT),
      b58(CREATOR),
      b58(CREATOR_ATA),
      b58(contribution.address),
      b58(TOKEN_PROGRAM_ID),
      b58(SYSTEM_PROGRAM_ID),
    ]);
  });
});

describe("contribute (Ix 3)", () => {
  it("disc, payload = u64(amount), 7 accounts incl. system program, derived escrow+contribution", async () => {
    const market = await pda.market(ORACLE, 0);
    const ix = await contribute({
      contributor: CONTRIBUTOR,
      market: market.address,
      contributorKassAta: CONTRIB_ATA,
      amount: 777n,
    });
    expect(ix.data[0]).toBe(Ix.Contribute);
    expect(ix.data.slice(1)).toEqual(new Uint8Array([0x09, 0x03, 0, 0, 0, 0, 0, 0]));

    const escrow = await pda.escrow(market.address);
    const contribution = await pda.contribution(market.address, CONTRIBUTOR);
    expect(ix.keys.length).toBe(7);
    expect(flags(ix.keys)).toBe("WWSWWrr");
    expect(addrsOf(ix.keys)).toEqual([
      b58(market.address),
      b58(escrow.address),
      b58(CONTRIBUTOR),
      b58(CONTRIB_ATA),
      b58(contribution.address),
      b58(TOKEN_PROGRAM_ID),
      b58(SYSTEM_PROGRAM_ID),
    ]);
  });
});

describe("cancel (Ix 4)", () => {
  it("empty payload, accounts [market(w), oracle(ro)]", async () => {
    const market = await pda.market(ORACLE, 0);
    const ix = await cancel({ market: market.address, oracle: ORACLE });
    expect(ix.data).toEqual(new Uint8Array([Ix.Cancel]));
    expect(ix.keys.length).toBe(2);
    expect(flags(ix.keys)).toBe("Wr");
    expect(addrsOf(ix.keys)).toEqual([b58(market.address), b58(ORACLE)]);
  });
});

describe("refund (Ix 5)", () => {
  it("empty payload, accounts [market(w), escrow(w), contribution(w), ata(w), contributor(w), token(ro)]", async () => {
    const market = await pda.market(ORACLE, 0);
    const ix = await refund({ market: market.address, contributor: CONTRIBUTOR, contributorKassAta: CONTRIB_ATA });
    expect(ix.data).toEqual(new Uint8Array([Ix.Refund]));
    const escrow = await pda.escrow(market.address);
    const contribution = await pda.contribution(market.address, CONTRIBUTOR);
    expect(ix.keys.length).toBe(6);
    expect(flags(ix.keys)).toBe("WWWWWr");
    expect(addrsOf(ix.keys)).toEqual([
      b58(market.address),
      b58(escrow.address),
      b58(contribution.address),
      b58(CONTRIB_ATA),
      b58(CONTRIBUTOR),
      b58(TOKEN_PROGRAM_ID),
    ]);
  });
});

describe("activate (Ix 6)", () => {
  it("empty payload, 22 accounts matching ix.rs order + flags", async () => {
    const market = await pda.market(ORACLE, 0);
    const md = (n: number) => A(100 + n);
    const ix = await activate({
      market: market.address,
      oracle: ORACLE,
      payer: PAYER,
      question: md(3),
      vault: md(4),
      vaultUnderlyingAta: md(5),
      yesMint: md(7),
      noMint: md(8),
      marketCyes: md(9),
      marketCno: md(10),
      amm: md(11),
      lpMint: md(12),
      lpVault: md(13),
      ammVaultBase: md(14),
      ammVaultQuote: md(15),
      cvEventAuthority: md(16),
      ammEventAuthority: md(18),
    });
    expect(ix.data).toEqual(new Uint8Array([Ix.Activate]));
    const escrow = await pda.escrow(market.address);
    expect(ix.keys.length).toBe(22);
    // 0 market(w) 1 oracle(ro) 2 payer(S) 3 question(ro) 4 vault(w) 5 vaultAta(w)
    // 6 escrow(w) 7 yes(w) 8 no(w) 9 cyes(w) 10 cno(w) 11 amm(w) 12 lpmint(w)
    // 13 lpvault(w) 14 ammBase(w) 15 ammQuote(w) 16 cvEA(ro) 17 cvProg(ro)
    // 18 ammEA(ro) 19 ammProg(ro) 20 token(ro) 21 system(ro)
    expect(flags(ix.keys)).toBe("WrSrWWWWWWWWWWWWrrrrrr");
    expect(b58(ix.keys[6]!.pubkey)).toBe(b58(escrow.address));
    expect(b58(ix.keys[17]!.pubkey)).toBe(b58(EXTERNAL_PROGRAM_IDS.conditionalVault));
    expect(b58(ix.keys[19]!.pubkey)).toBe(b58(EXTERNAL_PROGRAM_IDS.ammV04));
    expect(b58(ix.keys[20]!.pubkey)).toBe(b58(TOKEN_PROGRAM_ID));
    expect(b58(ix.keys[21]!.pubkey)).toBe(b58(SYSTEM_PROGRAM_ID));
  });
});

describe("claimLp (Ix 7)", () => {
  it("empty payload, accounts [market(w), lpVault(w), contribution(w), lpAta(w), contributor(w), token(ro)]", async () => {
    const market = await pda.market(ORACLE, 0);
    const ix = await claimLp({ market: market.address, contributor: CONTRIBUTOR, contributorLpAta: LP_ATA });
    expect(ix.data).toEqual(new Uint8Array([Ix.ClaimLp]));
    const lpVault = await pda.lpVault(market.address);
    const contribution = await pda.contribution(market.address, CONTRIBUTOR);
    expect(ix.keys.length).toBe(6);
    expect(flags(ix.keys)).toBe("WWWWWr");
    expect(addrsOf(ix.keys)).toEqual([
      b58(market.address),
      b58(lpVault.address),
      b58(contribution.address),
      b58(LP_ATA),
      b58(CONTRIBUTOR),
      b58(TOKEN_PROGRAM_ID),
    ]);
  });
});

describe("resolveMarket (Ix 8)", () => {
  it("empty payload, accounts [market(w), oracle(ro), question(w), cvEA(ro), cvProg(ro)]", async () => {
    const market = await pda.market(ORACLE, 0);
    const cvEA = A(50);
    const ix = await resolveMarket({
      market: market.address,
      oracle: ORACLE,
      question: QUESTION,
      cvEventAuthority: cvEA,
    });
    expect(ix.data).toEqual(new Uint8Array([Ix.ResolveMarket]));
    expect(ix.keys.length).toBe(5);
    // [market(w), oracle(ro), question(w), cvEA(ro), cvProg(ro)]
    expect(flags(ix.keys)).toBe("WrWrr");
    expect(addrsOf(ix.keys)).toEqual([
      b58(market.address),
      b58(ORACLE),
      b58(QUESTION),
      b58(cvEA),
      b58(EXTERNAL_PROGRAM_IDS.conditionalVault),
    ]);
  });
});

describe("collectFee (Ix 9)", () => {
  it("empty payload, 21 accounts matching ix.rs order + flags", async () => {
    const market = await pda.market(ORACLE, 0);
    const md = (n: number) => A(100 + n);
    const ix = await collectFee({
      market: market.address,
      feeDestination: FEE_DEST,
      question: QUESTION,
      vault: md(4),
      vaultUnderlyingAta: md(5),
      yesMint: md(7),
      noMint: md(8),
      marketCyes: md(9),
      marketCno: md(10),
      amm: md(11),
      lpMint: md(12),
      lpVault: md(13),
      ammVaultBase: md(14),
      ammVaultQuote: md(15),
      cvEventAuthority: md(16),
      ammEventAuthority: md(18),
    });
    expect(ix.data).toEqual(new Uint8Array([Ix.CollectFee]));
    const config = await pda.config();
    const escrow = await pda.escrow(market.address);
    expect(ix.keys.length).toBe(21);
    // 0 market(w) 1 config(ro) 2 feeDest(w) 3 question(ro) 4 vault(w) 5 vaultAta(w)
    // 6 escrow(w) 7 yes(w) 8 no(w) 9 cyes(w) 10 cno(w) 11 amm(w) 12 lpmint(w)
    // 13 lpvault(w) 14 ammBase(w) 15 ammQuote(w) 16 cvEA(ro) 17 cvProg(ro)
    // 18 ammEA(ro) 19 ammProg(ro) 20 token(ro)
    expect(flags(ix.keys)).toBe("WrWrWWWWWWWWWWWWrrrrr");
    expect(addrsOf(ix.keys)).toEqual([
      b58(market.address),
      b58(config.address),
      b58(FEE_DEST),
      b58(QUESTION),
      b58(md(4)),
      b58(md(5)),
      b58(escrow.address),
      b58(md(7)),
      b58(md(8)),
      b58(md(9)),
      b58(md(10)),
      b58(md(11)),
      b58(md(12)),
      b58(md(13)),
      b58(md(14)),
      b58(md(15)),
      b58(md(16)),
      b58(EXTERNAL_PROGRAM_IDS.conditionalVault),
      b58(md(18)),
      b58(EXTERNAL_PROGRAM_IDS.ammV04),
      b58(TOKEN_PROGRAM_ID),
    ]);
  });
});

describe("closeMarket (Ix 10)", () => {
  it("empty payload, accounts [market(w), creator(w), escrow(w), cyes(w), cno(w), lpVault(w), token(ro)]", async () => {
    const market = await pda.market(ORACLE, 0);
    const ix = await closeMarket({ market: market.address, creator: CREATOR });
    expect(ix.data).toEqual(new Uint8Array([Ix.CloseMarket]));
    const escrow = await pda.escrow(market.address);
    const cyes = await pda.cyes(market.address);
    const cno = await pda.cno(market.address);
    const lpVault = await pda.lpVault(market.address);
    expect(ix.keys.length).toBe(7);
    expect(flags(ix.keys)).toBe("WWWWWWr");
    expect(addrsOf(ix.keys)).toEqual([
      b58(market.address),
      b58(CREATOR),
      b58(escrow.address),
      b58(cyes.address),
      b58(cno.address),
      b58(lpVault.address),
      b58(TOKEN_PROGRAM_ID),
    ]);
  });
});

// ---------------------------------------------------------------------------
// Decoders
// ---------------------------------------------------------------------------

/** A small fixed-size buffer writer mirroring the on-chain Pod layout. */
class Buf {
  readonly bytes: Uint8Array;
  private readonly dv: DataView;
  constructor(size: number, accountType: AccountType) {
    this.bytes = new Uint8Array(size);
    this.dv = new DataView(this.bytes.buffer);
    this.bytes[0] = accountType; // account_type @0 (+ _pad_hdr[7])
  }
  u8(off: number, v: number): this {
    this.dv.setUint8(off, v);
    return this;
  }
  u16(off: number, v: number): this {
    this.dv.setUint16(off, v, true);
    return this;
  }
  u64(off: number, v: bigint): this {
    this.dv.setBigUint64(off, v, true);
    return this;
  }
  key(off: number, a: Address): this {
    this.bytes.set(a.toBytes(), off);
    return this;
  }
}

describe("decoders", () => {
  it("assertAccount rejects wrong size and wrong tag", () => {
    expect(() => assertAccount(new Uint8Array(87), AccountType.Config, ACCOUNT_SIZES.Config, "Config"))
      .toThrow(/wrong account size/);
    const wrongTag = new Uint8Array(ACCOUNT_SIZES.Config);
    wrongTag[0] = AccountType.Market;
    expect(() => assertAccount(wrongTag, AccountType.Config, ACCOUNT_SIZES.Config, "Config"))
      .toThrow(/wrong account_type/);
  });

  it("decodeConfig reads authority@8, kassMint@40, minLiquidity@72, bump@80, feeBps@82, feeDestination@84", () => {
    const buf = new Buf(ACCOUNT_SIZES.Config, AccountType.Config)
      .key(8, AUTHORITY)
      .key(40, KASS_MINT)
      .u64(72, 123456789n)
      .u8(80, 254)
      .u16(82, 250)
      .key(84, FEE_DEST);
    const c = decodeConfig(buf.bytes);
    expect(c.authority.toString()).toBe(AUTHORITY.toString());
    expect(c.kassMint.toString()).toBe(KASS_MINT.toString());
    expect(c.minLiquidity).toBe(123456789n);
    expect(c.bump).toBe(254);
    expect(c.feeBps).toBe(250);
    expect(c.feeDestination.toString()).toBe(FEE_DEST.toString());
  });

  it("decodeMarket reads oracle@8, openContributions@152, status@154, lpTotal@384, settled@392, feeBps@394, feeCollected@396, outcomeIndex@397", () => {
    const buf = new Buf(ACCOUNT_SIZES.Market, AccountType.Market)
      .key(8, ORACLE)
      .key(40, CREATOR)
      .u64(144, 9000n) // totalContributed
      .u16(152, 3) // openContributions
      .u8(154, MarketStatus.Active)
      .u8(155, 200) // bump
      .key(224, A(77)) // yesMint
      .u64(384, 555n) // lpTotal
      .u8(392, 1) // settled
      .u16(394, 100) // feeBps
      .u8(396, 1) // feeCollected
      .u8(397, 3); // outcomeIndex
    const m = decodeMarket(buf.bytes);
    expect(m.oracle.toString()).toBe(ORACLE.toString());
    expect(m.creator.toString()).toBe(CREATOR.toString());
    expect(m.totalContributed).toBe(9000n);
    expect(m.openContributions).toBe(3);
    expect(m.status).toBe(MarketStatus.Active);
    expect(m.bump).toBe(200);
    expect(m.yesMint.toString()).toBe(A(77).toString());
    expect(m.lpTotal).toBe(555n);
    expect(m.settled).toBe(true);
    expect(m.feeBps).toBe(100);
    expect(m.feeCollected).toBe(true);
    expect(m.outcomeIndex).toBe(3);
  });

  it("decodeContribution reads market@8, contributor@40, amount@72, claimed@80, bump@81", () => {
    const buf = new Buf(ACCOUNT_SIZES.Contribution, AccountType.Contribution)
      .key(8, A(31))
      .key(40, CONTRIBUTOR)
      .u64(72, 424242n)
      .u8(80, 1)
      .u8(81, 251);
    const c = decodeContribution(buf.bytes);
    expect(c.market.toString()).toBe(A(31).toString());
    expect(c.contributor.toString()).toBe(CONTRIBUTOR.toString());
    expect(c.amount).toBe(424242n);
    expect(c.claimed).toBe(true);
    expect(c.bump).toBe(251);
  });
});
