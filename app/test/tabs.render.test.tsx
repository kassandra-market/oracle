/**
 * Render coverage for the Auros Tabs primitive. Uses `renderToStaticMarkup` to
 * assert the accessible tablist wiring (roles, aria-selected, the tab↔panel
 * id/aria-controls linkage, roving tabindex) and that TabPanel renders ONLY the
 * active panel's content. Interaction (keyboard/click switching) is exercised by
 * the pages that consume it.
 */
import React from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'

import { Tabs, TabPanel } from '../src/components/ui/Tabs'

const ITEMS = [
  { id: 'overview', label: 'Overview' },
  { id: 'records', label: 'Records', count: 3 },
  { id: 'market', label: 'Market', dot: 'aqua' as const },
]

function render(active: string): string {
  return renderToStaticMarkup(
    <>
      <Tabs items={ITEMS} value={active} onChange={() => {}} ariaLabel="Test sections" />
      <TabPanel id="overview" active={active === 'overview'}>
        overview-body
      </TabPanel>
      <TabPanel id="records" active={active === 'records'}>
        records-body
      </TabPanel>
      <TabPanel id="market" active={active === 'market'}>
        market-body
      </TabPanel>
    </>,
  )
}

describe('Tabs', () => {
  it('renders an accessible tablist with one selected tab', () => {
    const html = render('overview')
    expect(html).toContain('role="tablist"')
    expect(html).toContain('aria-label="Test sections"')
    // The selected tab is marked and links to its panel.
    expect(html).toMatch(/id="tab-overview"[^>]*aria-selected="true"/)
    expect(html).toMatch(/aria-controls="panel-overview"/)
    // Non-active tabs are not selected.
    expect(html).toMatch(/id="tab-records"[^>]*aria-selected="false"/)
  })

  it('applies roving tabindex (only the active tab is focusable)', () => {
    const html = render('records')
    expect(html).toMatch(/id="tab-records"[^>]*tabindex="0"/)
    expect(html).toMatch(/id="tab-overview"[^>]*tabindex="-1"/)
  })

  it('renders a count badge and an accent dot where provided', () => {
    const html = render('overview')
    expect(html).toContain('>3<') // Records count badge
    expect(html).toContain('bg-aqua') // Market accent dot / active underline
  })

  it('renders only the active panel body', () => {
    const html = render('records')
    expect(html).toContain('records-body')
    expect(html).not.toContain('overview-body')
    expect(html).not.toContain('market-body')
    // The rendered panel carries its aria linkage back to the tab.
    expect(html).toMatch(/role="tabpanel"[^>]*id="panel-records"/)
    expect(html).toMatch(/aria-labelledby="tab-records"/)
  })
})
