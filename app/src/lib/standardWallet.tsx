/**
 * Real-wallet integration on the **Wallet Standard** (`@wallet-standard/react`),
 * replacing the legacy `@solana/wallet-adapter` `StandardWalletAdapter` — which is
 * broken here: built against `@solana/web3.js@3.x` while it peer-requires 1.x, it
 * hands Phantom a malformed request and Phantom throws "Unexpected error" on
 * connect.
 *
 * We drive the wallet directly: list wallets, connect an account (per-wallet
 * `useConnect` hook, in the modal rows), and sign with the account's
 * `solana:signTransaction` feature. The app builds a classic `web3.js`
 * transaction and SENDS it over its OWN RPC connection (so it works against a
 * local surfpool RPC without needing the wallet's network configured for send).
 *
 * The provider exposes the SAME `WalletContextState` the app already consumes via
 * `useWallet()`, so nothing downstream changes; the picker is a small custom
 * modal (`useWalletMenu`) since the wallet-adapter-react-ui modal is tied to the
 * legacy adapter.
 */
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from 'react'
import { WalletContext, type WalletContextState } from '@solana/wallet-adapter-react'
import { Address, Transaction, type Connection } from '@solana/web3.js'
import {
  getWalletAccountFeature,
  getWalletFeature,
  useWallets,
  type UiWallet,
  type UiWalletAccount,
} from '@wallet-standard/react'
import { SolanaSignTransaction } from '@solana/wallet-standard-features'

/** LocalStorage key remembering the last connected wallet (name + account) so a
 *  page refresh re-adopts it instead of dropping to disconnected. */
const LAST_WALLET_KEY = 'kassandra:lastWallet'

interface StoredWallet {
  name: string
  address: string
}

function readLastWallet(): StoredWallet | null {
  try {
    const raw = localStorage.getItem(LAST_WALLET_KEY)
    if (!raw) return null
    const v = JSON.parse(raw) as StoredWallet
    return v && typeof v.name === 'string' && typeof v.address === 'string' ? v : null
  } catch {
    return null
  }
}

function writeLastWallet(v: StoredWallet | null): void {
  try {
    if (v) localStorage.setItem(LAST_WALLET_KEY, JSON.stringify(v))
    else localStorage.removeItem(LAST_WALLET_KEY)
  } catch {
    /* localStorage unavailable (private mode / SSR) — persistence is best-effort. */
  }
}

/** What the mount-time restore should do next, given the remembered wallet, the
 *  currently-registered wallets, and whether we've already tried a silent connect.
 *  Pure (no localStorage / effects) so it's unit-testable. */
export type ReconnectAction =
  | { kind: 'adopt'; account: UiWalletAccount }
  | { kind: 'silent'; wallet: UiWallet }
  | { kind: 'none' }

export function resolveReconnect(
  stored: StoredWallet | null,
  wallets: readonly UiWallet[],
  triedSilent: boolean,
): ReconnectAction {
  if (!stored) return { kind: 'none' }
  const owner = wallets.find((w) => w.name === stored.name)
  if (!owner) return { kind: 'none' }
  // The remembered account is already exposed → adopt it directly.
  const exact = owner.accounts.find((a) => a.address === stored.address)
  if (exact) return { kind: 'adopt', account: exact }
  // Present but no accounts yet → one silent reconnect attempt.
  if (!triedSilent) return { kind: 'silent', wallet: owner }
  // Silent reconnect already ran → adopt whatever the wallet now authorizes.
  if (owner.accounts[0]) return { kind: 'adopt', account: owner.accounts[0] }
  return { kind: 'none' }
}

/** Ask a wallet to reconnect WITHOUT a prompt (returns already-authorized accounts
 *  only). Used on mount to restore the last session; a wallet that no longer
 *  authorizes us simply yields nothing and we stay disconnected. */
async function silentConnect(wallet: UiWallet): Promise<void> {
  if (!(wallet.features as readonly string[]).includes('standard:connect')) return
  try {
    const feature = getWalletFeature(wallet, 'standard:connect') as {
      connect: (input?: { silent?: boolean }) => Promise<unknown>
    }
    await feature.connect({ silent: true })
  } catch {
    /* Not authorized / wallet declined — remain disconnected. */
  }
}

/** Pick a Solana chain the account advertises (any — the app sends over its own RPC). */
function solanaChain(account: UiWalletAccount): `solana:${string}` {
  const c = account.chains.find((x): x is `solana:${string}` => x.startsWith('solana:'))
  return c ?? 'solana:mainnet'
}

/**
 * Sign a prepared (feePayer + blockhash already set) legacy transaction with the
 * account's Wallet-Standard `solana:signTransaction` feature, returning the fully
 * signed WIRE bytes. Shared by both write paths: the oracle `sendTransaction`
 * (relays these bytes over its RPC) and the markets `signTransaction` (rehydrates
 * a `Transaction` the indexer relay serializes + submits).
 */
