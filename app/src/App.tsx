import { Suspense, lazy } from 'react'
import { BrowserRouter, Routes, Route, Navigate } from 'react-router-dom'

import AppProviders from './providers/AppProviders.tsx'
import Layout from './components/layout/Layout.tsx'

// Route-level code-splitting: each page (+ its imported data/action/component
// code) becomes its own lazily-loaded chunk, so the entry only ships the shell
// (providers + NavBar) and the current route's chunk streams in on demand.
const Landing = lazy(() => import('./pages/Landing.tsx'))
const Oracles = lazy(() => import('./pages/Oracles.tsx'))
const CreateOracle = lazy(() => import('./pages/CreateOracle.tsx'))
const OracleDetail = lazy(() => import('./pages/OracleDetail.tsx'))
const Markets = lazy(() => import('./pages/Markets.tsx'))
const CreateMarket = lazy(() => import('./pages/CreateMarket.tsx'))
const MarketDetail = lazy(() => import('./pages/MarketDetail.tsx'))
const StyleGuide = lazy(() => import('./pages/StyleGuide.tsx'))
const Admin = lazy(() => import('./pages/Admin.tsx'))

/**
 * Quiet Auros placeholder shown while a route chunk streams in. On-brand
 * parchment tone (matches the "Reading the chain…" loading affordance) — a
 * transient, accessible status that the page replaces once loaded. Rendered
 * INSIDE the Layout's <Outlet>, so the NavBar/shell stays instant and only the
 * page content waits.
 */
function RouteFallback() {
  return (
    <main
      className="mx-auto flex max-w-[1200px] items-center justify-center px-6 py-24"
      role="status"
      aria-busy="true"
    >
      <p className="font-inter text-[15px] text-bronze">Consulting the oracle…</p>
    </main>
  )
}

/** Wrap a lazily-loaded page in the shared Auros Suspense fallback. */
function lazyRoute(element: React.ReactNode) {
  return <Suspense fallback={<RouteFallback />}>{element}</Suspense>
}

/**
 * App root: the provider shell + router. The NavBar/Layout render eagerly
 * (outside the Suspense boundary); only the routed page content lazy-loads.
 */
export default function App() {
  return (
    <AppProviders>
      <BrowserRouter>
        <Routes>
          <Route element={<Layout />}>
            <Route path="/" element={lazyRoute(<Landing />)} />
            <Route path="/oracles" element={lazyRoute(<Oracles />)} />
            <Route path="/oracles/new" element={lazyRoute(<CreateOracle />)} />
            <Route path="/oracles/:pubkey" element={lazyRoute(<OracleDetail />)} />
            <Route path="/markets" element={lazyRoute(<Markets />)} />
            <Route path="/markets/new" element={lazyRoute(<CreateMarket />)} />
            <Route path="/markets/:pubkey" element={lazyRoute(<MarketDetail />)} />
            <Route path="/styleguide" element={lazyRoute(<StyleGuide />)} />
            <Route path="/admin" element={lazyRoute(<Admin />)} />
            <Route path="*" element={<Navigate to="/" replace />} />
          </Route>
        </Routes>
      </BrowserRouter>
    </AppProviders>
  )
}
