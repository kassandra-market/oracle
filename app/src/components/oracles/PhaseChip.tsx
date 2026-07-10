import type { Phase } from '@kassandra-market/oracles'
import { Chip } from './Chip'
import { phaseView } from '../../lib/oracleView'

/**
 * The oracle lifecycle-phase status chip. Maps {@link Phase} → a readable label
 * + an on-brand tone (see `phaseView`); the active Challenge is the one ember
 * spark. The label is real text and an `aria-label` names it as the phase.
 */
export function PhaseChip({ phase }: { phase: Phase | undefined }) {
  const { label, tone } = phaseView(phase)
  return (
    <Chip tone={tone} aria-label={`Phase: ${label}`}>
      {label}
    </Chip>
  )
}

export default PhaseChip
