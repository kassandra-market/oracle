import { useState, type FormEvent } from 'react'
import type { Oracle } from '@kassandra-market/oracles'
import { Card } from '../../ui'
import { buildProposeIxs } from '../../../data/actions'
import { useWriteAction } from '../../../hooks/useWriteAction'
import { ConnectGate } from './ConnectGate'
import { Field, SubmitButton, TextInput } from './formPrimitives'
import { WriteStatusRegion } from './WriteStatusRegion'
import { parseAmount, balanceGateError } from './amount'
import { useKassBalance } from '../../../hooks/useKassBalance'
import { KassBalanceLine } from './kassBalance'

/**
 * Propose a categorical option + escrow a KASS bond (Proposal phase only).
 * Wraps WF1 `buildProposeIxs` via the wallet-backed sender.
 */
export function ProposeForm({
  pubkey,
  oracle,
  refetch,
}: {
  pubkey: string
  oracle: Oracle
  refetch: () => void
}) {
  const { balance, loading: balanceLoading, refetch: refetchBalance } = useKassBalance(
    String(oracle.kassMint),
  )
  const action = useWriteAction(() => {
    refetch()
    refetchBalance()
  })
  const [option, setOption] = useState(0)
  const [bond, setBond] = useState('')
  const [bondError, setBondError] = useState<string | undefined>()
  const balanceError = balanceGateError(parseAmount(bond).value, balance, 'bond')

  const onSubmit = (e: FormEvent) => {
    e.preventDefault()
    const parsed = parseAmount(bond)
    if (parsed.error) {
      setBondError(parsed.error)
      return
    }
    setBondError(undefined)
    void action.run(() =>
      buildProposeIxs({
        connection: action.connection,
        oracle: pubkey,
        kassMint: oracle.kassMint,
        authority: action.address!,
        option,
        bond: parsed.value!,
        optionsCount: oracle.optionsCount,
      }),
    )
  }

  return (
    <Card className="flex flex-col gap-4">
      <div>
        <h3 className="font-serif text-subheading font-light text-sepia">Propose an option</h3>
        <p className="mt-1 font-inter text-[13px] text-driftwood">
          Escrow a KASS bond behind the option you believe resolves this dispute.
        </p>
      </div>
      <ConnectGate connected={action.connected}>
        <form className="flex flex-col gap-4" onSubmit={onSubmit} noValidate>
          <Field label="Option">
            {(ids) => (
              <select
                id={ids.id}
                aria-describedby={ids.describedById}
                value={option}
                onChange={(e) => setOption(Number(e.target.value))}
                className="w-full rounded-tag border border-pebble bg-pure-card px-3 py-2 font-inter text-[14px] text-sepia focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sepia/40 focus-visible:ring-offset-2 focus-visible:ring-offset-parchment"
              >
                {Array.from({ length: oracle.optionsCount }, (_, i) => (
                  <option key={i} value={i}>
                    Option {i}
                  </option>
                ))}
              </select>
            )}
          </Field>
          <Field
            label="Bond (KASS base units)"
            hint="Raw, unscaled — as shown in the bond pool above."
            error={bondError ?? balanceError}
          >
            {(ids) => (
              <TextInput
                ids={ids}
                inputMode="numeric"
                placeholder="e.g. 5000000000"
                value={bond}
                onChange={(e) => setBond(e.target.value)}
              />
            )}
          </Field>
          <KassBalanceLine balance={balance} loading={balanceLoading} />
          <div className="flex items-center gap-3">
            <SubmitButton verb="Propose" status={action.status} disabled={Boolean(balanceError)} />
          </div>
          <WriteStatusRegion status={action.status} successVerb="Proposed" />
        </form>
      </ConnectGate>
    </Card>
  )
}

export default ProposeForm
