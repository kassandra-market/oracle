/**
 * CU3 — the React seam driving a STAGED, multi-tx compose→open SEQUENCE.
 *
 * The client-side challenge-market composition
 * ({@link buildComposeAndOpenChallengeIxs}) far exceeds one transaction, so it
 * returns an ORDERED list of {@link ComposeStep}s (question → 2 vaults →
 * fund+split → 2 pool seeds → open). This hook sends them as a SEQUENCE of
 * wallet-signed `sendAndConfirm` calls, tracking a per-step {@link StepStatus}
 * and, on a mid-sequence FAILURE, letting the caller RETRY from the failed step
 * (idempotent ATA-creates + the deterministic PDAs make a resume safe).
 *
 * It reuses the SAME wallet-adapter sender the single-write seam
 * ({@link useWriteAction}) uses; each step optionally prepends a
 * `setComputeUnitLimit` (the split / open steps CPI heavily). Under mock mode the
 * sends are skipped so the render harness can drive the staged progress.
 */
import { useCallback, useMemo, useState } from 'react'
import {
  ComputeBudgetProgram,
  Transaction,
  type TransactionInstruction,
} from '@solana/web3.js'
import { useWallet } from '@solana/wallet-adapter-react'
import { useConnection } from '../lib/cluster'
import { isMockMode } from '../data/mockOracles'
import { mockWriteConnection } from '../lib/mockWrite'
import { sendAndConfirm, type TxSender } from '../data/send'
import { mapWriteError } from '../data/writeAction'
import type { ComposeStep } from '../data/actions/challengeCompose'

/** The lifecycle of one step in the compose sequence. */
export type StepStatus =
  | { kind: 'pending' }
  | { kind: 'running' }
  | { kind: 'done'; signature: string }
  | { kind: 'error'; message: string; logs?: string[] }

/** True while any step is mid-flight. */
function isRunning(statuses: StepStatus[]): boolean {
  return statuses.some((s) => s.kind === 'running')
}

export interface ComposeSequence {
  /** Per-step status, index-aligned to the last-`prepare`d step list. */
  statuses: StepStatus[]
  /** Whether the sequence is currently sending a step. */
  busy: boolean
  /** Whether a wallet is connected (the form gates on this). */
  connected: boolean
  /** The connected wallet's base58 address (or null). */
  address: string | null
  /** The index of the first not-yet-done step (where a run/retry resumes). */
  resumeFrom: number
  /** Whether every step has completed. */
  allDone: boolean
  /**
   * Run the given ordered steps from `startIndex` (default: the first non-done),
   * stopping at the first failure. Safe to call again to retry from the failure.
   */
  run: (steps: ComposeStep[], startIndex?: number) => Promise<void>
  /** Reset all step statuses back to pending (e.g. re-compose). */
  reset: (count: number) => void
}

export function useComposeSequence(onDone?: () => void): ComposeSequence {
  const { connection: liveConnection } = useConnection()
  const { publicKey, connected, sendTransaction } = useWallet()
  const [statuses, setStatuses] = useState<StepStatus[]>([])

  const mock = isMockMode()
  const connection = useMemo(
    () => (mock ? mockWriteConnection() : liveConnection),
    [mock, liveConnection],
  )

  const walletSender = useMemo<TxSender | null>(() => {
    if (!connected || !publicKey) return null
    return async (ixs) => {
      const tx = new Transaction()
      for (const ix of ixs) tx.add(ix)
      return sendTransaction(tx, connection)
    }
  }, [connected, publicKey, sendTransaction, connection])

  const resumeFrom = useMemo(() => {
    const i = statuses.findIndex((s) => s.kind !== 'done')
    return i === -1 ? statuses.length : i
  }, [statuses])

  const allDone = statuses.length > 0 && statuses.every((s) => s.kind === 'done')

  const reset = useCallback((count: number) => {
    setStatuses(Array.from({ length: count }, () => ({ kind: 'pending' as const })))
  }, [])

  const run = useCallback(
    async (steps: ComposeStep[], startIndex?: number) => {
      if (isRunning(statuses)) return
      if (!walletSender) {
        setStatuses((prev) => {
          const next = prev.length === steps.length ? [...prev] : steps.map(() => ({ kind: 'pending' as const }))
          const at = startIndex ?? 0
          next[at] = { kind: 'error', message: 'Connect a wallet to compose the market.' }
          return next
        })
        return
      }

      // Seed / align the status array to the step list.
      let statusArr: StepStatus[] =
        statuses.length === steps.length
          ? [...statuses]
          : steps.map(() => ({ kind: 'pending' as const }))
      const from = startIndex ?? statusArr.findIndex((s) => s.kind !== 'done')
      const begin = from === -1 ? steps.length : from
      // Clear any prior error at/after the resume point.
      statusArr = statusArr.map((s, i) => (i >= begin && s.kind === 'error' ? { kind: 'pending' } : s))
      setStatuses(statusArr)

      for (let i = begin; i < steps.length; i++) {
        setStatuses((prev) => {
          const next = [...prev]
          next[i] = { kind: 'running' }
          return next
        })
        try {
          if (mock) {
            // Skip the real send in the render harness (fake keys aren't valid).
            await new Promise((r) => setTimeout(r, 0))
            setStatuses((prev) => {
              const next = [...prev]
              next[i] = { kind: 'done', signature: `mock-step-${i}` }
              return next
            })
            continue
          }
          const step = steps[i]
          const ixs: TransactionInstruction[] = step.computeUnits
            ? [ComputeBudgetProgram.setComputeUnitLimit({ units: step.computeUnits }), ...step.ixs]
            : step.ixs
          const { signature } = await sendAndConfirm(connection, walletSender, ixs)
          setStatuses((prev) => {
            const next = [...prev]
            next[i] = { kind: 'done', signature }
            return next
          })
        } catch (err) {
          const failure = mapWriteError(err)
          setStatuses((prev) => {
            const next = [...prev]
            next[i] = { kind: 'error', ...failure }
            return next
          })
          return // stop at the first failure; the caller can retry from here.
        }
      }
      onDone?.()
    },
    [statuses, walletSender, connection, mock, onDone],
  )

  return {
    statuses,
    busy: isRunning(statuses),
    connected,
    address: publicKey ? publicKey.toBase58() : null,
    resumeFrom,
    allDone,
    run,
    reset,
  }
}
