import { useEffect, useRef, useState } from "react";
import {
  CandlestickSeries,
  ColorType,
  CrosshairMode,
  createChart,
  type IChartApi,
  type ISeriesApi,
  type UTCTimestamp,
} from "lightweight-charts";
import { useIndexer, type CandleDto } from "../../market/lib/indexer";

/** Selectable candle widths (bucket seconds). */
const INTERVALS = [
  { label: "1m", secs: 60 },
  { label: "15m", secs: 900 },
  { label: "1H", secs: 3600 },
  { label: "1D", secs: 86_400 },
] as const;

/** Poll the indexer for fresh candles on this cadence (ms). */
const POLL_MS = 15_000;

const CHART_HEIGHT = 280;

/** Resolve a theme CSS custom property off a live element (falls back to `dflt`). */
function cssVar(el: HTMLElement, name: string, dflt: string): string {
  return getComputedStyle(el).getPropertyValue(name).trim() || dflt;
}

/**
 * A candlestick chart of a market's implied YES probability over time, backed by
 * the indexer's price series (`GET /api/markets/{pubkey}/candles`), which the
 * indexer records per-swap from a websocket `accountSubscribe` on the pool. Prices
 * are `0..1` probabilities rendered as percent. The chart is themed from the live
 * Auros CSS variables so it tracks the active palette, and polls for freshness.
 *
 * Meaningful only for an Active market (the cYES/cNO pool exists); a market with
 * no points yet renders a quiet empty state rather than a blank frame.
 */
export function PriceChart({ pubkey }: { pubkey: string }) {
  const indexer = useIndexer();
  const containerRef = useRef<HTMLDivElement | null>(null);
  const chartRef = useRef<IChartApi | null>(null);
  const seriesRef = useRef<ISeriesApi<"Candlestick"> | null>(null);
  const [intervalSecs, setIntervalSecs] = useState<number>(3600);
  const [empty, setEmpty] = useState(false);
  const [error, setError] = useState(false);

  // Create the chart once, themed from the resolved CSS variables.
  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    const up = cssVar(el, "--color-chestnut", "#8fe9dd");
    const down = cssVar(el, "--color-ember-orange", "#ff6f61");
    const text = cssVar(el, "--color-bronze", "#bbc7c6");
    const grid = "rgba(127, 143, 141, 0.16)";

    const chart = createChart(el, {
      layout: {
        background: { type: ColorType.Solid, color: "transparent" },
        textColor: text,
        fontFamily: "Inter, system-ui, sans-serif",
        attributionLogo: false,
      },
      grid: { vertLines: { color: grid }, horzLines: { color: grid } },
      crosshair: { mode: CrosshairMode.Normal },
      rightPriceScale: { borderColor: grid },
      timeScale: { borderColor: grid, timeVisible: true, secondsVisible: false },
      height: CHART_HEIGHT,
      width: Math.floor(el.clientWidth),
    });
    const series = chart.addSeries(CandlestickSeries, {
      upColor: up,
      downColor: down,
      borderUpColor: up,
      borderDownColor: down,
      wickUpColor: up,
      wickDownColor: down,
      priceFormat: {
        type: "custom",
        minMove: 0.001,
        formatter: (p: number) => `${(p * 100).toFixed(1)}%`,
      },
    });
    chartRef.current = chart;
    seriesRef.current = series;

    const ro = new ResizeObserver((entries) => {
      const w = entries[0]?.contentRect.width;
      if (w) chart.applyOptions({ width: Math.floor(w) });
    });
    ro.observe(el);

    return () => {
      ro.disconnect();
      chart.remove();
      chartRef.current = null;
      seriesRef.current = null;
    };
  }, []);

  // Load + poll candles for the selected interval.
  useEffect(() => {
    let active = true;
    const load = async () => {
      try {
        const candles: CandleDto[] = await indexer.getCandles(pubkey, intervalSecs);
        if (!active) return;
        setError(false);
        setEmpty(candles.length === 0);
        seriesRef.current?.setData(
          candles.map((c) => ({
            time: c.time as UTCTimestamp,
            open: c.open,
            high: c.high,
            low: c.low,
            close: c.close,
          })),
        );
        chartRef.current?.timeScale().fitContent();
      } catch {
        if (active) setError(true);
      }
    };
    void load();
    const id = setInterval(load, POLL_MS);
    return () => {
      active = false;
      clearInterval(id);
    };
  }, [indexer, pubkey, intervalSecs]);

  return (
    <div className="flex flex-col gap-3">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <span className="font-inter text-[12px] text-driftwood">
          Implied YES probability · history
        </span>
        <div
          role="group"
          aria-label="Candle interval"
          className="inline-flex rounded-button border border-pebble p-0.5"
        >
          {INTERVALS.map((iv) => {
            const selected = iv.secs === intervalSecs;
            return (
              <button
                key={iv.secs}
                type="button"
                aria-pressed={selected}
                onClick={() => setIntervalSecs(iv.secs)}
                className={`rounded-[10px] px-2.5 py-1 font-inter text-[12px] transition-colors ${
                  selected ? "bg-chestnut text-parchment" : "text-sepia hover:bg-pebble/50"
                }`}
              >
                {iv.label}
              </button>
            );
          })}
        </div>
      </div>
      <div className="relative">
        <div
          ref={containerRef}
          data-testid="price-chart"
          data-empty={empty ? "true" : "false"}
          className="w-full"
          style={{ height: CHART_HEIGHT }}
        />
        {(empty || error) && (
          <div className="pointer-events-none absolute inset-0 flex items-center justify-center">
            <p
              data-testid="price-chart-empty"
              className={`font-inter text-[13px] ${
                error ? "text-ember-orange" : "text-driftwood"
              }`}
            >
              {error
                ? "Couldn’t load price history."
                : "No price history yet — trades will populate the chart."}
            </p>
          </div>
        )}
      </div>
    </div>
  );
}

export default PriceChart;
