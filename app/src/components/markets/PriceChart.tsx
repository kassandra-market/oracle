import { useEffect, useRef, useState } from "react";
import {
  ColorType,
  CrosshairMode,
  LineSeries,
  LineType,
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

/**
 * Both share curves are probabilities, so the price axis is PINNED to the full
 * 0..1 (0–100%) range rather than autoscaling to the data — the YES/NO split is
 * always read against the same fixed scale. Returned from each series'
 * `autoscaleInfoProvider`.
 */
const FULL_SCALE = { priceRange: { minValue: 0, maxValue: 1 } };

/** Percent price format shared by both curves. */
const PERCENT_FORMAT = {
  type: "custom" as const,
  minMove: 0.001,
  formatter: (v: number) => `${(v * 100).toFixed(1)}%`,
};

/** Resolve a theme CSS custom property off a live element (falls back to `dflt`). */
function cssVar(el: HTMLElement, name: string, dflt: string): string {
  return getComputedStyle(el).getPropertyValue(name).trim() || dflt;
}

/**
 * A price-history chart of a market's two outcome shares — one curve per share
 * (YES + its complement NO), each an implied probability line — backed by the
 * indexer's series (`GET /api/markets/{pubkey}/candles`, recorded per-swap from a
 * websocket `accountSubscribe` on the pool). The vertical axis is fixed 0–100%
 * (probabilities span the full range and the two curves always sum to 100%). The
 * chart is themed from the live Auros CSS variables and polls for freshness.
 *
 * Meaningful only for an Active market (the cYES/cNO pool exists); a market with
 * no points yet renders a quiet empty state rather than a blank frame.
 */
export function PriceChart({ pubkey }: { pubkey: string }) {
  const indexer = useIndexer();
  const containerRef = useRef<HTMLDivElement | null>(null);
  const chartRef = useRef<IChartApi | null>(null);
  const yesRef = useRef<ISeriesApi<"Line"> | null>(null);
  const noRef = useRef<ISeriesApi<"Line"> | null>(null);
  const [intervalSecs, setIntervalSecs] = useState<number>(3600);
  const [empty, setEmpty] = useState(false);
  const [error, setError] = useState(false);

  // Create the chart once, themed from the resolved CSS variables.
  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    const yesColor = cssVar(el, "--color-chestnut", "#8fe9dd");
    const noColor = cssVar(el, "--color-ember-orange", "#ff6f61");
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
    // One curve per share (YES + NO), both pinned to the 0–100% scale.
    const yes = chart.addSeries(LineSeries, {
      color: yesColor,
      lineWidth: 2,
      lineType: LineType.Curved,
      priceFormat: PERCENT_FORMAT,
      autoscaleInfoProvider: () => FULL_SCALE,
    });
    const no = chart.addSeries(LineSeries, {
      color: noColor,
      lineWidth: 2,
      lineType: LineType.Curved,
      priceFormat: PERCENT_FORMAT,
      autoscaleInfoProvider: () => FULL_SCALE,
    });
    chartRef.current = chart;
    yesRef.current = yes;
    noRef.current = no;

    const ro = new ResizeObserver((entries) => {
      const w = entries[0]?.contentRect.width;
      if (w) chart.applyOptions({ width: Math.floor(w) });
    });
    ro.observe(el);

    return () => {
      ro.disconnect();
      chart.remove();
      chartRef.current = null;
      yesRef.current = null;
      noRef.current = null;
    };
  }, []);

  // Load + poll candles for the selected interval; split each into the YES curve
  // (the implied YES probability) and the complementary NO curve (1 − YES).
  useEffect(() => {
    let active = true;
    const load = async () => {
      try {
        const candles: CandleDto[] = await indexer.getCandles(pubkey, intervalSecs);
        if (!active) return;
        setError(false);
        setEmpty(candles.length === 0);
        yesRef.current?.setData(
          candles.map((c) => ({ time: c.time as UTCTimestamp, value: c.close })),
        );
        noRef.current?.setData(
          candles.map((c) => ({ time: c.time as UTCTimestamp, value: 1 - c.close })),
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
        <div className="flex items-center gap-3 font-inter text-[12px]">
          <span className="text-driftwood">Share price · history</span>
          <span className="inline-flex items-center gap-1.5 text-sepia">
            <span className="h-2 w-2 rounded-full bg-chestnut" aria-hidden="true" />
            YES
          </span>
          <span className="inline-flex items-center gap-1.5 text-sepia">
            <span className="h-2 w-2 rounded-full bg-ember-orange" aria-hidden="true" />
            NO
          </span>
        </div>
        <div
          role="group"
          aria-label="Interval"
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
