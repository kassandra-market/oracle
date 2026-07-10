import { useState, type FormEvent } from 'react'
import type { Oracle } from '@kassandra-market/oracles'
import { Card } from '../../ui'
import { buildSubmitAiClaimIxs } from '../../../data/actions/challenge'
import { useWriteAction } from '../../../hooks/useWriteAction'
import { ConnectGate } from './ConnectGate'
import { Field, SubmitButton, TextInput } from './formPrimitives'
import { WriteStatusRegion } from './WriteStatusRegion'

/** Parse a 64-char hex string into 32 bytes, or return an error message. */
function parseHash32(raw: string, label: string): { value?: Uint8Array; error?: string } {
  const t = raw.trim().replace(/^0x/i, '')
  if (t === '') return { error: `Enter the ${label} (64 hex characters).` }
  if (!/^[0-9a-fA-F]{64}$/.test(t)) return { error: `${label} must be 64 hex characters (32 bytes).` }
  const bytes = new Uint8Array(32)
  for (let i = 0; i < 32; i++) bytes[i] = parseInt(t.slice(i * 2, i * 2 + 2), 16)
  return { value: bytes }
}

type Mode = 'fields' | 'paste'

/** Shape of the runner's emitted AI-claim payload (hex hashes + option). */
interface RunnerPayload {
  modelId?: string
  model_id?: string
  paramsHash?: string
  params_hash?: string
  ioHash?: string
  io_hash?: string
  option?: number
}

/** Read a hex field from a runner payload under either camel or snake case. */
function pick(o: RunnerPayload, camel: keyof RunnerPayload, snake: keyof RunnerPayload): string {
  const v = o[camel] ?? o[snake]
  return typeof v === 'string' ? v : ''
}

/**
 * RF4 — the AiClaim-phase submit form. A proposer stamps its AI claim: the pinned
 * model id, the params hash, and the input/output hash (the RUNNER produces these
 * three 32-byte hashes — paste them as hex, or paste the runner's JSON payload),
 * plus the claimed option. The submitter must be a proposer on this oracle; the
 * proposer PDA is derived from the connected wallet, so no address entry is
 * needed. Wraps {@link buildSubmitAiClaimIxs}.
 */
