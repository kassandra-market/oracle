import { Outlet } from 'react-router-dom'
import NavBar from '../landing/NavBar'
import SiteFooter from '../landing/SiteFooter'

/**
 * Shared app shell — the Auros NavBar (with the real wallet connect + cluster
 * selector) above the routed page, with the SiteFooter beneath. Wraps every
 * route so the chrome is consistent across the landing, styleguide, and the
 * oracle browse views.
 */
export default function Layout() {
  return (
    <div className="flex min-h-screen flex-col bg-parchment">
      <NavBar />
      {/* Each routed page owns its own <main> landmark. */}
      <div className="flex-1">
        <Outlet />
      </div>
      <SiteFooter />
    </div>
  )
}
