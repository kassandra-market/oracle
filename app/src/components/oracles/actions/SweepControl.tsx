import { useState, type FormEvent } from 'react'
import type { Oracle } from '@kassandra-market/oracles'
import { Card } from '../../ui'
import { relativeDeadline } from '../../../lib/oracleView'
import { recallNonce } from '../../../lib/nonceStore'
import { resolveOracleNonce } from '../../../data/actions/finalize'
import { buildSweepOracleIxs, resolveDaoAuthority } from '../../../data/actions/claims'
import { useWriteAction } from '../../../hooks/useWriteAction'
import { ConnectGate } from './ConnectGate'
import { SubmitButton } from './formPrimitives'
import { WriteStatusRegion } from './WriteStatusRegion'

/** 30-day sweep grace (config.rs SWEEP_GRACE = 30·24·60·60). */
const SWEEP_GRACE = 30n * 24n * 60n * 60n

/**
 * RF2 — the permissionless, grace-gated SWEEP control: after the 30-day grace on
 * a terminal oracle, any wallet can sweep the residual vault dust (or a no-show
 * staker's forfeited principal) to the DAO treasury and CLOSE the stake-vault +
 * Oracle, refunding both rents to the creator. Only meaningful once governance is
 * set (the treasury is `ATA(dao_authority, kass_mint)`), so the build resolves
 * the DAO authority from the Protocol singleton at click time and surfaces a
 * clear error if governance isn't linked yet.
 *
 * Before the grace elapses the button is withheld with a note showing when the
 * sweep opens; the underlying `sweep_oracle` would otherwise fail the grace guard.
 */
export function SweepControl({
  oracle,
  oracleAccount,
  refetch,
}: {
  /** The oracle PDA (base58). */
  oracle: string
  /** The decoded oracle (for `phase_ends_at`, `creator`, `kass_mint`). */
  oracleAccount: Oracle
  refetch: () => void
}) {
  const action = useWriteAction(refetch)
  const [error, setError] = useState<string | undefined>()

  const graceEndsAt = oracleAccount.phaseEndsAt + SWEEP_GRACE
  const nowUnix = BigInt(Math.floor(Date.now() / 1000))
  const graceElapsed = nowUnix >= graceEndsAt

  const onSubmit = (e: FormEvent) => {
    e.preventDefault()
    setError(undefined)
    void action.run(async () => {
      const conn = action.connection
      const nonce = recallNonce(oracle) ?? (await resolveOracleNonce(oracle))
      // The DAO authority (treasury owner) lives on the Protocol singleton.
      const daoAuthority = await resolveDaoAuthority(conn)
      return buildSweepOracleIxs({
        oracleNonce: nonce,
        kassMint: oracleAccount.kassMint,
        daoAuthority,
        creator: oracleAccount.creator,
      })
    })
  }

  return (
    <Card className="flex flex-col gap-4">
      <div>
        <h3 className="font-serif text-subheading font-light text-sepia">Sweep oracle</h3>
        <p className="mt-1 font-inter text-[13px] text-driftwood">
          Once the grace period has passed, sweep the residual vault balance to the DAO treasury and
          close the oracle, refunding its rent to the creator.
        </p>
        <p className="mt-1 font-inter text-[12px] text-driftwood">
          Permissionless — any connected wallet can run this; it only pays the fee.
        </p>
      </div>
      {!graceElapsed ? (
        <div className="rounded-tag border border-pebble bg-soft-cream px-3 py-2">
          <p className="font-inter text-[13px] text-bronze">
            The sweep opens after the 30-day grace ({relativeDeadline(graceEndsAt)}).
          </p>
        </div>
      ) : (
        <ConnectGate connected={action.connected}>
          <form className="flex flex-col gap-3" onSubmit={onSubmit} noValidate>
            <div className="flex items-center gap-3">
              <SubmitButton verb="Sweep oracle" status={action.status} />
            </div>
            {error ? (
              <p className="font-inter text-[12px] text-ember-orange">{error}</p>
            ) : null}
            <WriteStatusRegion status={action.status} successVerb="Swept" />
          </form>
        </ConnectGate>
      )}
    </Card>
  )
}

export default SweepControl
