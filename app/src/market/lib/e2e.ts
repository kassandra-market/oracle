/**
 * E2E-mode detection. Gated ON in the Playwright browser suite (the Vite dev
 * server is started with `VITE_E2E=1`) or ad-hoc via a `?e2e` query param, so
 * {@link AppProviders} can swap the real WalletProvider for the injected
 * real-signing {@link E2eWalletProvider}. Never true in a normal build.
 */
export function isE2eMode(): boolean {
  if (import.meta.env.VITE_E2E === '1') return true
  return typeof window !== 'undefined' && new URLSearchParams(window.location.search).has('e2e')
}
