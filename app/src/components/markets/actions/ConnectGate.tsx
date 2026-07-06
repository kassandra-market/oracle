import type { ReactNode } from "react";
import { useWalletModal } from "@solana/wallet-adapter-react-ui";

/**
 * Gates its children behind a connected wallet. Disconnected → a muted note +
 * a "Connect wallet" affordance (opens the same wallet-adapter modal the nav
 * uses). Read-only browsing is unaffected — this only wraps the write inputs.
 */
export function ConnectGate({ connected, children }: { connected: boolean; children: ReactNode }) {
  const { setVisible } = useWalletModal();
  if (connected) return <>{children}</>;
  return (
    <div className="flex flex-wrap items-center gap-3">
      <p className="font-inter text-[14px] text-driftwood">Connect a wallet to participate.</p>
      <button
        type="button"
        onClick={() => setVisible(true)}
        className="rounded-button border border-pebble bg-soft-cream px-4 py-2 font-inter text-[13px] text-sepia hover:bg-pebble/60 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-pebble focus-visible:ring-offset-2 focus-visible:ring-offset-parchment"
      >
        Connect wallet
      </button>
    </div>
  );
}

export default ConnectGate;
