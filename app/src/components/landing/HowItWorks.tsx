import { Card, EyebrowTag, Reveal, SectionHeader, TriggerPreviewCard } from '../ui'

const STEPS: { step: string; title: string; body: string }[] = [
  {
    step: '01',
    title: 'Propose',
    body: 'Anyone answers an open question and posts a proposer bond. Optimism is the default — most answers are correct and cheap.',
  },
  {
    step: '02',
    title: 'Challenge window',
    body: 'A timed window opens. Anyone can dispute the answer by posting a challenge bond and staking against it.',
  },
  {
    step: '03',
    title: 'AI verdict',
    body: 'An open-source runner reruns a pinned model over the agreed facts. The inputs and verdict hashes are committed on-chain.',
  },
  {
    step: '04',
    title: 'Settle',
    body: 'Bonds and stakes are distributed to the honest side, the wrong side is slashed, and dead-end disputes are burned.',
  },
]

/**
 * "How it works" — a centered SectionHeader over the optimistic lifecycle as a
 * 4-up row of flat Cards (collapses to 2-col then 1-col), plus a TriggerPreview-
 * Card rendering the core Kassandra mechanic: the AI rerun on challenge, with
 * the state variable in ember orange and a "+ Settle" action row.
 */
export default function HowItWorks() {
  return (
    <section id="how-it-works" aria-label="How it works" className="px-6 py-20">
      <div className="mx-auto max-w-[1200px]">
        <Reveal>
          <SectionHeader
            eyebrow="How it works"
            eyebrowPill
            line1="An optimistic oracle"
            line2="with a mind."
            paragraph="Propose an answer, open a challenge window, let an open-source runner rerun a pinned model over the agreed facts, then settle — every input and verdict hash committed on-chain."
          />
        </Reveal>
        <ol className="mt-16 grid list-none grid-cols-1 gap-6 sm:grid-cols-2 lg:grid-cols-4">
          {STEPS.map((s, i) => (
            <Reveal as="li" key={s.step} delay={i * 90} className="h-full">
              <Card className="h-full transition-[transform,border-color] duration-200 hover:-translate-y-1 hover:border-cyan-phosphor/40">
                <EyebrowTag>{`Step ${s.step}`}</EyebrowTag>
                <h3 className="mt-3 font-serif text-heading-sm font-light text-sepia">{s.title}</h3>
                <p className="mt-2 font-inter text-body text-bronze">{s.body}</p>
              </Card>
            </Reveal>
          ))}
        </ol>

        <Reveal className="mx-auto mt-10 max-w-[520px]" delay={120}>
          <TriggerPreviewCard
            whenLabel="When"
            condition="AI reruns the pinned model over the agreed facts while"
            variable="oracle.state == Challenged"
            actionLabel="+ Settle · commit verdict hash on-chain"
          />
        </Reveal>
      </div>
    </section>
  )
}
