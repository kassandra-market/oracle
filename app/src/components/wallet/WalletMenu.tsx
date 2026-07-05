/**
 * The Wallet-Standard connect picker (replaces the legacy wallet-adapter-react-ui
 * modal). Lists the wallets the browser has registered and connects the chosen
 * one via its per-wallet `useConnect` hook, handing the first account to the
 * {@link StandardWalletProvider}. Rendered once, near the app root; visibility is
 * driven by `useWalletMenu().open`.
 */
import { useConnect, type UiWallet, type UiWalletAccount } from '@wallet-standard/react'

import { useWalletMenu } from '../../lib/standardWallet'

function WalletRow({
  wallet,
  onConnected,
}: {
  wallet: UiWallet
  onConnected: (account: UiWalletAccount) => void
}) {
  const [isConnecting, connect] = useConnect(wallet)
  return (
    <button
      type="button"
      disabled={isConnecting}
      onClick={async () => {
        const accounts = await connect()
        if (accounts[0]) onConnected(accounts[0])
      }}
      className="flex items-center gap-3 rounded-button border border-pebble bg-liquid-kelp px-4 py-3 font-inter text-[14px] text-platinum hover:bg-[#04524c] disabled:opacity-60"
    >
      {wallet.icon ? <img src={wallet.icon} alt="" className="h-5 w-5 rounded" /> : null}
      <span className="flex-1 text-left">{wallet.name}</span>
      {isConnecting ? <span className="text-driftwood">Connecting…</span> : null}
    </button>
  )
}

export function WalletMenu() {
  const { wallets, open, setOpen, adopt } = useWalletMenu()
  if (!open) return null
  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-label="Connect a wallet"
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4"
      onClick={() => setOpen(false)}
    >
      <div
        className="w-full max-w-sm rounded-card border border-pebble bg-liquid-deep p-6"
        onClick={(e) => e.stopPropagation()}
      >
        <h2 className="font-serif text-heading-sm font-light text-platinum">Connect a wallet</h2>
        <div className="mt-4 flex flex-col gap-2">
          {wallets.length === 0 ? (
            <p className="font-inter text-[14px] text-driftwood">
              No wallets detected. Install Phantom or Solflare (and, for this local chain, add a
              custom RPC pointing at 127.0.0.1:8899).
            </p>
          ) : (
            wallets.map((w) => (
              <WalletRow
                key={w.name}
                wallet={w}
                onConnected={(a) => {
                  adopt(a)
                  setOpen(false)
                }}
              />
            ))
          )}
        </div>
      </div>
    </div>
  )
}

export default WalletMenu
