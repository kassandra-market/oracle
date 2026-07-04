import type { ReactNode } from 'react'
import { AvatarBubble, Button } from '../ui'

interface QuestionCardData {
  name: string
  role: string
  verified?: boolean
  question: string
  /** A short verdict / confidence line; the punchy fragment is highlighted in ember. */
  verdict?: ReactNode
  /** Desktop-only absolute placement (the scatter). Ignored in the mobile flow grid. */
  pos: string
}

/**
 * The "orbiting voices" of the constellation — living-looking oracle questions
 * and mini verdicts. Each is a flat white card (12px radius, hairline border):
 * an AvatarBubble + a name/role line + a one-line question + an optional verdict
 * with the punchy word/number in ember orange.
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
  },
  {
    name: 'Milo Trent',
    role: 'Challenger',
    question: 'Disputing the CPI count — bond in escrow.',
    verdict: <>Challenge window: open</>,
    pos: 'lg:top-[286px] lg:left-0',
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
  },
  {
    name: 'Grants DAO',
    role: 'Proposer',
    question: 'Will the milestone verify on-chain by epoch end?',
    pos: 'lg:bottom-[8px] lg:left-[96px]',
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
  },
]

function QuestionCard({ card }: { card: QuestionCardData }) {
  return (
    <article
      className={
        'w-full rounded-button border border-pebble bg-pure-card p-4 ' +
        'lg:absolute lg:w-[248px] ' +
        card.pos
      }
    >
      <div className="flex items-center gap-3">
        <AvatarBubble name={card.name} size={44} verified={card.verified} />
        <div className="min-w-0">
          <p className="truncate font-inter text-[13px] font-medium text-sepia">{card.name}</p>
          <p className="font-inter text-[12px] text-driftwood">{card.role}</p>
        </div>
      </div>
      <p className="mt-3 font-inter text-[14px] leading-snug text-charcoal-bark">{card.question}</p>
      {card.verdict ? (
        <p className="mt-2 font-inter text-[13px] font-medium text-bronze">{card.verdict}</p>
      ) : null}
    </article>
  )
}

/**
 * Hero — the signature Auros constellation. A centered two-line serif display
 * headline surrounded by scattered white question cards ("orbiting voices").
 * Desktop: cards are absolutely positioned around the headline. Mobile: the
 * whole thing collapses to a vertical stack (headline first) + a 1/2-col grid
 * of the cards. There is no hero illustration — the voices are the visual.
 */
export default function Hero() {
  return (
    <section id="top" aria-labelledby="hero-heading" className="px-6 pt-16 pb-8 lg:pt-20">
      <div className="relative mx-auto max-w-[1200px] lg:min-h-[680px]">
        {/* Headline layer — first in DOM (mobile order), centered overlay on desktop. */}
        <div className="relative z-10 mx-auto flex max-w-[680px] flex-col items-center text-center lg:absolute lg:inset-0 lg:justify-center">
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

        {/* Cards layer — static grid on mobile, absolute scatter on desktop. */}
        <div
          aria-label="Example oracle questions and verdicts"
          className="mt-12 grid grid-cols-1 gap-4 sm:grid-cols-2 lg:mt-0 lg:block"
        >
          {CARDS.map((card) => (
            <QuestionCard key={card.name} card={card} />
          ))}
        </div>
      </div>
    </section>
  )
}
