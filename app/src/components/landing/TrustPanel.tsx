import { Card, EyebrowTag, Reveal } from '../ui'

const FLANK: { title: string; body: string }[] = [
  {
    title: 'Futarchy governance',
    body: 'MetaDAO markets set parameters and steer the treasury.',
  },
  {
    title: 'Challenge markets',
    body: 'Disputes become markets where honest capital is rewarded.',
  },
  {
    title: 'Bonds & slashing',
    body: 'Every claim is backed; wrong answers lose their bond.',
  },
  {
    title: 'Verifiable verdicts',
    body: 'Pinned model + committed hashes — reproduce it yourself.',
  },
]

function FlankCard({ title, body, delay = 0 }: { title: string; body: string; delay?: number }) {
  return (
    <Reveal delay={delay} className="h-full">
      <Card className="h-full transition-[transform,border-color] duration-200 hover:-translate-y-1 hover:border-cyan-phosphor/40">
        <h3 className="font-serif text-subheading font-light text-sepia">{title}</h3>
        <p className="mt-2 font-inter text-body text-bronze">{body}</p>
      </Card>
    </Reveal>
  )
}

/**
 * Trust / credibility — the Centered Portrait Panel pattern. A tall 16px-radius
 * centerpiece carries the ONE gradient permitted by the guide (a bioluminescent
 * teal orb with a lavender-phosphor edge) with a white role overlay near the
 * bottom that fades into the abyss canvas. Flanked by two stacked feature cards
 * on each side; on mobile everything collapses to a single column with the
 * portrait first.
 */
export default function TrustPanel() {
  return (
    <section aria-labelledby="trust-heading" className="px-6 py-20">
      <div className="mx-auto max-w-[1200px]">
        <h2 id="trust-heading" className="sr-only">
          Trust and credibility
        </h2>
        <div className="grid grid-cols-1 items-stretch gap-6 lg:grid-cols-[1fr_minmax(320px,380px)_1fr]">
          {/* Left flank (stacked) — appears after the portrait on mobile via order. */}
          <div className="order-2 grid grid-cols-1 gap-6 sm:grid-cols-2 lg:order-1 lg:grid-cols-1">
            <FlankCard {...FLANK[0]} delay={80} />
            <FlankCard {...FLANK[1]} delay={160} />
          </div>

          {/* Centerpiece portrait panel — the ONE allowed card gradient, now a
              bioluminescent teal orb with a lavender-phosphor edge (Auros). */}
          <Reveal className="order-1 lg:order-2">
            <div
              role="img"
              aria-label="The open-source resolver — anyone can run it"
              className="relative flex min-h-[440px] flex-col justify-between overflow-hidden rounded-card border border-pebble p-8 pb-16 lg:min-h-[520px]"
              style={{
                background:
                  'radial-gradient(90% 70% at 80% 10%, rgba(253,233,255,0.4) 0%, transparent 46%), ' +
                  'radial-gradient(120% 100% at 25% 20%, #23c3b2 0%, #0f7d72 32%, #0a4a44 66%, #063431 100%)',
              }}
            >
              <div className="relative z-10">
                <EyebrowTag className="!text-cyan-phosphor">Open source</EyebrowTag>
                <p className="mt-4 max-w-[16ch] font-serif text-heading-sm font-light leading-tight text-white">
                  Reproducible by anyone, trusted by no one.
                </p>
              </div>

              <div className="relative z-10">
                <p className="font-inter text-[15px] font-medium text-white">
                  The open-source resolver
                </p>
                <p className="mt-1 font-inter text-[13px] text-white/80">
                  Rerun the pinned model over the agreed facts and check the committed hashes
                  yourself.
                </p>
              </div>

              {/* Soft bottom fade merging into the (lightened) abyss canvas. */}
              <div
                aria-hidden="true"
                className="pointer-events-none absolute inset-x-0 bottom-0 z-0 h-12"
                style={{
                  background: 'linear-gradient(to top, #0b3f3a 0%, rgba(11,63,58,0) 100%)',
                }}
              />
            </div>
          </Reveal>

          {/* Right flank (stacked). */}
          <div className="order-3 grid grid-cols-1 gap-6 sm:grid-cols-2 lg:grid-cols-1">
            <FlankCard {...FLANK[2]} delay={80} />
            <FlankCard {...FLANK[3]} delay={160} />
          </div>
        </div>
      </div>
    </section>
  )
}
