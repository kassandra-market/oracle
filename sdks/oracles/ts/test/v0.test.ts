/**
 * I2 offline unit test for the v0-tx + ALT path (`src/v0.ts`).
 *
 * Hermetic (NO validator): builds a near-cap `finalizeOracle` instruction, then
 * proves — with a MOCK `AddressLookupTableAccount` — that
 *   1. `compileV0Message` produces a v0 (`version === 0`) message that references
 *      the ALT (one `addressTableLookups` entry),
 *   2. the read-only proposer set is packed into the table's `readonlyIndexes`
 *      and those indices resolve back to the exact proposer PDAs (ALT-key
 *      packing is correct),
 *   3. the same instruction as a LEGACY message OVERFLOWS the 1232-byte packet
 *      while the v0 message fits — i.e. the v0/ALT path removes the limitation.
 *
 * The surfpool proof (`test/surfpool/v0-alt-e2e.test.ts`) drives the live ALT
 * create/extend/activate + a real v0 send; this file covers what is testable
 * without a slot-progressing cluster.
 */
import {
  AddressLookupTableAccount,
  Address,
  TransactionMessage,
  type Blockhash,
} from "@solana/web3.js";
import { describe, expect, it } from "vitest";

import { finalizeOracle } from "../src/instructions/index.js";
import { compileV0Message } from "../src/v0.js";

/** Solana single-packet limit (`PACKET_DATA_SIZE`). */
const PACKET_DATA_SIZE = 1232;

/** A deterministic, distinct 32-byte address (not necessarily on-curve — fine for an ALT). */
function fakeAddress(seed: number): Address {
  const bytes = new Uint8Array(32);
  bytes[0] = seed & 0xff;
  bytes[1] = (seed >> 8) & 0xff;
  bytes[2] = 0xa5;
  bytes[31] = 0x01;
  return new Address(bytes);
}

describe("v0 + ALT finalize path (offline)", () => {
  // 40 proposers: comfortably past the ~28-proposer legacy overflow threshold,
  // under MAX_PROPOSERS = 60.
  const PROPOSER_COUNT = 40;
  const proposers = Array.from({ length: PROPOSER_COUNT }, (_, i) => fakeAddress(i + 1000));
  const payer = fakeAddress(1);
  const kassMint = fakeAddress(2);
  const blockhash = "11111111111111111111111111111111" as Blockhash;

  it("legacy compiled message overflows the 1232-byte packet at 40 proposers", async () => {
    const ix = await finalizeOracle({ nonce: 7n, kassMint, proposers });
    const legacy = new TransactionMessage({
      payerKey: payer,
      recentBlockhash: blockhash,
      instructions: [ix],
    }).compileToLegacyMessage();
    // The compiled message ALONE exceeds the packet — before signatures.
    expect(legacy.serialize().length).toBeGreaterThan(PACKET_DATA_SIZE);
  });

  it("v0 message over the ALT references the proposers by index and fits the packet", async () => {
    const ix = await finalizeOracle({ nonce: 7n, kassMint, proposers });

    // Mock the resolved ALT holding exactly the proposer PDAs.
    const alt = new AddressLookupTableAccount({
      key: fakeAddress(9999),
      state: {
        deactivationSlot: 2n ** 64n - 1n, // never deactivated → isActive()
        lastExtendedSlot: 1n,
        lastExtendedSlotStartIndex: 0,
        authority: payer,
        addresses: [...proposers],
      },
    });

    const msg = compileV0Message({
      payer,
      instructions: [ix],
      lookupTableAccounts: [alt],
      recentBlockhash: blockhash,
    });

    // (1) it is a v0 message that references the one ALT.
    expect(msg.version).toBe(0);
    expect(msg.addressTableLookups.length).toBe(1);
    const lookup = msg.addressTableLookups[0];
    expect(lookup.accountKey.toString()).toBe(alt.key.toString());

    // (2) all 40 read-only proposers are packed into the table's readonlyIndexes,
    //     and those indices resolve back to the exact proposer PDAs.
    expect(lookup.writableIndexes.length).toBe(0);
    expect(lookup.readonlyIndexes.length).toBe(PROPOSER_COUNT);
    const resolved = new Set(lookup.readonlyIndexes.map((i) => alt.state.addresses[i].toString()));
    for (const p of proposers) expect(resolved.has(p.toString())).toBe(true);

    // (3) the v0 message fits the packet where the legacy one did not.
    const v0Len = msg.serialize().length;
    const legacyLen = new TransactionMessage({
      payerKey: payer,
      recentBlockhash: blockhash,
      instructions: [ix],
    })
      .compileToLegacyMessage()
      .serialize().length;
    expect(v0Len).toBeLessThan(PACKET_DATA_SIZE);
    expect(v0Len).toBeLessThan(legacyLen);
  });
});
