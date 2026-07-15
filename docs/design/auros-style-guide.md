# Auros — Style Reference (Kassandra UI visual language)

> abyssal terminal with bioluminescent data orbs · **Theme: dark**

The Kassandra UI adopts this visual language. Auros operates as an abyssal fintech terminal: a near-black teal canvas with bioluminescent data orbs and teal-to-pink light gradients that suggest depth, liquidity, and flow. The interface is sparse and cinematic, relying on a single custom display face (Matter) at medium weight with aggressive negative tracking to create scale without shouting. Color is rationed — achromatic whites and silvers carry almost all content, while the chromatic palette is reserved for atmospheric gradients, card-surface differentiation, and one signature pill button that morphs from teal-cyan to lavender-pink. Cards float on subtle teal-tinted surface lifts (16px radius, no shadows) rather than using elevation, so hierarchy reads as depth-of-water rather than shadow-on-paper. Components feel engineered and instrument-like: uppercase tracked labels, thin geometric arrow icons, large numerical stats in pale pink.

> **Authoritative tokens = the CSS block below.** (The source guide's prose tables carried OCR artifacts — garbled token names, a stray `9999px` pill radius, truncated hex values. The values here are corrected: buttons/small elements are **6px**, cards **16px** — the system is sharp-rounded, not pill-shaped.)

## Colors
| Token | Value | Role |
|-------|-------|------|
| `--color-liquid-abyss` | `#012624` | Page canvas — header, hero, sections. The dominant dark-teal field; all content floats on this. |
| `--color-liquid-deep` | `#011d1c` | Recessed surface — footer + deep panels that sink a half-step below the canvas. |
| `--color-liquid-kelp` | `#003734` | Raised card surface **and** primary button fill — the lifted layer one step above the abyss. |
| `--color-liquid-mist` | `#edfffe` | Emphasized body text, section labels, warm-light typographic moments (a barely-there cyan whisper). |
| `--color-platinum` | `#ffffff` | Headings (H1–H3), nav items, icon strokes — high-contrast text ONLY, never body copy. |
| `--color-silver-mist` | `#bbc7c6` | Secondary body text, muted descriptions, resting link color (faint green undertone). |
| `--color-ash` | `#f2f2f2` | Tertiary text for pull-quotes / testimonial copy — neutral cool-gray fallback. |
| `--color-slate-deep` | `#707777` | Low-emphasis surface tint (inactive states) + hairline borders. |
| `--color-lavender-phosphor` | `#fde9ff` | Large statistics, counter numbers, emphasis figures — the pink end of the signature gradient, used sparingly as luminous punctuation. |
| `--color-bioluminescent` | `linear-gradient(90deg, rgb(0,130,124) 0%, rgb(203,255,252) 100%)` | Signature button/UI gradient — teal-cyan → pale aqua. The brand's chromatic gesture. |
| `--color-aurora` | `linear-gradient(90deg, rgb(203,255,252) 0%, rgb(237,255,254) 26.25%, rgb(255,253,250) 47.57%, rgb(250,209,255) 88.96%)` | Supporting sweep — cyan → white → lavender-pink; the primary-CTA fill and small decorative accents. |

## Typography
- **Display / headings + body:** Matter (proprietary) → substitute **Inter** (also DM Sans / Satoshi). Weight **500** for ALL headings (H1–H3) and oversized kinetic text (86–295px) — no bold, no light; the medium-only strategy is core to the mechanical confidence. Weight **400** for body and UI copy. Aggressive negative tracking at scale (-0.04em at 61px, -0.046em at 86px); wide positive tracking on uppercase labels (0.08em at 20px, 0.12em at 12px, 0.15em at 10px).
- **UI fallback:** **Arial** (`system-ui`) at 14px — safe generic fallback for nav, buttons, hero micro-copy, footer.

Type scale: caption 10 (lh 1.4, tracking 1.5px) / body 16 (lh 1.4) / subheading 24 (lh 1.3, -0.48px) / heading 36 (lh 1) / heading-lg 61 (lh 1, -2.44px) / display 96 (lh 1, -3.84px). Line-height 1.0 for all display ≥36px, 1.4 for body — the contrast defines the rhythm.

## Spacing & shape
- Base unit 4px; scale {12,16,20,24,28,32,36,40,48,64,80,120,140,160,164}. Density: **spacious**.
- Radii vocabulary is SHORT — ONLY {6 (buttons/small elements), 16 (cards/image cards)}px. No pill radii, nothing above 16px.
- Layout: page max-width 1440px, section-gap 68px, card padding 36–48px, element gap 20px.

## Surfaces (flat elevation model)
0 Abyss `#012624` (page/hero/sections) · 1 Deep `#011d1c` (footer, sunken panels) · 2 Kelp `#003734` (content cards, lifted UI) · 3 Slate `#707777` (low-emphasis tint / inactive).
Depth is communicated through the teal surface stack (abyss → deep → kelp), NOT shadows — objects float at different water depths. **No drop shadows or box-shadows anywhere.**

## Components
- **Gradient Pill Button** (primary CTA) — aurora-gradient fill (cyan → white → pink), 6px radius, ~32px/22px padding, dark text `#222222` at 14px Arial uppercase. The most important action per section; horizontal sunrise sweep.
- **Ghost Navigation Link** — transparent, no border, uppercase 12px Matter 400, 0.12em tracking. Platinum white active / silver `#bbc7c6` inactive. Tight 16px column-gap, no padding.
- **Surface Card** — kelp `#003734` fill, 16px radius, 36px padding, no shadow/border. 36px Matter-500 platinum heading, 16px Matter-400 silver body.
- **Recessed Card** — deep `#011d1c` fill, 16px radius, 120px vertical padding — the deepest sunken well, for CTA panels with maximum breathing room.
- **Feature Row Card** — transparent, 16px radius, 48px/36px padding: heading + body + a small **Arrow Icon Button** top-right (service-listing rows).
- **Arrow Icon Button** — 32×32 square, 6px radius, semi-transparent dark-teal fill `rgba(3,81,75,0.5)`, white diagonal ↗. Always right of a card title as a "go-to" trigger.
- **Uppercase Section Label** (eyebrow/kicker) — 12–20px Matter-500 uppercase, 0.08–0.12em tracking, silver/mist. Reads as technical instrumentation labeling.
- **Hero Headline** — 61–96px Matter-500, lh 1.0, -0.04em, platinum; fluid via `clamp()`. Tight tracking compensates for the letterforms at scale.
- **Oversized Kinetic Text** — 86–295px Matter-500, lh 1.0, -0.046em, for massive section markers; text as environmental element.
- **Statistic Counter** — large number in lavender-phosphor `#fde9ff` + label below in mist/silver 13px uppercase tracked. The signature pink-on-teal glow — stats ONLY.
- **Navigation Bar** — full-width, transparent, ~80px tall. Wordmark left, links centered, CTA right; items at 16–24px gaps.
- **Particle Sphere Visual** — 3D teal-cyan + white particle orb picking up the canvas teal and pink accent at its edges (bioluminescent data-entity). The brand-defining hero visual — appears at least once on any major page.
- **Geometric Molecule Illustration** — flat white circles + thin connector lines forming an abstract network/molecular diagram; a right-column balancing element.

## Do / Don't
**Do:** teal surface stack (`#011d1c → #012624 → #003734`) for ALL background differentiation; aurora gradient ONLY on primary CTAs / signature accents; all headings Matter-500 (no bold/light at display sizes); uppercase 0.08–0.15em tracking on labels/eyebrows at 10–20px; lavender-phosphor `#fde9ff` ONLY for large statistics; radii only {6,16}; lh 1.0 for display ≥36px, lh 1.4 for body.
**Don't:** no drop/box shadows (depth = surface color shifts, not shadow); no bold(600+)/light(300-) at display sizes; no pure white body text (use silver `#bbc7c6` / mist `#edfffe` — white is headings/nav only); no aurora gradient on text, borders, or anything larger than a button; no radii above 16px / no pills; no color outside the Liquid teal scale, silver neutrals, and lavender-phosphor accent.

## Layout / imagery
Full-bleed dark canvas, max-width 1440px centered. Hero = centered text stack (eyebrow → headline → subtext → CTA) over a near-full-viewport particle sphere. Sections are full-width bands at 68px+ gaps, alternating canvas and slightly recessed surfaces; content centered in narrow (~600px) columns. The Explore-style section is asymmetric two-column: stacked feature cards left, geometric molecule illustration right. Footer is a recessed `#011d1c` well with 120px padding (deep-pool effect). Nav is a thin transparent bar. Imagery is minimal and atmospheric — no photography, no people; pure data-graphics and abstract forms. The particle sphere is the defining brand visual and anchors the deep-water metaphor.

## Authoritative CSS custom properties (corrected)
```css
:root {
  --color-liquid-abyss:#012624; --color-liquid-deep:#011d1c; --color-liquid-kelp:#003734;
  --color-liquid-mist:#edfffe; --color-platinum:#ffffff; --color-silver-mist:#bbc7c6;
  --color-ash:#f2f2f2; --color-slate-deep:#707777; --color-lavender-phosphor:#fde9ff;
  --gradient-bioluminescent:linear-gradient(90deg, rgb(0,130,124) 0%, rgb(203,255,252) 100%);
  --gradient-aurora:linear-gradient(90deg, rgb(203,255,252) 0%, rgb(237,255,254) 26.25%, rgb(255,253,250) 47.57%, rgb(250,209,255) 88.96%);

  --font-matter:'Matter', Inter, 'DM Sans', ui-sans-serif, system-ui, sans-serif;  /* display + body, weight 400/500 */
  --font-arial:'Arial', ui-sans-serif, system-ui, sans-serif;                       /* 14px UI fallback */

  --text-caption:10px;    --leading-caption:1.4;  --tracking-caption:1.5px;
  --text-body:16px;       --leading-body:1.4;
  --text-subheading:24px; --leading-subheading:1.3; --tracking-subheading:-0.48px;
  --text-heading:36px;    --leading-heading:1;
  --text-heading-lg:61px; --leading-heading-lg:1;   --tracking-heading-lg:-2.44px;
  --text-display:96px;    --leading-display:1;       --tracking-display:-3.84px;

  --font-weight-regular:400; --font-weight-medium:500;

  --spacing-12:12px; --spacing-16:16px; --spacing-20:20px; --spacing-24:24px; --spacing-28:28px;
  --spacing-32:32px; --spacing-36:36px; --spacing-40:40px; --spacing-48:48px; --spacing-64:64px;
  --spacing-80:80px; --spacing-120:120px; --spacing-140:140px; --spacing-160:160px; --spacing-164:164px;

  --radius-small:6px; --radius-button:6px; --radius-card:16px;

  --page-max-width:1440px; --section-gap:68px;
}
```

## Kassandra content adaptation
Keep the visual language EXACTLY; adapt the copy to Kassandra — a decentralized, AI-assisted **optimistic oracle** on Solana. Themes to weave: optimistic resolution (propose an answer + a challenge window); AI-assisted verdicts (an open-source runner reruns a pinned model over the agreed facts, all hashes committed on-chain); economic security (proposer/fact/vote bonds, staker settlement, slashing, dead-end burns); futarchy governance (MetaDAO) for parameters/treasury; challenge markets. The hero's **particle sphere becomes a live oracle data-orb**, and floating card clusters become glowing oracle questions + mini verdicts/proposer lines — bioluminescent instrument readouts rather than testimonials. Statistic counters (lavender-phosphor) surface bond totals, settled-question counts, and TWAP figures. Tagline direction: editorial, instrument-like, not hypey (e.g. "Truth, settled." / "An optimistic oracle with a mind.").
