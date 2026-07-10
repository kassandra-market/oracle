/**
 * M1 — Meteora DAMM v2 (cp-amm) builder byte/meta tests.
 *
 * For each builder we assert `data == [disc, ...borsh args LE]` (rebuilt
 * INDEPENDENTLY here from the pinned disc + arg layout) and the account-meta
 * order/roles (against the `#[derive(Accounts)]` structs at commit
 * bdd8a1e355f484b3cff131578a662c560b97b72f), plus that the PDA derivers
 * reproduce the documented seeds. The decoder round-trips live in
 * meteora-decoders.test.ts; shared fixtures/helpers in ./helpers/meteora.ts.
 * Offline (default suite).
 */
import { Address } from "@solana/web3.js";
import { describe, expect, it } from "vitest";

import { meteora } from "../src/index.js";
import { TOKEN_PROGRAM_ID, SYSTEM_PROGRAM_ID, EXTERNAL_PROGRAM_IDS } from "../src/constants.js";
import {
  CONFIG,
  CREATOR,
  DISC,
  METEORA_DAMM_V2_ID,
  MINT_A,
  MINT_B,
  NFT_MINT,
  PAYER,
  PAYER_TOKEN_A,
  PAYER_TOKEN_B,
  POOL_ACCOUNT_DISCRIMINATOR,
  POOL_ACCOUNT_SIZE,
  POOL_INIT_SPACE,
  POSITION_ACCOUNT_DISCRIMINATOR,
  POSITION_ACCOUNT_SIZE,
  POSITION_INIT_SPACE,
  SEED,
  TOKEN_2022_PROGRAM_ID,
  cat,
  hex,
  metasEq,
  pda,
  poolAddr,
  ro,
  u128,
  u64,
  w,
} from "./helpers/meteora.js";

describe("meteora wire constants", () => {
  it("pins the program id (lib.rs:41 declare_id!)", () => {
    expect(METEORA_DAMM_V2_ID.toString()).toBe("cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG");
    expect(METEORA_DAMM_V2_ID.toString()).toBe(EXTERNAL_PROGRAM_IDS.meteoraDammV2.toString());
  });

  it("cross-checks the 3 program-pinned discs + computes the 3 new ones", () => {
    // pinned in programs/oracles/src/cpi/metadao_v06.rs:127-131
    expect(hex(DISC.initializePool)).toBe("5fb40aac54aee828");
    expect(hex(DISC.swap)).toBe("f8c69e91e17587c8");
    expect(hex(DISC.addLiquidity)).toBe("b59d59438fb63448");
    // computed sha256("global:<name>")[..8]
    expect(hex(DISC.createPosition)).toBe("30d7c59960cbb485");
    expect(hex(DISC.removeLiquidity)).toBe("5055d14818ceb16c");
    expect(hex(DISC.claimPositionFee)).toBe("b4269a118521a2d3");
  });

  it("pins the account discriminators + sizes", () => {
    expect(hex(POOL_ACCOUNT_DISCRIMINATOR)).toBe("f19a6d0411b16dbc"); // metadao_v06.rs:133
    expect(hex(POSITION_ACCOUNT_DISCRIMINATOR)).toBe("aabc8fe47a40f7d0");
    expect(POOL_INIT_SPACE).toBe(1104);
    expect(POSITION_INIT_SPACE).toBe(400);
    expect(POOL_ACCOUNT_SIZE).toBe(1112);
    expect(POSITION_ACCOUNT_SIZE).toBe(408);
  });
});

