import { useState, type FormEvent } from "react";
import { MarketStatus } from "@kassandra-market/markets";
import { Button, Card } from "../../ui";
import { useConfig } from "../../../market/hooks/useMarketDetail";
import { useKassBalance } from "../../../market/hooks/useKassBalance";
import { useActionSequence } from "../../../market/hooks/useActionSequence";
import { useIndexer } from "../../../market/lib/indexer";
import {
  buildBulkAddLiquiditySteps,
  buildBulkClaimLpSteps,
  buildBulkContributeSteps,
  uniformSplit,
  type ActivateStep,
  type BulkAddLiquidityEntry,
  type BulkContributeEntry,
} from "../../../market/data/actions";
import { parseKassAmount, balanceGateError } from "../../../market/data/amount";
import { formatKass, outcomeLabel } from "../../../market/lib/marketView";
import type { OracleGroupState } from "../../../market/hooks/useOracleGroup";
import { ConnectGate } from "./ConnectGate";
import { Field, KassBalanceLine, TextInput } from "./formPrimitives";
import { BatchStepList } from "./CreateMarketForm/BatchStepList";

/**
 * Bulk liquidity for a categorical oracle's GROUP of sub-markets: deposit into,
 * or withdraw LP from, several/all outcomes at once. The default deposit splits
 * the entered total UNIFORMLY ({@link uniformSplit}) across every outcome that can
 * currently take liquidity — Funding outcomes via `contribute`, Active outcomes
 * via `add_liquidity` into the live AMM — routing each share to the right builder.
 * Withdraw claims LP across every outcome whose fee has been collected. All fan the
 * single-market builders into one {@link useActionSequence} run. Renders nothing
 * for a lone market (not a group).
 *
 * While any outcome is still Funding, this is the ONLY place a categorical
 * group's floor can be seeded from — the market-detail page suppresses each
 * outcome's own per-market contribute form while a group exists (see
 * `MarketLiquidityActions`'s `isGrouped` gate), so there is exactly one funding
 * action (this uniform-split deposit). The single cumulative progress bar for
 * the group lives in `LiquidityOverview` above this panel, not duplicated here.
 */
