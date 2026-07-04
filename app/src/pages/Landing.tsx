import Hero from '../components/landing/Hero'
import HowItWorks from '../components/landing/HowItWorks'
import WhyKassandra from '../components/landing/WhyKassandra'
import TrustPanel from '../components/landing/TrustPanel'

/**
 * The Kassandra landing page — composed entirely from the U1 Auros primitives.
 * The NavBar + SiteFooter chrome is provided by the shared Layout shell; this
 * page renders the editorial main content:
 * hero constellation → how it works → why → trust portrait.
 */
export default function Landing() {
  return (
    <main>
      <Hero />
      <HowItWorks />
      <WhyKassandra />
      <TrustPanel />
    </main>
  )
}
