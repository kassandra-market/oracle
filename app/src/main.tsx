import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'

// Local fonts (offline-safe — no hotlinked Google Fonts CDN).
// Cormorant Garamond 300/400 (display serif), Inter 400/500 (body/UI),
// Roboto Mono 400 (code accents). Only the weights we use.
import '@fontsource/cormorant-garamond/300.css'
import '@fontsource/cormorant-garamond/400.css'
import '@fontsource/inter/400.css'
import '@fontsource/inter/500.css'
import '@fontsource/roboto-mono/400.css'

// Wallet-adapter base styles — loaded here (as their own module) rather than
// `@import`ed from index.css, so the stylesheet's leading Google-Fonts `@import`
// stays valid (see the note in index.css). Imported BEFORE index.css so our
// Auros overrides there win the cascade.
import '@solana/wallet-adapter-react-ui/styles.css'
import './index.css'
import App from './App.tsx'

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <App />
  </StrictMode>,
)
