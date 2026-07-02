import { KASSANDRA_PROGRAM_ID } from '@kassandra/sdk'
import {
  AvatarBubble,
  Button,
  Card,
  EyebrowTag,
  SectionHeader,
  TriggerPreviewCard,
} from '../components/ui'

/** Every Delphi color token: name + hex, in @theme order. */
const COLORS: { name: string; token: string; hex: string; dark?: boolean }[] = [
  { name: 'parchment', token: 'bg-parchment', hex: '#fdf6ee' },
  { name: 'soft-cream', token: 'bg-soft-cream', hex: '#f0e6dc' },
  { name: 'pure-card', token: 'bg-pure-card', hex: '#ffffff' },
  { name: 'ink-black', token: 'bg-ink-black', hex: '#000000', dark: true },
  { name: 'charcoal-bark', token: 'bg-charcoal-bark', hex: '#21201c', dark: true },
  { name: 'sepia', token: 'bg-sepia', hex: '#2b180a', dark: true },
  { name: 'bronze', token: 'bg-bronze', hex: '#7f6e60', dark: true },
  { name: 'driftwood', token: 'bg-driftwood', hex: '#94877c', dark: true },
  { name: 'stone', token: 'bg-stone', hex: '#a99d93', dark: true },
  { name: 'pebble', token: 'bg-pebble', hex: '#d9cfc3' },
  { name: 'chestnut', token: 'bg-chestnut', hex: '#3e2407', dark: true },
  { name: 'ember-orange', token: 'bg-ember-orange', hex: '#f65726', dark: true },
  { name: 'saffron-pulse', token: 'bg-saffron-pulse', hex: '#ff5c00', dark: true },
  { name: 'peach-glow', token: 'bg-peach-glow', hex: '#fed0b3' },
  { name: 'cobalt', token: 'bg-cobalt', hex: '#1da1f2', dark: true },
]

/** Type-scale roles. Serif is used ONLY for display >= 20px (subheading and up). */
const TYPE_SCALE: { role: string; cls: string; serif: boolean; px: string }[] = [
  { role: 'display', cls: 'text-display', serif: true, px: '64px' },
  { role: 'heading-lg', cls: 'text-heading-lg', serif: true, px: '56px' },
  { role: 'heading', cls: 'text-heading', serif: true, px: '40px' },
  { role: 'heading-sm', cls: 'text-heading-sm', serif: true, px: '24px' },
  { role: 'subheading', cls: 'text-subheading', serif: true, px: '20px' },
  { role: 'body', cls: 'text-body', serif: false, px: '15px' },
  { role: 'caption', cls: 'text-caption', serif: false, px: '10px' },
]

function Panel({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="mb-20">
      <h2 className="mb-6 font-serif text-heading-sm font-light text-sepia">{title}</h2>
      {children}
    </section>
  )
}

