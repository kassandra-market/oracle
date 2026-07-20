/**
 * Offline unit tests for `src/market/data/send.ts` — the indexer-relay
 * send/confirm seam, focused on the pieces the batch-sign path added:
 * `buildUnsignedTx` and `sendSignedAndConfirm` (relay + confirm an
 * ALREADY-SIGNED transaction, the counterpart to `sendAndConfirm` used once
 * `signAllTransactions` has produced several signed transactions up front).
 */
import { Keypair, SystemProgram, Transaction, type Address } from "@solana/web3.js";
import { describe, expect, it } from "vitest";

import type { IndexerClient } from "../src/market/lib/indexer";
import { SendError, buildUnsignedTx, sendSignedAndConfirm } from "../src/market/data/send";

/** A mock IndexerClient: scripted signature status + a sendTransaction spy/stub. */
function mockIndexer(opts: {
  sendResult?: string | (() => never);
  statusSeq?: Array<{ status: string; err?: string | null }>;
} = {}): { indexer: IndexerClient; sent: string[] } {
  const sent: string[] = [];
  let statusCalls = 0;
  const statusSeq = opts.statusSeq ?? [{ status: "confirmed" }];
  const indexer = {
    getBlockhash: async () => "11111111111111111111111111111111111111111",
    sendTransaction: async (txBase64: string) => {
      sent.push(txBase64);
      if (typeof opts.sendResult === "function") return opts.sendResult();
      return opts.sendResult ?? "SIG_OK";
    },
    getSignatureStatus: async () => {
      const s = statusSeq[Math.min(statusCalls, statusSeq.length - 1)];
      statusCalls++;
      return s;
    },
  } as unknown as IndexerClient;
  return { indexer, sent };
}

/** A SIGNED transaction — `sendSignedAndConfirm` relays wire bytes, which requires a real signature. */
async function fixtureTx(): Promise<{ feePayer: Address; tx: Transaction }> {
  const payer = await Keypair.generate();
  const noop = SystemProgram.transfer({ fromPubkey: payer.publicKey, toPubkey: payer.publicKey, lamports: 0n });
  const tx = buildUnsignedTx(payer.publicKey, "11111111111111111111111111111111111111111", [noop]);
  await tx.sign(payer);
  return { feePayer: payer.publicKey, tx };
}

describe("buildUnsignedTx", () => {
  it("stamps the fee payer and blockhash, preserving instruction order", async () => {
    const feePayer = (await Keypair.generate()).publicKey;
    const programId = (await Keypair.generate()).publicKey;
    const ixA = { programId, keys: [], data: new Uint8Array([1]) };
    const ixB = { programId, keys: [], data: new Uint8Array([2]) };
    const tx = buildUnsignedTx(feePayer, "11111111111111111111111111111111111111111", [ixA, ixB]);
    expect(tx.feePayer?.toString()).toBe(feePayer.toString());
    expect(tx.recentBlockhash).toBe("11111111111111111111111111111111111111111");
    expect(tx.instructions.map((i) => Array.from(i.data))).toEqual([[1], [2]]);
  });
});

describe("sendSignedAndConfirm", () => {
  it("relays the already-signed wire bytes and confirms", async () => {
    const { indexer, sent } = mockIndexer({ sendResult: "SIG_OK" });
    const { tx } = await fixtureTx();
    const result = await sendSignedAndConfirm(indexer, tx);
    expect(result.signature).toBe("SIG_OK");
    expect(sent.length).toBe(1);
  });

  it("wraps a relay failure as a SendError with program logs", async () => {
    const { indexer } = mockIndexer({
      sendResult: () => {
        throw Object.assign(new Error("Simulation failed"), { logs: ["Program log: boom"] });
      },
    });
    const { tx } = await fixtureTx();
    await expect(sendSignedAndConfirm(indexer, tx)).rejects.toThrow(SendError);
    try {
      await sendSignedAndConfirm(indexer, tx);
      expect.unreachable();
    } catch (e) {
      expect(e).toBeInstanceOf(SendError);
      expect((e as SendError).logs).toEqual(["Program log: boom"]);
    }
  });

  it("wraps a failed confirmation as a SendError carrying the signature", async () => {
    const { indexer } = mockIndexer({
      sendResult: "SIG_BAD",
      statusSeq: [{ status: "failed", err: "custom program error: 0x1" }],
    });
    const { tx } = await fixtureTx();
    try {
      await sendSignedAndConfirm(indexer, tx);
      expect.unreachable();
    } catch (e) {
      expect(e).toBeInstanceOf(SendError);
      expect((e as SendError).signature).toBe("SIG_BAD");
    }
  });
});