async function signWithAccount(account: UiWalletAccount, tx: Transaction): Promise<Uint8Array> {
  const wire = await tx.serialize({ requireAllSignatures: false, verifySignatures: false })
  // NOTE (needs live-wallet validation): the exact Wallet-Standard
  // `signTransaction` input shape (esp. the `account` — UiWalletAccount vs the
  // underlying WalletAccount) is the one bit we can't verify without a browser
  // wallet, so the feature is typed permissively here.
  const getFeature = getWalletAccountFeature as (a: UiWalletAccount, f: string) => unknown
  const feature = getFeature(account, SolanaSignTransaction) as {
    signTransaction: (input: unknown) => Promise<ReadonlyArray<{ signedTransaction: Uint8Array }>>
  }
  const [{ signedTransaction }] = await feature.signTransaction({
    account,
    transaction: wire,
    chain: solanaChain(account),
  })
  return signedTransaction
}

interface WalletMenuValue {
  wallets: readonly UiWallet[]
  open: boolean
  setOpen: (o: boolean) => void
  /** Called by a modal row after a successful per-wallet connect. */
  adopt: (account: UiWalletAccount) => void
}
/** No-op default so NavBar / ConnectGate (rendered in every wallet mode) can call
 *  `openPicker` unconditionally; in E2E/mock modes the wallet is auto-connected,
 *  so the picker is never actually needed. */
const NOOP_MENU: WalletMenuValue = { wallets: [], open: false, setOpen: () => {}, adopt: () => {} }
const WalletMenuContext = createContext<WalletMenuValue>(NOOP_MENU)
export function useWalletMenu(): WalletMenuValue {
  return useContext(WalletMenuContext)
}

export function StandardWalletProvider({ children }: { children: ReactNode }) {
  const wallets = useWallets()
  const [account, setAccount] = useState<UiWalletAccount | null>(null)
  const [open, setOpen] = useState(false)

  const publicKey = useMemo(() => (account ? new Address(account.address) : null), [account])

  // Adopt a connected account AND remember it, so a refresh restores the session.
  const adopt = useCallback(
    (acct: UiWalletAccount) => {
      setAccount(acct)
      const owner = wallets.find((w) => w.accounts.some((a) => a.address === acct.address))
      writeLastWallet({ name: owner?.name ?? '', address: acct.address })
    },
    [wallets],
  )

  // Restore the last session on mount / once the remembered wallet registers.
  // Wallets register asynchronously (and after a refresh a previously-authorized
  // wallet may need a silent reconnect before it re-exposes its accounts), so this
  // re-runs as `wallets` updates until it can adopt an account. `triedSilent` bounds
  // us to a single silent-connect attempt per page load.
  const triedSilent = useRef(false)
  useEffect(() => {
    if (account) return
    const action = resolveReconnect(readLastWallet(), wallets, triedSilent.current)
    if (action.kind === 'adopt') {
      setAccount(action.account)
    } else if (action.kind === 'silent') {
      triedSilent.current = true
      void silentConnect(action.wallet)
    }
  }, [wallets, account])

  // The oracle write path: sign locally, then SEND over the passed RPC connection.
  const sendTransaction = useCallback(
    async (tx: Transaction, connection: Connection): Promise<string> => {
      if (!account || !publicKey) throw new Error('wallet not connected')
      tx.feePayer = publicKey
      tx.recentBlockhash = (await connection.getLatestBlockhash()).blockhash
      const signed = await signWithAccount(account, tx)
      return connection.sendRawTransaction(signed, { skipPreflight: false })
    },
    [account, publicKey],
  )

  // The markets write path (`signAndRelay`): sign a prepared (feePayer + blockhash
  // already set) tx locally and hand the signed `Transaction` back — the indexer
  // relay serializes + submits it. Without this, the markets `useWriteAction` gate
  // (`!signTransaction → null sender`) misreports a connected wallet as "Connect a
  // wallet to participate." on every trade.
  const signTransaction = useCallback(
    async (tx: Transaction): Promise<Transaction> => {
      if (!account || !publicKey) throw new Error('wallet not connected')
      return Transaction.from(await signWithAccount(account, tx))
    },
    [account, publicKey],
  )

  const disconnect = useCallback(async () => {
    const wallet = wallets.find((w) => w.accounts.some((a) => a.address === account?.address))
    const feat = (wallet?.features as Record<string, { disconnect?: () => Promise<void> }> | undefined)?.[
      'standard:disconnect'
    ]
    await feat?.disconnect?.().catch(() => {})
    setAccount(null)
    writeLastWallet(null)
  }, [wallets, account])

  const value = useMemo<WalletContextState>(
    () =>
      ({
        autoConnect: false,
        wallets: [],
        wallet: null,
        publicKey,
        connecting: false,
        connected: account !== null,
        disconnecting: false,
        select: () => setOpen(true),
        connect: async () => setOpen(true),
        disconnect,
        sendTransaction: sendTransaction as unknown as WalletContextState['sendTransaction'],
        signTransaction: signTransaction as unknown as WalletContextState['signTransaction'],
        signAllTransactions: undefined,
        signMessage: undefined,
        signIn: undefined,
      }) as unknown as WalletContextState,
    [publicKey, account, disconnect, sendTransaction, signTransaction],
  )

  const menu = useMemo<WalletMenuValue>(
    () => ({ wallets, open, setOpen, adopt }),
    [wallets, open, adopt],
  )

  return (
    <WalletContext.Provider value={value}>
      <WalletMenuContext.Provider value={menu}>{children}</WalletMenuContext.Provider>
    </WalletContext.Provider>
  )
}