export default function StyleGuide() {
  return (
    <main className="mx-auto max-w-[1200px] px-6 py-16">
      <header className="mb-16 text-center">
        <EyebrowTag pill>Design System · U1</EyebrowTag>
        <h1 className="mt-4 font-serif text-display font-light text-sepia">Kassandra</h1>
        <p className="mx-auto mt-3 max-w-[560px] font-inter text-[17px] text-bronze">
          The Delphi visual language — warm parchment editorial with ember sparks. A living
          gallery of tokens and primitives.
        </p>
      </header>

      {/* Colors */}
      <Panel title="Color tokens">
        <div className="grid grid-cols-2 gap-4 sm:grid-cols-3 md:grid-cols-5">
          {COLORS.map((c) => (
            <div key={c.name} className="overflow-hidden rounded-tag border border-pebble">
              <div
                className={`${c.token} flex h-20 items-end p-2 ${c.dark ? 'text-white' : 'text-sepia'}`}
              >
                <span className="font-mono text-[11px]">{c.hex}</span>
              </div>
              <div className="bg-pure-card px-2 py-1.5 font-inter text-[12px] text-charcoal-bark">
                {c.name}
              </div>
            </div>
          ))}
        </div>
      </Panel>

      {/* Type scale */}
      <Panel title="Type scale">
        <div className="flex flex-col gap-5">
          {TYPE_SCALE.map((t) => (
            <div key={t.role} className="flex items-baseline gap-6 border-b border-pebble pb-4">
              <span className="w-28 shrink-0 font-mono text-[12px] text-driftwood">
                {t.role} · {t.px}
              </span>
              <span
                className={`${t.cls} ${t.serif ? 'font-serif font-light' : 'font-inter'} text-sepia`}
              >
                Truth, settled.
              </span>
            </div>
          ))}
        </div>
      </Panel>

      {/* Buttons */}
      <Panel title="Buttons">
        <div className="flex flex-wrap items-center gap-6">
          <Button variant="PrimaryChestnut">Propose an answer</Button>
          <Button variant="GhostOutline">Read the docs</Button>
          <Button variant="NavPill">Connect</Button>
          <Button variant="PrimaryChestnut" disabled>
            Disabled
          </Button>
        </div>
        <p className="mt-4 font-inter text-[13px] text-driftwood">
          PrimaryChestnut carries the signature peach <code className="font-mono">#fed0b3</code>{' '}
          bloom box-shadow — a warm glow radiating behind the fill, not a neutral drop shadow.
        </p>
      </Panel>

      {/* Cards + EyebrowTag */}
      <Panel title="Card + EyebrowTag">
        <div className="grid gap-6 md:grid-cols-2">
          <Card>
            <EyebrowTag>Optimistic resolution</EyebrowTag>
            <h3 className="mt-3 font-serif text-heading-sm font-light text-sepia">Propose &amp; challenge</h3>
            <p className="mt-2 font-inter text-body text-bronze">
              Anyone proposes an answer. A challenge window opens. If unchallenged, it settles.
            </p>
          </Card>
          <Card>
            <div className="flex gap-3">
              <EyebrowTag pill>Eyebrow · pill</EyebrowTag>
              <EyebrowTag>Eyebrow · bare</EyebrowTag>
            </div>
            <p className="mt-4 font-inter text-body text-bronze">
              Pure-card surface, 16px radius, 24px padding, a single 1px pebble hairline — flat, no
              heavy shadow.
            </p>
          </Card>
        </div>
      </Panel>

      {/* SectionHeader */}
      <Panel title="SectionHeader">
        <SectionHeader
          eyebrow="How it works"
          eyebrowPill
          line1="An optimistic oracle"
          line2="with a mind."
          paragraph="Propose an answer, open a challenge window, let an open-source runner rerun a pinned model over the agreed facts, then settle — all hashes committed on-chain."
        />
      </Panel>

      {/* Avatars */}
      <Panel title="AvatarBubble + VerifiedDot">
        <div className="flex flex-wrap items-center gap-10">
          <div className="text-center">
            <AvatarBubble name="Cassandra Vela" verified />
            <p className="mt-2 font-inter text-[12px] text-driftwood">placeholder + verified</p>
          </div>
          <div className="text-center">
            <AvatarBubble name="Milo Trent" />
            <p className="mt-2 font-inter text-[12px] text-driftwood">placeholder fallback</p>
          </div>
          <div className="text-center">
            <AvatarBubble
              name="Data feed"
              src="data:image/svg+xml;utf8,%3Csvg xmlns='http://www.w3.org/2000/svg' width='70' height='70'%3E%3Crect width='70' height='70' fill='%233e2407'/%3E%3Ccircle cx='35' cy='35' r='16' fill='%23f65726'/%3E%3C/svg%3E"
              verified
            />
            <p className="mt-2 font-inter text-[12px] text-driftwood">image src + verified</p>
          </div>
        </div>
      </Panel>

      {/* TriggerPreviewCard */}
      <Panel title="TriggerPreviewCard">
        <div className="max-w-[420px]">
          <TriggerPreviewCard
            condition="Proposed answer is challenged before"
            variable="challenge_window_ends"
          />
        </div>
      </Panel>

      <footer className="mt-16 border-t border-pebble pt-6 font-mono text-[12px] text-driftwood">
        @kassandra/sdk workspace import resolves · KASSANDRA_PROGRAM_ID ={' '}
        {KASSANDRA_PROGRAM_ID.toString()}
      </footer>
    </main>
  )
}
