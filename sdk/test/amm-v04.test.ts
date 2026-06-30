/**
 * CS1 — MetaDAO v0.4 standalone AMM (`AMMyu265…`) builder byte/meta tests.
 *
 * For each builder we assert `data == [disc, ...args LE]` (the expected buffer
 * built INDEPENDENTLY here from the `cpi/metadao.rs:82-94` disc + arg layout) and
 * the account-meta order/roles (against the real-`.so`-proven orderings in
 * `programs/kassandra/tests/challenge_e2e.rs:676-769`), plus that the PDA derivers
 * reproduce the documented seeds. Offline (default suite).
 */
import { Address } from "@solana/web3.js";
import { describe, expect, it } from "vitest";

import { ammV04 } from "../src/index.js";
import { TOKEN_PROGRAM_ID, SYSTEM_PROGRAM_ID, EXTERNAL_PROGRAM_IDS } from "../src/constants.js";

const { DISC, SwapType, AMM_V04_ID, ATA_PROGRAM_ID, SEED, pda } = ammV04;

// Deterministic valid base58 stand-ins.
const PAYER = "rqRMW2HFJsi1FE1jb8Rvaz4Qz3xHzNkZDb8am1pqEHE";
const BASE_MINT = "So11111111111111111111111111111111111111112";
const QUOTE_MINT = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";

const enc = new TextEncoder();
const hex = (b: Uint8Array) => Buffer.from(b).toString("hex");
const u8 = (v: number) => Uint8Array.from([v & 0xff]);
const u64 = (v: bigint) => {
  const o = new Uint8Array(8);
  new DataView(o.buffer).setBigUint64(0, v, true);
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
      [new Address(owner as string).toBytes(), TOKEN_PROGRAM_ID.toBytes(), new Address(mint as string).toBytes()],
      ATA_PROGRAM_ID,
    )
  )[0];

// Independently re-derived PDAs (matching challenge_e2e.rs:641-650).
const ammPda = async () =>
  (
    await Address.findProgramAddress(
      [SEED.amm, new Address(BASE_MINT).toBytes(), new Address(QUOTE_MINT).toBytes()],
      AMM_V04_ID,
    )
  )[0];
const lpPda = async (amm: Address) =>
  (await Address.findProgramAddress([SEED.lpMint, amm.toBytes()], AMM_V04_ID))[0];
const eventAuthPda = async () =>
  (await Address.findProgramAddress([SEED.eventAuthority], AMM_V04_ID))[0];

type Meta = { pubkey: Address; isSigner: boolean; isWritable: boolean };
const w = (p: Address, s = false): Meta => ({ pubkey: p, isSigner: s, isWritable: true });
const ro = (p: Address, s = false): Meta => ({ pubkey: p, isSigner: s, isWritable: false });
const metasEq = (got: readonly Meta[], want: Meta[]) => {
  expect(got.length).toBe(want.length);
  got.forEach((m, i) => {
    expect(m.pubkey.toString()).toBe(want[i].pubkey.toString());
    expect(m.isSigner).toBe(want[i].isSigner);
    expect(m.isWritable).toBe(want[i].isWritable);
  });
};

describe("amm-v04 wire constants", () => {
  it("pins the program id (metadao.rs:59)", () => {
    expect(AMM_V04_ID.toString()).toBe("AMMyu265tkBpRW21iGQxKGLaves3gKm2JcMUqfXNSpqD");
    expect(AMM_V04_ID.toString()).toBe(EXTERNAL_PROGRAM_IDS.ammV04.toString());
  });

  it("pins the binary-validated discriminators (metadao.rs:82-94)", () => {
    expect(hex(DISC.createAmm)).toBe("f25b15aa05447d40");
    expect(hex(DISC.addLiquidity)).toBe("b59d59438fb63448");
    expect(hex(DISC.swap)).toBe("f8c69e91e17587c8");
    expect(hex(DISC.crankThatTwap)).toBe("dc6419f9005cc3c1");
  });

  it("pins the seed prefixes (challenge_e2e.rs:641-646)", () => {
    expect(hex(SEED.amm)).toBe(hex(enc.encode("amm__")));
    expect(hex(SEED.lpMint)).toBe(hex(enc.encode("amm_lp_mint")));
    expect(hex(SEED.eventAuthority)).toBe(hex(enc.encode("__event_authority")));
  });
});

describe("amm-v04 PDA derivers (challenge_e2e.rs:641-650)", () => {
  it("amm == [b\"amm__\", base, quote]", async () => {
    expect((await pda.amm(BASE_MINT, QUOTE_MINT)).address.toString()).toBe((await ammPda()).toString());
  });
  it("lpMint == [b\"amm_lp_mint\", amm]", async () => {
    const amm = await ammPda();
    expect((await pda.lpMint(amm)).address.toString()).toBe((await lpPda(amm)).toString());
  });
  it("eventAuthority == [b\"__event_authority\"]", async () => {
    expect((await pda.eventAuthority()).address.toString()).toBe((await eventAuthPda()).toString());
  });
  it("vault ATAs == ata(amm, mint)", async () => {
    const amm = await ammPda();
    expect((await pda.ata(amm, BASE_MINT)).toString()).toBe((await ata(amm, BASE_MINT)).toString());
    expect((await pda.ata(amm, QUOTE_MINT)).toString()).toBe((await ata(amm, QUOTE_MINT)).toString());
  });
});