export function SubmitAiClaimForm({
  pubkey,
  oracle,
  refetch,
}: {
  pubkey: string
  oracle: Oracle
  refetch: () => void
}) {
  const action = useWriteAction(refetch)
  const [mode, setMode] = useState<Mode>('fields')
  const [modelHex, setModelHex] = useState('')
  const [paramsHex, setParamsHex] = useState('')
  const [ioHex, setIoHex] = useState('')
  const [option, setOption] = useState('0')
  const [paste, setPaste] = useState('')
  const [errors, setErrors] = useState<{ model?: string; params?: string; io?: string; option?: string; paste?: string }>({})

  const onSubmit = (e: FormEvent) => {
    e.preventDefault()
    const next: typeof errors = {}

    // In paste mode, decode the runner JSON into the hex fields first.
    let mHex = modelHex
    let pHex = paramsHex
    let iHex = ioHex
    let optRaw = option
    if (mode === 'paste') {
      try {
        const parsed = JSON.parse(paste) as RunnerPayload
        mHex = pick(parsed, 'modelId', 'model_id')
        pHex = pick(parsed, 'paramsHash', 'params_hash')
        iHex = pick(parsed, 'ioHash', 'io_hash')
        if (typeof parsed.option === 'number') optRaw = String(parsed.option)
      } catch {
        next.paste = 'Could not parse the runner payload as JSON.'
        setErrors(next)
        return
      }
    }

    const model = parseHash32(mHex, 'model id')
    const params = parseHash32(pHex, 'params hash')
    const io = parseHash32(iHex, 'I/O hash')
    if (model.error) next.model = model.error
    if (params.error) next.params = params.error
    if (io.error) next.io = io.error
    const optNum = Number(optRaw)
    if (!Number.isInteger(optNum) || optNum < 0 || optNum >= oracle.optionsCount) {
      next.option = `Option must be an integer in 0..${oracle.optionsCount - 1}.`
    }
    setErrors(next)
    if (Object.keys(next).length > 0) return

    void action.run(async () =>
      buildSubmitAiClaimIxs({
        oracle: pubkey,
        submitter: action.address!,
        modelId: model.value!,
        paramsHash: params.value!,
        ioHash: io.value!,
        option: optNum,
        optionsCount: oracle.optionsCount,
      }),
    )
  }

  const radioClass = (active: boolean) =>
    `rounded-tag border px-3 py-1.5 font-inter text-[13px] ${
      active ? 'border-chestnut bg-soft-cream text-chestnut' : 'border-pebble bg-pure-card text-driftwood'
    } focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sepia/40 focus-visible:ring-offset-2 focus-visible:ring-offset-parchment`

  return (
    <Card className="flex flex-col gap-4">
      <div>
        <h3 className="font-serif text-subheading font-light text-sepia">Submit an AI claim</h3>
        <p className="mt-1 font-inter text-[13px] text-driftwood">
          As a proposer, stamp your model’s verdict for this oracle. The runner produces the model,
          params and I/O hashes — paste them as hex, or paste the runner’s JSON payload.
        </p>
      </div>
      <ConnectGate connected={action.connected}>
        <form className="flex flex-col gap-4" onSubmit={onSubmit} noValidate>
          <div className="flex flex-col gap-2">
            <span className="font-inter text-[13px] font-medium text-sepia">Input mode</span>
            <div className="flex gap-2" role="radiogroup" aria-label="AI-claim input mode">
              <button
                type="button"
                role="radio"
                aria-checked={mode === 'fields'}
                onClick={() => setMode('fields')}
                className={radioClass(mode === 'fields')}
              >
                Enter hashes
              </button>
              <button
                type="button"
                role="radio"
                aria-checked={mode === 'paste'}
                onClick={() => setMode('paste')}
                className={radioClass(mode === 'paste')}
              >
                Paste runner output
              </button>
            </div>
          </div>

          {mode === 'paste' ? (
            <Field
              label="Runner payload (JSON)"
              hint="e.g. { &quot;model_id&quot;: &quot;…&quot;, &quot;params_hash&quot;: &quot;…&quot;, &quot;io_hash&quot;: &quot;…&quot;, &quot;option&quot;: 0 }"
              error={errors.paste}
            >
              {(ids) => (
                <textarea
                  id={ids.id}
                  aria-describedby={ids.describedById}
                  aria-invalid={ids.invalid}
                  rows={4}
                  placeholder='{ "model_id": "64 hex…", "params_hash": "64 hex…", "io_hash": "64 hex…", "option": 0 }'
                  value={paste}
                  onChange={(e) => setPaste(e.target.value)}
                  className="w-full rounded-tag border border-pebble bg-pure-card px-3 py-2 font-mono text-[12px] text-sepia placeholder:text-driftwood focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sepia/40 focus-visible:ring-offset-2 focus-visible:ring-offset-parchment aria-[invalid=true]:border-ember-orange/60"
                />
              )}
            </Field>
          ) : (
            <>
              <Field label="Model id (32-byte hex)" error={errors.model}>
                {(ids) => (
                  <TextInput ids={ids} placeholder="64 hex characters" value={modelHex} onChange={(e) => setModelHex(e.target.value)} className="font-mono" />
                )}
              </Field>
              <Field label="Params hash (32-byte hex)" error={errors.params}>
                {(ids) => (
                  <TextInput ids={ids} placeholder="64 hex characters" value={paramsHex} onChange={(e) => setParamsHex(e.target.value)} className="font-mono" />
                )}
              </Field>
              <Field label="I/O hash (32-byte hex)" error={errors.io}>
                {(ids) => (
                  <TextInput ids={ids} placeholder="64 hex characters" value={ioHex} onChange={(e) => setIoHex(e.target.value)} className="font-mono" />
                )}
              </Field>
            </>
          )}

          <Field label={`Option (0..${oracle.optionsCount - 1})`} error={errors.option}>
            {(ids) => (
              <TextInput ids={ids} inputMode="numeric" placeholder="e.g. 0" value={option} onChange={(e) => setOption(e.target.value)} />
            )}
          </Field>

          <div>
            <SubmitButton verb="Submit AI claim" status={action.status} />
          </div>
          <WriteStatusRegion status={action.status} successVerb="Submitted" />
        </form>
      </ConnectGate>
    </Card>
  )
}

export default SubmitAiClaimForm
