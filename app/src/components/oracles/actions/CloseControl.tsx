import type { FormEvent } from 'react'
import type { TransactionInstruction } from '@solana/web3.js'
import { useWriteAction } from '../../../hooks/useWriteAction'
import { ConnectGate } from './ConnectGate'
import { SubmitButton } from './formPrimitives'
import { WriteStatusRegion } from './WriteStatusRegion'

/**
 * RF2 — a permissionless CLOSE control (chestnut): reaps a terminal child
 * account (an AiClaim or a settled Market + its escrow) and refunds the rent to
 * its owner. Like the RF1 finalize crank, the close instruction carries no
 * required signer — ANY connected wallet may send it (it only pays the fee); the
 * rent still routes to the recorded recipient. Wallet-backed sender + shared
 * write-status UX + a refetch on success (the closed account disappears).
 */
export function CloseControl({
  verb,
  successVerb,
  description,
  build,
  refetch,
}: {
  /** Idle button label, e.g. "Close AI claim". */
  verb: string
  /** Past-tense confirmation verb, e.g. "Closed". */
  successVerb: string
  /** Short helper line under the button. */
  description: string
  /** Assemble the close ixs at click time (skipped under mock mode). */
  build: () => Promise<TransactionInstruction[]>
  refetch: () => void
}) {
  const action = useWriteAction(refetch)

  const onSubmit = (e: FormEvent) => {
    e.preventDefault()
    void action.run(build)
  }

  return (
    <div className="mt-1 border-t border-pebble pt-3">
      <ConnectGate connected={action.connected}>
        <form className="flex flex-col gap-2" onSubmit={onSubmit} noValidate>
          <div className="flex items-center gap-3">
            <SubmitButton verb={verb} status={action.status} />
          </div>
          <p className="font-inter text-[12px] text-driftwood">{description}</p>
          <WriteStatusRegion status={action.status} successVerb={successVerb} />
        </form>
      </ConnectGate>
    </div>
  )
}

export default CloseControl
