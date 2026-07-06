import { SectionHeader } from '../components/ui'
import { CreateMarketForm } from '../components/markets/actions/CreateMarketForm'

/**
 * Create-market page: bind a new binary prediction market to an existing
 * Kassandra oracle and seed its funding via {@link CreateMarketForm}.
 */
export default function CreateMarket() {
  return (
    <main className="mx-auto max-w-[1200px] px-6 py-20">
      <SectionHeader
        as="h1"
        eyebrow="Create"
        line1="New market"
        paragraph="Bind a new prediction market to an existing Kassandra oracle and seed its liquidity."
      />
      <div className="mx-auto mt-16 max-w-[640px]">
        <CreateMarketForm />
      </div>
    </main>
  )
}
