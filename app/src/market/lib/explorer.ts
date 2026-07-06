/**
 * Solana Explorer link helpers + a signature truncator.
 *
 * The app is RPC/cluster-agnostic (it only talks to the indexer), so these links
 * point at the public Explorer without a cluster hint — a cosmetic "view on
 * Explorer" affordance, not a data path. `VITE_EXPLORER_CLUSTER` (e.g. `devnet`)
 * can append a cluster query when a deployment wants it.
 */
const EXPLORER_CLUSTER = import.meta.env.VITE_EXPLORER_CLUSTER as string | undefined;

function clusterQuery(): string {
  return EXPLORER_CLUSTER ? `?cluster=${encodeURIComponent(EXPLORER_CLUSTER)}` : "";
}

/** A Solana Explorer address URL. */
export function explorerAddressUrl(address: string): string {
  return `https://explorer.solana.com/address/${address}${clusterQuery()}`;
}

/** A Solana Explorer transaction URL. */
export function explorerTxUrl(signature: string): string {
  return `https://explorer.solana.com/tx/${signature}${clusterQuery()}`;
}

/** A truncated base58 signature for the write-status success line (`abcd…wxyz`). */
export function shortSig(signature: string): string {
  return signature.length <= 12 ? signature : `${signature.slice(0, 6)}…${signature.slice(-6)}`;
}