export function GroupLiquidityPanel({
  group,
  embedded = false,
  onSuccess,
}: {
  /** The oracle group this market belongs to, computed once by the parent page
   *  via `useOracleGroup` (shared with the per-market contribute-form gate). */
  group: OracleGroupState;
  /** Render as a bare subsection (no Card wrapper) — for folding into another panel. */
  embedded?: boolean;
  /** Called after a deposit/withdraw sequence completes — the parent market-detail
   *  page's `refetchAfterWrite`, so its pool value / price impact also refreshes. */
  onSuccess?: () => void;
}) {
  const indexer = useIndexer();
  const config = useConfig();
  const kassMint = config.data ? config.data.kassMint.toString() : undefined;
  const { balance, loading: balanceLoading, refetch: refetchBalance } = useKassBalance(kassMint);
  const { siblings, claimable, depositable, refetch: refetchMarkets } = group;

  const [total, setTotal] = useState("");
  const [error, setError] = useState<string | undefined>();
  const [steps, setSteps] = useState<ActivateStep[]>([]);

  // A deposit/withdraw changes both the group's own AMM reserves/LP state (this
  // panel's `siblings`, sourced from `useMarkets`) and — for the market whose
  // detail page this panel is embedded in — the pool value + price impact shown
  // above it, sourced from a SEPARATE `useMarketDetail` fetch. Refetching only the
  // KASS balance left both of those stuck on pre-deposit reserves until the next
  // 15s poll or a manual reload.
  const seq = useActionSequence(() => {
    refetchBalance();
    refetchMarkets();
    onSuccess?.();
  });

  // Parse the total + its uniform per-outcome split across every depositable outcome.
  const parsed = total.trim() === "" ? null : parseKassAmount(total);
  const totalValue = parsed?.value ?? null;
  const shares = totalValue !== null ? uniformSplit(totalValue, depositable.length) : [];
  const perShareLabel =
    depositable.length > 0 && totalValue !== null && totalValue > 0n
      ? `${formatKass(shares[0])}${shares.some((s) => s !== shares[0]) ? "–" + formatKass(shares.find((s) => s !== shares[0])!) : ""} KASS each`
      : null;

  async function onDeposit(e: FormEvent) {
    e.preventDefault();
    setError(undefined);
    if (!kassMint || !seq.address) return;
    if (parsed?.error) return setError(parsed.error);
    if (totalValue === null || totalValue <= 0n) return setError("Enter an amount to deposit.");
    const gate = balanceGateError(totalValue, balance);
    if (gate) return setError(gate);

    // Route each depositable outcome's uniform share to the right builder: Funding
    // → contribute, Active → add_liquidity (into the live AMM).
    const contributeEntries: BulkContributeEntry[] = [];
    const addEntries: BulkAddLiquidityEntry[] = [];
    for (let i = 0; i < depositable.length; i++) {
      const m = depositable[i];
      const label = outcomeLabel(m.market.outcomeIndex);
      if (m.market.status === MarketStatus.Funding) {
        contributeEntries.push({ market: m.pubkey, label, amount: shares[i] });
      } else {
        addEntries.push({
          market: m.pubkey,
          label,
          amount: shares[i],
          marketAccount: m.market,
          reserves: m.reserves!,
        });
      }
    }
    try {
      const built: ActivateStep[] = [];
      if (contributeEntries.some((entry) => entry.amount > 0n)) {
        built.push(
          ...(await buildBulkContributeSteps({
            indexer,
            kassMint,
            contributor: seq.address,
            entries: contributeEntries,
          })),
        );
      }
      if (addEntries.some((entry) => entry.amount > 0n)) {
        built.push(
          ...(await buildBulkAddLiquiditySteps({ contributor: seq.address, entries: addEntries })),
        );
      }
      seq.reset();
      setSteps(built);
      await seq.run(built);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  }

  async function onWithdraw() {
    setError(undefined);
    if (!seq.address) return;
    const entries = claimable.map((m) => ({
      market: m.pubkey,
      label: outcomeLabel(m.market.outcomeIndex),
      lpMint: m.market.lpMint.toString(),
    }));
    try {
      const built = await buildBulkClaimLpSteps({ indexer, contributor: seq.address, entries });
      seq.reset();
      setSteps(built);
      await seq.run(built);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  }

  // Not a group (a lone market uses its own Contribute / Claim-LP controls).
  if (siblings.length <= 1) return null;

  const body = (
    <>
      <div>
        <h3 className="font-serif text-subheading font-light text-platinum">Group liquidity</h3>
        <p className="mt-1 font-inter text-[13px] text-silver">
          Fund or withdraw across all {siblings.length} outcomes of this market at once.
        </p>
      </div>

      <ConnectGate connected={seq.connected}>
        <div className="flex flex-col gap-5">
          {/* Deposit — uniform split across every outcome accepting liquidity
              (Funding → contribute, Active → add_liquidity into the live AMM). */}
          {depositable.length > 0 ? (
            <form onSubmit={onDeposit} className="flex flex-col gap-2">
              <Field
                label="Deposit (total KASS)"
                hint={
                  perShareLabel
                    ? `Split uniformly across ${depositable.length} outcome${depositable.length > 1 ? "s" : ""} · ${perShareLabel}`
                    : `Split uniformly across ${depositable.length} outcome${depositable.length > 1 ? "s" : ""}`
                }
                error={error}
              >
                {(ids) => (
                  <TextInput
                    ids={ids}
                    inputMode="decimal"
                    placeholder="0.0"
                    value={total}
                    onChange={(ev) => setTotal(ev.target.value)}
                  />
                )}
              </Field>
              <KassBalanceLine balance={balance} loading={balanceLoading} format={formatKass} />
              <div>
                <Button type="submit" variant="PrimaryChestnut" disabled={seq.busy}>
                  {seq.busy ? "Depositing…" : `Deposit into ${depositable.length} outcomes`}
                </Button>
              </div>
            </form>
          ) : (
            <p className="font-inter text-[13px] text-silver">
              No outcomes are accepting liquidity — deposits are closed for this group.
            </p>
          )}

          {/* Withdraw — claim LP across every outcome whose fee has been collected. */}
          {claimable.length > 0 ? (
            <div className="flex flex-col gap-2 border-t border-hairline pt-4">
              <p className="font-inter text-[13px] text-silver">
                Withdraw your LP from {claimable.length} settled outcome
                {claimable.length > 1 ? "s" : ""}.
              </p>
              <div>
                <Button type="button" variant="GhostOutline" disabled={seq.busy} onClick={onWithdraw}>
                  {seq.busy ? "Withdrawing…" : `Withdraw from ${claimable.length} outcomes`}
                </Button>
              </div>
            </div>
          ) : null}

          {steps.length > 0 ? <BatchStepList steps={steps} statuses={seq.statuses} /> : null}
        </div>
      </ConnectGate>
    </>
  );

  // Embedded → a bare subsection (divider + content) to fold into another panel;
  // standalone → its own Card.
  return embedded ? (
    <div className="flex flex-col gap-4 border-t border-hairline pt-5">{body}</div>
  ) : (
    <Card className="flex flex-col gap-4">{body}</Card>
  );
}

export default GroupLiquidityPanel;
