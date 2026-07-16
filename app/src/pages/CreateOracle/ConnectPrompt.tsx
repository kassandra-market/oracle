import { useWalletMenu } from '../../lib/standardWallet'
import { Card } from '../../components/ui'

/** Disconnected gate with copy tailored to creating an oracle. */
export function ConnectPrompt() {
  const { setOpen } = useWalletMenu()
  return (
    <Card className="flex flex-wrap items-center gap-3">
      <p className="font-inter text-[15px] text-driftwood">Connect a wallet to create an oracle.</p>
      <button
        type="button"
        onClick={() => setOpen(true)}
        className="rounded-button border border-pebble bg-soft-cream px-4 py-2 font-inter text-[13px] text-sepia hover:bg-pebble/60 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-pebble focus-visible:ring-offset-2 focus-visible:ring-offset-parchment"
      >
        Connect wallet
      </button>
    </Card>
  )
}