describe("meteora PDA derivers (byte-sourced seeds)", () => {
  it("pool == [b\"pool\", config, max(a,b), min(a,b)] and is mint-order-independent", async () => {
    const [hi, lo] = pda.sortMints(MINT_A, MINT_B);
    const expected = (
      await Address.findProgramAddress(
        [SEED.pool, new Address(CONFIG).toBytes(), hi, lo],
        METEORA_DAMM_V2_ID,
      )
    )[0];
    expect((await pda.pool(CONFIG, MINT_A, MINT_B)).address.toString()).toBe(expected.toString());
    // swapping the two mints yields the SAME pool (sorted seeds)
    expect((await pda.pool(CONFIG, MINT_B, MINT_A)).address.toString()).toBe(expected.toString());
  });

  it("position == [b\"position\", nft_mint]", async () => {
    const expected = (
      await Address.findProgramAddress([SEED.position, new Address(NFT_MINT).toBytes()], METEORA_DAMM_V2_ID)
    )[0];
    expect((await pda.position(NFT_MINT)).address.toString()).toBe(expected.toString());
  });

  it("positionNftAccount == [b\"position_nft_account\", nft_mint]", async () => {
    const expected = (
      await Address.findProgramAddress([SEED.positionNftAccount, new Address(NFT_MINT).toBytes()], METEORA_DAMM_V2_ID)
    )[0];
    expect((await pda.positionNftAccount(NFT_MINT)).address.toString()).toBe(expected.toString());
  });

  it("tokenVault == [b\"token_vault\", mint, pool]", async () => {
    const p = await poolAddr();
    const expected = (
      await Address.findProgramAddress(
        [SEED.tokenVault, new Address(MINT_A).toBytes(), p.toBytes()],
        METEORA_DAMM_V2_ID,
      )
    )[0];
    expect((await pda.tokenVault(MINT_A, p)).address.toString()).toBe(expected.toString());
  });

  it("poolAuthority == [b\"pool_authority\"] and eventAuthority == [b\"__event_authority\"]", async () => {
    const pa = (await Address.findProgramAddress([SEED.poolAuthority], METEORA_DAMM_V2_ID))[0];
    const ea = (await Address.findProgramAddress([SEED.eventAuthority], METEORA_DAMM_V2_ID))[0];
    expect((await pda.poolAuthority()).address.toString()).toBe(pa.toString());
    expect((await pda.eventAuthority()).address.toString()).toBe(ea.toString());
  });
});

describe("initializePool (InitializePoolCtx)", () => {
  it("data == disc ++ liquidity:u128 ++ sqrtPrice:u128 ++ Option<u64> and metas match", async () => {
    const liquidity = 1_000_000_000_000_000_000n;
    const sqrtPrice = 18446744073709551616n; // 1.0 in Q64.64
    const ix = await meteora.initializePool({
      creator: CREATOR,
      payer: PAYER,
      positionNftMint: NFT_MINT,
      config: CONFIG,
      tokenAMint: MINT_A,
      tokenBMint: MINT_B,
      payerTokenA: PAYER_TOKEN_A,
      payerTokenB: PAYER_TOKEN_B,
      liquidity,
      sqrtPrice,
      activationPoint: 12345n,
    });
    expect(hex(ix.data)).toBe(
      hex(cat(DISC.initializePool, u128(liquidity), u128(sqrtPrice), Uint8Array.from([1]), u64(12345n))),
    );

    const p = await poolAddr();
    const position = (await pda.position(NFT_MINT)).address;
    const nftAcc = (await pda.positionNftAccount(NFT_MINT)).address;
    const vaultA = (await pda.tokenVault(MINT_A, p)).address;
    const vaultB = (await pda.tokenVault(MINT_B, p)).address;
    const poolAuth = (await pda.poolAuthority()).address;
    const eventAuth = (await pda.eventAuthority()).address;
    metasEq(ix.keys, [
      ro(CREATOR),
      w(NFT_MINT, true),
      w(nftAcc),
      w(PAYER, true),
      ro(CONFIG),
      ro(poolAuth),
      w(p),
      w(position),
      ro(MINT_A),
      ro(MINT_B),
      w(vaultA),
      w(vaultB),
      w(PAYER_TOKEN_A),
      w(PAYER_TOKEN_B),
      ro(TOKEN_PROGRAM_ID),
      ro(TOKEN_PROGRAM_ID),
      ro(TOKEN_2022_PROGRAM_ID),
      ro(SYSTEM_PROGRAM_ID),
      ro(eventAuth),
      ro(METEORA_DAMM_V2_ID),
    ]);
  });

  it("encodes activationPoint None as a single 0 byte", async () => {
    const ix = await meteora.initializePool({
      creator: CREATOR,
      payer: PAYER,
      positionNftMint: NFT_MINT,
      config: CONFIG,
      tokenAMint: MINT_A,
      tokenBMint: MINT_B,
      payerTokenA: PAYER_TOKEN_A,
      payerTokenB: PAYER_TOKEN_B,
      liquidity: 1n,
      sqrtPrice: 2n,
    });
    expect(ix.data.length).toBe(8 + 16 + 16 + 1);
    expect(ix.data[ix.data.length - 1]).toBe(0);
  });
});

