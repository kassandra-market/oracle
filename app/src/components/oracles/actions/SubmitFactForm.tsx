import { useState, type FormEvent } from 'react'
import type { Oracle } from '@kassandra-market/oracles'
import { Card } from '../../ui'
import { buildSubmitFactIxs, hashToContentHash } from '../../../data/actions'
import { useWriteAction } from '../../../hooks/useWriteAction'
import { ConnectGate } from './ConnectGate'
import { Field, SubmitButton, TextInput } from './formPrimitives'
import { WriteStatusRegion } from './WriteStatusRegion'
import { parseAmount, balanceGateError } from './amount'
import { useKassBalance } from '../../../hooks/useKassBalance'
import { KassBalanceLine } from './kassBalance'

const enc = new TextEncoder()

/** Parse a 64-char hex string into 32 bytes, or return an error message. */
function parseHash32(raw: string): { value?: Uint8Array; error?: string } {
  const t = raw.trim().replace(/^0x/i, '')
  if (t === '') return { error: 'Enter a 32-byte hex hash, or switch to hashing text.' }
  if (!/^[0-9a-fA-F]{64}$/.test(t)) return { error: 'Content hash must be 64 hex characters (32 bytes).' }
  const bytes = new Uint8Array(32)
  for (let i = 0; i < 32; i++) bytes[i] = parseInt(t.slice(i * 2, i * 2 + 2), 16)
  return { value: bytes }
}

type HashMode = 'text' | 'hex'

/**
 * Submit a fact: a content hash (hash pasted text OR paste a 32-byte hex hash),
 * an off-chain uri (<=200 bytes), and an escrowed KASS stake (FactProposal
 * phase only). Wraps WF1 `buildSubmitFactIxs`.
 */
export function SubmitFactForm({
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
  const [mode, setMode] = useState<HashMode>('text')
  const [text, setText] = useState('')
  const [hex, setHex] = useState('')
  const [uri, setUri] = useState('')
  const [stake, setStake] = useState('')
  const [errors, setErrors] = useState<{ hash?: string; uri?: string; stake?: string }>({})

  const uriBytes = enc.encode(uri).length
  const balanceError = balanceGateError(parseAmount(stake).value, balance, 'stake')

  const onSubmit = (e: FormEvent) => {
    e.preventDefault()
    const next: { hash?: string; uri?: string; stake?: string } = {}
    const stakeParsed = parseAmount(stake)
    if (stakeParsed.error) next.stake = stakeParsed.error
    if (uriBytes > 200) next.uri = `URI is ${uriBytes} bytes (max 200).`
    let contentHash: Uint8Array | undefined
    if (mode === 'text') {
      if (text.trim() === '') next.hash = 'Enter the fact text to hash.'
    } else {
      const parsed = parseHash32(hex)
      if (parsed.error) next.hash = parsed.error
      else contentHash = parsed.value
    }
    setErrors(next)
    if (Object.keys(next).length > 0) return

    void action.run(async () => {
      const hash = mode === 'text' ? await hashToContentHash(text) : contentHash!
      return buildSubmitFactIxs({
        connection: action.connection,
        oracle: pubkey,
        kassMint: oracle.kassMint,
        submitter: action.address!,
        contentHash: hash,
        stake: stakeParsed.value!,
        uri,
      })
    })
  }

  const radioClass = (active: boolean) =>
    `rounded-tag border px-3 py-1.5 font-inter text-[13px] ${
      active ? 'border-chestnut bg-soft-cream text-chestnut' : 'border-pebble bg-pure-card text-driftwood'
    } focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sepia/40 focus-visible:ring-offset-2 focus-visible:ring-offset-parchment`

  return (
    <Card className="flex flex-col gap-4">
      <div>
        <h3 className="font-serif text-subheading font-light text-sepia">Submit a fact</h3>
        <p className="mt-1 font-inter text-[13px] text-driftwood">
          Stake KASS behind a fact. The content hash seeds the fact PDA; the URI points at the
          off-chain evidence.
        </p>
      </div>
      <ConnectGate connected={action.connected}>
        <form className="flex flex-col gap-4" onSubmit={onSubmit} noValidate>
          <div className="flex flex-col gap-2">
            <span className="font-inter text-[13px] font-medium text-sepia">Content hash source</span>
            <div className="flex gap-2" role="radiogroup" aria-label="Content hash source">
              <button
                type="button"
                role="radio"
                aria-checked={mode === 'text'}
                onClick={() => setMode('text')}
                className={radioClass(mode === 'text')}
              >
                Hash text
              </button>
              <button
                type="button"
                role="radio"
                aria-checked={mode === 'hex'}
                onClick={() => setMode('hex')}
                className={radioClass(mode === 'hex')}
              >
                Paste 32-byte hex
              </button>
            </div>
          </div>

          {mode === 'text' ? (
            <Field
              label="Fact text"
              hint="SHA-256 of this text becomes the 32-byte content hash."
              error={errors.hash}
            >
              {(ids) => (
                <TextInput
                  ids={ids}
                  placeholder="The claim to record"
                  value={text}
                  onChange={(e) => setText(e.target.value)}
                />
              )}
            </Field>
          ) : (
            <Field label="Content hash (32-byte hex)" error={errors.hash}>
              {(ids) => (
                <TextInput
                  ids={ids}
                  placeholder="64 hex characters"
                  value={hex}
                  onChange={(e) => setHex(e.target.value)}
                  className="font-mono"
                />
              )}
            </Field>
          )}

          <Field
            label="URI (off-chain evidence)"
            hint={`${uriBytes}/200 bytes`}
            error={errors.uri}
          >
            {(ids) => (
              <TextInput
                ids={ids}
                placeholder="ipfs://… or https://…"
                value={uri}
                onChange={(e) => setUri(e.target.value)}
                className="font-mono"
              />
            )}
          </Field>

          <Field label="Stake (KASS base units)" error={errors.stake ?? balanceError}>
            {(ids) => (
              <TextInput
                ids={ids}
                inputMode="numeric"
                placeholder="e.g. 1000000000"
                value={stake}
                onChange={(e) => setStake(e.target.value)}
              />
            )}
          </Field>
          <KassBalanceLine balance={balance} loading={balanceLoading} />

          <div>
            <SubmitButton verb="Submit fact" status={action.status} disabled={Boolean(balanceError)} />
          </div>
          <WriteStatusRegion status={action.status} successVerb="Submitted" />
        </form>
      </ConnectGate>
    </Card>
  )
}

export default SubmitFactForm
