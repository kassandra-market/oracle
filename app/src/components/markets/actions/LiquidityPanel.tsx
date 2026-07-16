import { useMemo, useState, type FormEvent } from "react";
import { MarketStatus } from "@kassandra-market/markets";
import { Button, Card } from "../../ui";
import { useMarkets } from "../../../market/hooks/useMarkets";
import { useConfig } from "../../../market/hooks/useMarketDetail";
import { useKassBalance } from "../../../market/hooks/useKassBalance";
import { useActionSequence } from "../../../market/hooks/useActionSequence";
import { useIndexer } from "../../../market/lib/indexer";
import {
  buildBulkClaimLpSteps,
  buildBulkContributeSteps,
  uniformSplit,
  type ActivateStep,
} from "../../../market/data/actions";
import { parseKassAmount, balanceGateError } from "../../../market/data/amount";
import { formatKass, outcomeLabel } from "../../../market/lib/marketView";
import type { MarketDetail, MarketSummary } from "../../../market/data/markets";
import { ConnectGate } from "./ConnectGate";
import { Field, KassBalanceLine, TextInput } from "./formPrimitives";
import { BatchStepList } from "./CreateMarketForm/BatchStepList";

/**
 * Liquidity provision + withdrawal for a market — on EVERY market, not just a
 * categorical group. Providing liquidity funds the market's escrow (Contribute),
 * which becomes the cYES/cNO pool traders trade against once it activates; when
 * an oracle has several outcome sub-markets they GROUP, and the default deposit
 * splits the entered total UNIFORMLY across the outcomes still in funding
 * ({@link uniformSplit}). Withdrawal claims the caller's pro-rata LP once fees are
 * collected. Both fan the single-market builders into one {@link useActionSequence}
 * run. Renders on all markets — a lone market is simply a group of one.
 */
export function LiquidityPanel({
  detail,
  onSuccess,
}: {
  detail: MarketDetail;
  onSuccess?: () => void;
}) {
  const oracle = detail.market.oracle.toString();
  const indexer = useIndexer();
  const config = useConfig();
  const kassMint = config.data ? config.data.kassMint.toString() : undefined;
  const { balance, loading: balanceLoading, refetch: refetchBalance } = useKassBalance(kassMint);
  const { data: allMarkets } = useMarkets();

  // The market's group = every sub-market on this oracle, in outcome order. Falls
  // back to the current market alone while the list loads / if it's not listed.
  const group = useMemo<MarketSummary[]>(() => {
    const siblings = (allMarkets ?? [])
      .filter((m) => m.market.oracle.toString() === oracle)
      .sort((a, b) => a.market.outcomeIndex - b.market.outcomeIndex);
    return siblings.length > 0
      ? siblings
      : [{ pubkey: detail.pubkey, market: detail.market, reserves: detail.reserves, oracleOptionsCount: null }];
  }, [allMarkets, oracle, detail]);

  const isGroup = group.length > 1;
  const funding = useMemo(
    () => group.filter((m) => m.market.status === MarketStatus.Funding),
    [group],
  );
  const feeCollected = useMemo(() => group.filter((m) => m.market.feeCollected), [group]);

  const [total, setTotal] = useState("");
  const [error, setError] = useState<string | undefined>();
  const [steps, setSteps] = useState<ActivateStep[]>([]);

  const seq = useActionSequence(() => {
    refetchBalance();
    onSuccess?.();
  });

  // Withdrawal is gated on the CALLER actually holding an unclaimed position on
  // this market (once its fee is collected) — mirrors the single-market ClaimLp
  // gate. For a group we then claim across every fee-collected outcome.
  const walletHasPosition =
    seq.address != null &&
    detail.market.feeCollected &&
    detail.contributions.some(
      (c) => c.contribution.contributor.toString() === seq.address && !c.contribution.claimed,
    );

  const parsed = total.trim() === "" ? null : parseKassAmount(total);
  const totalValue = parsed?.value ?? null;
  const shares = totalValue !== null ? uniformSplit(totalValue, funding.length) : [];
  const perShareHint =
    isGroup && funding.length > 0
      ? totalValue !== null && totalValue > 0n
        ? `Split uniformly across ${funding.length} funding outcome${funding.length > 1 ? "s" : ""} · ~${formatKass(shares[0])} KASS each`
        : `Split uniformly across ${funding.length} funding outcome${funding.length > 1 ? "s" : ""}`
      : "Funds the pool traders trade against once the market activates.";

  async function onDeposit(e: FormEvent) {
    e.preventDefault();
    setError(undefined);
    if (!kassMint || !seq.address) return;
    if (parsed?.error) return setError(parsed.error);
    if (totalValue === null || totalValue <= 0n) return setError("Enter an amount to deposit.");
    const gate = balanceGateError(totalValue, balance);
    if (gate) return setError(gate);

    const entries = funding.map((m, i) => ({
      market: m.pubkey,
      label: outcomeLabel(m.market.outcomeIndex),
      amount: shares[i],
    }));
    try {
      const built = await buildBulkContributeSteps({
        indexer,
        kassMint,
        contributor: seq.address,
        entries,
      });
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
    const entries = feeCollected.map((m) => ({
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

  const depositVerb = seq.busy
    ? "Providing…"
    : isGroup
      ? `Provide liquidity to ${funding.length} outcomes`
      : "Provide liquidity";

  return (
    <Card className="flex flex-col gap-4">
      <div>
        <h3 className="font-serif text-subheading font-light text-sepia">Provide liquidity</h3>
        <p className="mt-1 font-inter text-[13px] text-bronze">
          {isGroup
            ? `Fund or withdraw across all ${group.length} outcomes of this market at once — the liquidity traders trade against.`
            : "Add liquidity so traders can trade against the pool, then withdraw your share later."}
        </p>
      </div>

      <ConnectGate connected={seq.connected}>
        <div className="flex flex-col gap-5">
          {/* Deposit — provide liquidity to the outcome(s) still in funding. */}
          {funding.length > 0 ? (
            <form onSubmit={onDeposit} className="flex flex-col gap-2">
              <Field label="Provide liquidity (KASS)" hint={perShareHint} error={error}>
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
                  {depositVerb}
                </Button>
              </div>
            </form>
          ) : (
            <p className="font-inter text-[13px] text-driftwood">
              Funding is closed — this market&apos;s liquidity is live in the trading pool.
            </p>
          )}

          {/* Withdraw — claim the caller's LP once fees are collected. */}
          {walletHasPosition ? (
            <div className="flex flex-col gap-2 border-t border-pebble pt-4">
              <p className="font-inter text-[13px] text-bronze">
                {isGroup
                  ? `Withdraw your liquidity from ${feeCollected.length} settled outcome${feeCollected.length > 1 ? "s" : ""}.`
                  : "Withdraw your provided liquidity."}
              </p>
              <div>
                <Button type="button" variant="GhostOutline" disabled={seq.busy} onClick={onWithdraw}>
                  {seq.busy
                    ? "Withdrawing…"
                    : isGroup
                      ? `Withdraw from ${feeCollected.length} outcomes`
                      : "Withdraw liquidity"}
                </Button>
              </div>
            </div>
          ) : null}

          {steps.length > 0 ? <BatchStepList steps={steps} statuses={seq.statuses} /> : null}
        </div>
      </ConnectGate>
    </Card>
  );
}

export default LiquidityPanel;