describe("createPosition (CreatePositionCtx)", () => {
  it("data == disc (no args) and metas match", async () => {
    const p = await poolAddr();
    const ix = await meteora.createPosition({ owner: CREATOR, positionNftMint: NFT_MINT, pool: p, payer: PAYER });
    expect(hex(ix.data)).toBe(hex(DISC.createPosition));
    expect(ix.data.length).toBe(8);

    const position = (await pda.position(NFT_MINT)).address;
    const nftAcc = (await pda.positionNftAccount(NFT_MINT)).address;
    const poolAuth = (await pda.poolAuthority()).address;
    const eventAuth = (await pda.eventAuthority()).address;
    metasEq(ix.keys, [
      ro(CREATOR),
      w(NFT_MINT, true),
      w(nftAcc),
      w(p),
      w(position),
      ro(poolAuth),
      w(PAYER, true),
      ro(TOKEN_2022_PROGRAM_ID),
      ro(SYSTEM_PROGRAM_ID),
      ro(eventAuth),
      ro(METEORA_DAMM_V2_ID),
    ]);
  });
});

describe("addLiquidity / removeLiquidity (Modify liquidity)", () => {
  const common = async () => {
    const p = await poolAddr();
    const position = (await pda.position(NFT_MINT)).address;
    const nftAcc = (await pda.positionNftAccount(NFT_MINT)).address;
    const vaultA = (await pda.tokenVault(MINT_A, p)).address;
    const vaultB = (await pda.tokenVault(MINT_B, p)).address;
    return {
      pool: p,
      position,
      tokenAAccount: PAYER_TOKEN_A,
      tokenBAccount: PAYER_TOKEN_B,
      tokenAVault: vaultA,
      tokenBVault: vaultB,
      tokenAMint: MINT_A,
      tokenBMint: MINT_B,
      positionNftAccount: nftAcc,
      signer: PAYER,
      liquidityDelta: 500n,
      tokenAAmountThreshold: 999n,
      tokenBAmountThreshold: 888n,
    };
  };

  it("addLiquidity: data == disc ++ u128 ++ u64 ++ u64 and metas match", async () => {
    const args = await common();
    const ix = await meteora.addLiquidity(args);
    expect(hex(ix.data)).toBe(hex(cat(DISC.addLiquidity, u128(500n), u64(999n), u64(888n))));
    const eventAuth = (await pda.eventAuthority()).address;
    metasEq(ix.keys, [
      w(args.pool),
      w(args.position),
      w(PAYER_TOKEN_A),
      w(PAYER_TOKEN_B),
      w(args.tokenAVault),
      w(args.tokenBVault),
      ro(MINT_A),
      ro(MINT_B),
      ro(args.positionNftAccount),
      ro(PAYER, true),
      ro(TOKEN_PROGRAM_ID),
      ro(TOKEN_PROGRAM_ID),
      ro(eventAuth),
      ro(METEORA_DAMM_V2_ID),
    ]);
  });

  it("removeLiquidity: same body, metas PREFIXED with pool_authority", async () => {
    const args = await common();
    const ix = await meteora.removeLiquidity(args);
    expect(hex(ix.data)).toBe(hex(cat(DISC.removeLiquidity, u128(500n), u64(999n), u64(888n))));
    const poolAuth = (await pda.poolAuthority()).address;
    const eventAuth = (await pda.eventAuthority()).address;
    metasEq(ix.keys, [
      ro(poolAuth),
      w(args.pool),
      w(args.position),
      w(PAYER_TOKEN_A),
      w(PAYER_TOKEN_B),
      w(args.tokenAVault),
      w(args.tokenBVault),
      ro(MINT_A),
      ro(MINT_B),
      ro(args.positionNftAccount),
      ro(PAYER, true),
      ro(TOKEN_PROGRAM_ID),
      ro(TOKEN_PROGRAM_ID),
      ro(eventAuth),
      ro(METEORA_DAMM_V2_ID),
    ]);
  });
});

