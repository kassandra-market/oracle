import type { FormEvent } from 'react'
import type { Connection, TransactionInstruction } from '@solana/web3.js'
import { useWriteAction } from '../../../hooks/useWriteAction'
import { SubmitButton } from './formPrimitives'
import { WriteStatusRegion } from './WriteStatusRegion'

/**
 * RF2 — a per-participant CLAIM control (chestnut): pulls one staker's KASS
 * payout out of a Resolved oracle's vault into their canonical ATA + closes the
 * child account, via the wallet-backed sender + the shared write-status UX + a
 * refetch on success (the claimed account then disappears).
 *
 * Gating: a claim is signed by (and pays) a specific participant, so this renders
 * ONLY when a wallet is connected AND its address equals `authority` (the
 * proposer / fact submitter / voter who owns the payout). Any other viewer —
 * disconnected or a different wallet — sees nothing, so read-only browsing is
 * intact and other people's cards stay uncluttered. When `authority` is omitted
 * the control is self-scoped (any connected wallet — used for "claim your own
 * fact vote", where the FactVote PDA is derived from the connected key).
 */
export function ClaimControl({
  authority,
  verb,
  successVerb,
  description,
  build,
  refetch,
}: {
  /** The address permitted to claim (proposer.authority / fact.proposer). Omit for self-scoped. */
  authority?: string
  /** Idle button label, e.g. "Claim payout". */
  verb: string
  /** Past-tense confirmation verb, e.g. "Claimed". */
  successVerb: string
  /** Short helper line under the button. */
  description: string
  /** Assemble the claim ixs at click time (skipped under mock mode); receives the connected wallet. */
  build: (ctx: { address: string; connection: Connection }) => Promise<TransactionInstruction[]>
  refetch: () => void
}) {
  const action = useWriteAction(refetch)

  // Strict authority gate: nothing to render for a disconnected viewer or a
  // wallet that isn't the payout owner.
  if (!action.connected || action.address === null) return null
  if (authority !== undefined && action.address !== authority) return null

  const address = action.address
  const onSubmit = (e: FormEvent) => {
    e.preventDefault()
    void action.run(() => build({ address, connection: action.connection }))
  }

  return (
    <div className="mt-1 border-t border-pebble pt-3">
      <form className="flex flex-col gap-2" onSubmit={onSubmit} noValidate>
        <div className="flex items-center gap-3">
          <SubmitButton verb={verb} status={action.status} />
        </div>
        <p className="font-inter text-[12px] text-driftwood">{description}</p>
        <WriteStatusRegion status={action.status} successVerb={successVerb} />
      </form>
    </div>
  )
}

export default ClaimControl
