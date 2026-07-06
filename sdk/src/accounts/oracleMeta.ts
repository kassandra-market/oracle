/**
 * Decoder for the `oracle_meta` account (written by `write_oracle_meta`). NOT a
 * fixed Pod struct — a length-prefixed variable buffer, so it's parsed with
 * explicit offsets rather than the `assertAccount` + fixed-size path:
 *
 *   account_type u8 ++ bump u8 ++ oracle[32] ++ subject_len u16 ++ subject ++
 *   options_count u8 ++ [option_len u16 ++ option]* ++ uri_len u16 ++ uri ++
 *   uri_hash[32]
 *
 * The subject/options are the program-readable on-chain metadata; the `uri` +
 * `uriHash` point at (and bind) the extended off-chain JSON.
 */
import { Address } from "@solana/web3.js";

import { AccountType } from "../constants.js";
import { readPubkey, view } from "./common.js";

export interface OracleMeta {
  accountType: AccountType.OracleMeta;
  /** The oracle this metadata describes (back-reference). */
  oracle: Address;
  /** The plaintext question. */
  subject: string;
  /** The per-outcome option labels. */
  options: string[];
  /** URL of the extended off-chain metadata JSON (may be empty). */
  uri: string;
  /** 32-byte `sha256` binding the off-chain JSON. */
  uriHash: Uint8Array;
}

/** Decode an `oracle_meta` account buffer. Throws on a wrong tag / short input. */
export function decodeOracleMeta(data: Uint8Array): OracleMeta {
  if (data[0] !== AccountType.OracleMeta) {
    throw new Error(`not an OracleMeta account (account_type tag ${data[0]})`);
  }
  const dv = view(data);
  const dec = new TextDecoder();
  const oracle = readPubkey(data, 2);
  let off = 34;
  const u16 = (): number => {
    const v = dv.getUint16(off, true);
    off += 2;
    return v;
  };
  const str = (len: number): string => {
    const s = dec.decode(data.subarray(off, off + len));
    off += len;
    return s;
  };

  const subject = str(u16());
  const optionsCount = data[off];
  off += 1;
  const options: string[] = [];
  for (let i = 0; i < optionsCount; i += 1) options.push(str(u16()));
  const uri = str(u16());
  const uriHash = data.slice(off, off + 32);

  return { accountType: AccountType.OracleMeta, oracle, subject, options, uri, uriHash };
}