describe("swap (SwapCtx)", () => {
  it("data == disc ++ amountIn:u64 ++ minOut:u64 (no swap_type) and metas match; None referral => program id (ro)", async () => {
    const p = await poolAddr();
    const vaultA = (await pda.tokenVault(MINT_A, p)).address;
    const vaultB = (await pda.tokenVault(MINT_B, p)).address;
    const ix = await meteora.swap({
      pool: p,
      inputTokenAccount: PAYER_TOKEN_A,
      outputTokenAccount: PAYER_TOKEN_B,
      tokenAVault: vaultA,
      tokenBVault: vaultB,
      tokenAMint: MINT_A,
      tokenBMint: MINT_B,
      payer: PAYER,
      amountIn: 1_000_000n,
      minimumAmountOut: 990_000n,
    });
    expect(hex(ix.data)).toBe(hex(cat(DISC.swap, u64(1_000_000n), u64(990_000n))));
    const poolAuth = (await pda.poolAuthority()).address;
    const eventAuth = (await pda.eventAuthority()).address;
    metasEq(ix.keys, [
      ro(poolAuth),
      w(p),
      w(PAYER_TOKEN_A),
      w(PAYER_TOKEN_B),
      w(vaultA),
      w(vaultB),
      ro(MINT_A),
      ro(MINT_B),
      ro(PAYER, true),
      ro(TOKEN_PROGRAM_ID),
      ro(TOKEN_PROGRAM_ID),
      ro(METEORA_DAMM_V2_ID), // referral None sentinel = program id, read-only
      ro(eventAuth),
      ro(METEORA_DAMM_V2_ID),
    ]);
  });

  it("with a referral account, that slot is writable", async () => {
    const p = await poolAddr();
    const ix = await meteora.swap({
      pool: p,
      inputTokenAccount: PAYER_TOKEN_A,
      outputTokenAccount: PAYER_TOKEN_B,
      tokenAVault: (await pda.tokenVault(MINT_A, p)).address,
      tokenBVault: (await pda.tokenVault(MINT_B, p)).address,
      tokenAMint: MINT_A,
      tokenBMint: MINT_B,
      payer: PAYER,
      amountIn: 1n,
      referralTokenAccount: PAYER_TOKEN_A,
    });
    const referralMeta = ix.keys[11];
    expect(referralMeta.pubkey.toString()).toBe(PAYER_TOKEN_A);
    expect(referralMeta.isWritable).toBe(true);
  });
});

describe("claimPositionFee (ClaimPositionFeeCtx)", () => {
  it("data == disc (no args) and pool is READ-ONLY", async () => {
    const p = await poolAddr();
    const position = (await pda.position(NFT_MINT)).address;
    const nftAcc = (await pda.positionNftAccount(NFT_MINT)).address;
    const vaultA = (await pda.tokenVault(MINT_A, p)).address;
    const vaultB = (await pda.tokenVault(MINT_B, p)).address;
    const ix = await meteora.claimPositionFee({
      pool: p,
      position,
      tokenAAccount: PAYER_TOKEN_A,
      tokenBAccount: PAYER_TOKEN_B,
      tokenAVault: vaultA,
      tokenBVault: vaultB,
      tokenAMint: MINT_A,
      tokenBMint: MINT_B,
      positionNftAccount: nftAcc,
      signer: PAYER,
    });
    expect(hex(ix.data)).toBe(hex(DISC.claimPositionFee));
    const poolAuth = (await pda.poolAuthority()).address;
    const eventAuth = (await pda.eventAuthority()).address;
    metasEq(ix.keys, [
      ro(poolAuth),
      ro(p), // pool is read-only in claim_position_fee
      w(position),
      w(PAYER_TOKEN_A),
      w(PAYER_TOKEN_B),
      w(vaultA),
      w(vaultB),
      ro(MINT_A),
      ro(MINT_B),
      ro(nftAcc),
      ro(PAYER, true),
      ro(TOKEN_PROGRAM_ID),
      ro(TOKEN_PROGRAM_ID),
      ro(eventAuth),
      ro(METEORA_DAMM_V2_ID),
    ]);
  });
});