describe("createAmm (challenge_e2e.rs:676-689)", () => {
  it("data == disc ++ u128 ++ u128 ++ u64 and metas match", async () => {
    const initialObs = 1_500_000_000_000n;
    const maxChange = 0xffffffffffffffffffffffffffffffffn;
    const startDelay = 0n;
    const ix = await ammV04.createAmm({
      payer: PAYER,
      baseMint: BASE_MINT,
      quoteMint: QUOTE_MINT,
      twapInitialObservation: initialObs,
      twapMaxObservationChangePerUpdate: maxChange,
      twapStartDelaySlots: startDelay,
    });
    expect(hex(ix.data)).toBe(hex(cat(DISC.createAmm, u128(initialObs), u128(maxChange), u64(startDelay))));
    expect(ix.data.length).toBe(8 + 16 + 16 + 8);

    const amm = await ammPda();
    const lp = await lpPda(amm);
    metasEq(ix.keys, [
      w(new Address(PAYER), true),
      w(amm),
      w(lp),
      ro(new Address(BASE_MINT)),
      ro(new Address(QUOTE_MINT)),
      w(await ata(amm, BASE_MINT)),
      w(await ata(amm, QUOTE_MINT)),
      ro(ATA_PROGRAM_ID),
      ro(TOKEN_PROGRAM_ID),
      ro(SYSTEM_PROGRAM_ID),
      ro(await eventAuthPda()),
      ro(AMM_V04_ID),
    ]);
  });

  it("defaults twapStartDelaySlots to 0", async () => {
    const ix = await ammV04.createAmm({
      payer: PAYER,
      baseMint: BASE_MINT,
      quoteMint: QUOTE_MINT,
      twapInitialObservation: 1n,
      twapMaxObservationChangePerUpdate: 2n,
    });
    expect(hex(ix.data.slice(40))).toBe(hex(u64(0n)));
  });
});

describe("addLiquidity (challenge_e2e.rs:703-714)", () => {
  it("data == disc ++ quote:u64 ++ maxBase:u64 ++ minLp:u64 and metas match", async () => {
    const quote = 5_000_000n;
    const maxBase = 3_000_000n;
    const minLp = 0n;
    const ix = await ammV04.addLiquidity({
      payer: PAYER,
      baseMint: BASE_MINT,
      quoteMint: QUOTE_MINT,
      quoteAmount: quote,
      maxBaseAmount: maxBase,
      minLpTokens: minLp,
    });
    expect(hex(ix.data)).toBe(hex(cat(DISC.addLiquidity, u64(quote), u64(maxBase), u64(minLp))));

    const amm = await ammPda();
    const lp = await lpPda(amm);
    metasEq(ix.keys, [
      w(new Address(PAYER), true),
      w(amm),
      w(lp),
      w(await ata(PAYER, lp)),
      w(await ata(PAYER, BASE_MINT)),
      w(await ata(PAYER, QUOTE_MINT)),
      w(await ata(amm, BASE_MINT)),
      w(await ata(amm, QUOTE_MINT)),
      ro(TOKEN_PROGRAM_ID),
      ro(await eventAuthPda()),
      ro(AMM_V04_ID),
    ]);
  });
});

describe("swap (challenge_e2e.rs:723-752)", () => {
  it("Buy: data == disc ++ type:u8(0) ++ amount:u64 ++ minOut:u64 and metas match", async () => {
    const amount = 90_000_000n;
    const minOut = 0n;
    const ix = await ammV04.swap({
      payer: PAYER,
      baseMint: BASE_MINT,
      quoteMint: QUOTE_MINT,
      swapType: SwapType.Buy,
      inputAmount: amount,
      minOutputAmount: minOut,
    });
    expect(hex(ix.data)).toBe(hex(cat(DISC.swap, u8(0), u64(amount), u64(minOut))));

    const amm = await ammPda();
    metasEq(ix.keys, [
      w(new Address(PAYER), true),
      w(amm),
      w(await ata(PAYER, BASE_MINT)),
      w(await ata(PAYER, QUOTE_MINT)),
      w(await ata(amm, BASE_MINT)),
      w(await ata(amm, QUOTE_MINT)),
      ro(TOKEN_PROGRAM_ID),
      ro(await eventAuthPda()),
      ro(AMM_V04_ID),
    ]);
  });

  it("Sell: swap_type tag == 1", async () => {
    const ix = await ammV04.swap({
      payer: PAYER,
      baseMint: BASE_MINT,
      quoteMint: QUOTE_MINT,
      swapType: SwapType.Sell,
      inputAmount: 1n,
    });
    expect(ix.data[8]).toBe(1);
  });
});

describe("crankThatTwap (challenge_e2e.rs:756-769)", () => {
  it("data == disc (no args) and metas == [amm(w), event_authority, amm_program]", async () => {
    const amm = await ammPda();
    const ix = await ammV04.crankThatTwap({ amm });
    expect(hex(ix.data)).toBe(hex(DISC.crankThatTwap));
    expect(ix.data.length).toBe(8);
    metasEq(ix.keys, [w(amm), ro(await eventAuthPda()), ro(AMM_V04_ID)]);
  });
});
