import type { CSSProperties, ReactNode } from 'react'
import { AvatarBubble, Button, Reveal } from '../ui'
import { usePointerField } from '../../hooks/usePointerField'

interface QuestionCardData {
  name: string
  role: string
  verified?: boolean
  question: string
  /** A short verdict / confidence line; the punchy fragment is highlighted in ember. */
  verdict?: ReactNode
  /** Desktop-only absolute placement (the scatter). Ignored in the mobile flow grid. */
  pos: string
  /** Parallax depth in px — how far this card drifts toward the cursor (foreground = larger). */
  depth: number
}

/**
 * The "orbiting voices" of the constellation — living-looking oracle questions
 * and mini verdicts. Each is a flat card (16px radius, hairline border): an
 * AvatarBubble + a name/role line + a one-line question + an optional verdict
 * with the punchy word/number in the lavender-phosphor accent. Each carries a
 * `depth` so the cards drift at different rates under the cursor (parallax).
 */
const CARDS: QuestionCardData[] = [
  {
    name: 'Cassandra Vela',
    role: 'Proposer',
    verified: true,
    question: 'Did protocol X ship mainnet by Jun 30?',
    verdict: (
      <>
        Proposed: <span className="text-lavender-phosphor">Yes</span> · bond posted
      </>
    ),
    pos: 'lg:top-[24px] lg:left-0',
    depth: 12,
  },
  {
    name: 'Runner Node',
    role: 'AI Resolver',
    verified: true,
    question: 'Rerunning the pinned model over the agreed facts.',
    verdict: (
      <>
        Confidence <span className="text-lavender-phosphor">96%</span> · hash committed
      </>
    ),
    pos: 'lg:top-[8px] lg:right-[16px]',
    depth: 7,
  },
  {
    name: 'Milo Trent',
    role: 'Challenger',
    question: 'Disputing the CPI count — bond in escrow.',
    verdict: <>Challenge window: open</>,
    pos: 'lg:top-[286px] lg:left-0',
    depth: 9,
  },
  {
    name: 'Ada North',
    role: 'Staker',
    question: 'Backed the honest side of the market.',
    verdict: (
      <>
        Settlement <span className="text-lavender-phosphor">+2.4%</span>
      </>
    ),
    pos: 'lg:top-[300px] lg:right-0',
    depth: 13,
  },
  {
    name: 'Grants DAO',
    role: 'Proposer',
    question: 'Will the milestone verify on-chain by epoch end?',
    pos: 'lg:bottom-[8px] lg:left-[96px]',
    depth: 6,
  },
  {
    name: 'Facts Registry',
    role: 'AI Resolver',
    verified: true,
    question: 'Verdict committed over the agreed facts.',
    verdict: (
      <>
        Resolved: <span className="text-lavender-phosphor">No</span>
      </>
    ),
    pos: 'lg:bottom-[28px] lg:right-[120px]',
    depth: 10,
  },
]

function QuestionCard({ card, index }: { card: QuestionCardData; index: number }) {
  return (
    // Outer: absolute scatter position + staggered scroll-reveal entrance.
    <Reveal
      className={'w-full lg:absolute lg:w-[248px] ' + card.pos}
      delay={index * 90}
    >
      {/* Inner: pointer parallax drift (kept off the reveal element so the two
          transforms don't clash). */}
      <div className="drift" style={{ '--drift-depth': `${card.depth}px` } as CSSProperties}>
        <article
          className={
            'rounded-card border border-pebble bg-pure-card p-4 ' +
            'transition-[transform,border-color,box-shadow] duration-200 ' +
            'hover:-translate-y-1 hover:border-cyan-phosphor/40'
          }
        >
          <div className="flex items-center gap-3">
            <AvatarBubble name={card.name} size={44} verified={card.verified} />
            <div className="min-w-0">
              <p className="truncate font-inter text-[13px] font-medium text-sepia">{card.name}</p>
              <p className="font-inter text-[12px] text-driftwood">{card.role}</p>
            </div>
          </div>
          <p className="mt-3 font-inter text-[14px] leading-snug text-charcoal-bark">
            {card.question}
          </p>
          {card.verdict ? (
            <p className="mt-2 font-inter text-[13px] font-medium text-bronze">{card.verdict}</p>
          ) : null}
        </article>
      </div>
    </Reveal>
  )
}

/**
 * Hero — the signature Auros constellation, now cursor-reactive. A centered
 * two-line display headline surrounded by scattered question cards ("orbiting
 * voices"). A bioluminescent orb tracks the cursor across the field, and every
 * card drifts toward the pointer at its own depth for a parallax-in-water feel.
 * Desktop: cards are absolutely positioned around the headline. Mobile: the
 * whole thing collapses to a vertical stack (headline first) + a 1/2-col grid.
 * All motion is transform/opacity only and disabled under prefers-reduced-motion
 * (see usePointerField + the `.reveal`/`.drift` utilities).
 */
export default function Hero() {
  const fieldRef = usePointerField<HTMLElement>()

  return (
    <section
      id="top"
      ref={fieldRef}
      aria-labelledby="hero-heading"
      className="relative overflow-hidden px-6 pt-16 pb-8 lg:pt-20"
    >
      {/* Bioluminescent cursor orb — atmospheric, non-interactive. Full-bleed
          across the section and clipped at the viewport edges (overflow-hidden
          above), so its soft falloff never shows a box edge mid-content. */}
      <div aria-hidden="true" className="cursor-orb pointer-events-none absolute inset-0 z-0" />

      <div className="relative z-10 mx-auto max-w-[1200px] lg:min-h-[680px]">
        {/* Headline layer — first in DOM (mobile order), centered overlay on desktop.
            Drifts gently OPPOSITE the cards (negative depth) for layered depth. */}
        <div className="relative z-10 mx-auto flex max-w-[680px] flex-col items-center text-center lg:absolute lg:inset-0 lg:justify-center">
          <div className="drift" style={{ '--drift-depth': '-4px' } as CSSProperties}>
            <h1
              id="hero-heading"
              className="font-serif font-light text-sepia text-[clamp(3rem,8vw,4rem)] leading-[1] tracking-[-0.03em]"
            >
              <span className="block">Truth,</span>
              <span className="block italic text-bronze">settled.</span>
            </h1>
            <p className="mt-6 max-w-[520px] font-inter text-[17px] leading-relaxed text-bronze">
              Kassandra is a decentralized, AI-assisted optimistic oracle on Solana: propose an
              answer, open a challenge window, and let anyone reproduce the verdict.
            </p>
            <div className="mt-8 flex flex-wrap items-center justify-center gap-4">
              <Button variant="PrimaryChestnut">Read the docs</Button>
              <Button
                variant="GhostOutline"
                onClick={() =>
                  document.getElementById('how-it-works')?.scrollIntoView({ behavior: 'smooth' })
                }
              >
                See how it works
              </Button>
            </div>
          </div>
        </div>

        {/* Cards layer — static grid on mobile, absolute scatter on desktop. */}
        <div
          aria-label="Example oracle questions and verdicts"
          className="mt-12 grid grid-cols-1 gap-4 sm:grid-cols-2 lg:mt-0 lg:block"
        >
          {CARDS.map((card, i) => (
            <QuestionCard key={card.name} card={card} index={i} />
          ))}
        </div>
      </div>
    </section>
  )
}
