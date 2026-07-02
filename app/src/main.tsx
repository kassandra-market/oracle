import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { BrowserRouter, Routes, Route, Navigate } from 'react-router-dom'

// Local fonts (offline-safe — no hotlinked Google Fonts CDN).
// Cormorant Garamond 300/400 (display serif), Inter 400/500 (body/UI),
// Roboto Mono 400 (code accents). Only the weights we use.
import '@fontsource/cormorant-garamond/300.css'
import '@fontsource/cormorant-garamond/400.css'
import '@fontsource/inter/400.css'
import '@fontsource/inter/500.css'
import '@fontsource/roboto-mono/400.css'

import './index.css'
import AppProviders from './providers/AppProviders.tsx'
import Layout from './components/layout/Layout.tsx'
import Landing from './pages/Landing.tsx'
import StyleGuide from './pages/StyleGuide.tsx'
import Oracles from './pages/Oracles.tsx'
import OracleDetail from './pages/OracleDetail.tsx'

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <AppProviders>
      <BrowserRouter>
        <Routes>
          <Route element={<Layout />}>
            <Route path="/" element={<Landing />} />
            <Route path="/oracles" element={<Oracles />} />
            <Route path="/oracles/:pubkey" element={<OracleDetail />} />
            <Route path="/styleguide" element={<StyleGuide />} />
            <Route path="*" element={<Navigate to="/" replace />} />
          </Route>
        </Routes>
      </BrowserRouter>
    </AppProviders>
  </StrictMode>,
)
