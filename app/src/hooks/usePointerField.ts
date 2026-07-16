import { useEffect, useRef } from 'react'

/**
 * Tracks the pointer inside a container and publishes its position as CSS custom
 * properties, SPRUNG toward the pointer each frame rather than snapped. A single
 * rAF integrator runs a critically-damped spring (no overshoot) per channel, so
 * the motion is velocity-aware and interruptible: a new pointer position just
 * re-targets the spring, which keeps its current velocity — no jump, no seam
 * (Apple's fluid-interface model). The `.drift` / `.cursor-orb` layers read the
 * sprung vars directly with NO CSS transition of their own.
 *
 * Properties set on the ref'd element:
 *   --pointer-x / --pointer-y   : -1..1, offset from the element center
 *   --pointer-px / --pointer-py : 0%..100%, raw position (for the spotlight)
 *
 * Gated to fine-pointer + hover devices, and disabled under
 * `prefers-reduced-motion` (re-evaluated live if the user toggles either). When
 * the cursor stops or leaves, the spring simply settles at its last target
 * (the field HOLDS its last position rather than recentering).
 */
export function usePointerField<T extends HTMLElement>() {
  const ref = useRef<T>(null)

  useEffect(() => {
    const el = ref.current
    if (!el) return

    const fine = window.matchMedia('(hover: hover) and (pointer: fine)')
    const reduce = window.matchMedia('(prefers-reduced-motion: reduce)')

    // Critically-damped spring (response ~0.3s, no overshoot — the calm default
    // for a non-momentum interaction; a cursor field shouldn't bounce).
    const STIFFNESS = 150
    const DAMPING = 2 * Math.sqrt(STIFFNESS)

    // Each channel: published position `p`, velocity `v`, and target `t`.
    const ch = {
      x: { p: 0, v: 0, t: 0 },
      y: { p: 0, v: 0, t: 0 },
      px: { p: 50, v: 0, t: 50 },
      py: { p: 32, v: 0, t: 32 },
    }
    type Channel = (typeof ch)[keyof typeof ch]
    const all: Channel[] = [ch.x, ch.y, ch.px, ch.py]

    let raf = 0
    let last = 0

    const write = () => {
      el.style.setProperty('--pointer-x', ch.x.p.toFixed(4))
      el.style.setProperty('--pointer-y', ch.y.p.toFixed(4))
      el.style.setProperty('--pointer-px', `${ch.px.p.toFixed(3)}%`)
      el.style.setProperty('--pointer-py', `${ch.py.p.toFixed(3)}%`)
    }

    const step = (ts: number) => {
      if (!last) last = ts
      // Clamp dt so a tab-switch / GC pause can't blow up the integrator.
      const dt = Math.min((ts - last) / 1000, 1 / 30)
      last = ts

      let moving = false
      for (const c of all) {
        const force = -STIFFNESS * (c.p - c.t) - DAMPING * c.v
        c.v += force * dt
        c.p += c.v * dt
        if (Math.abs(c.v) > 0.01 || Math.abs(c.p - c.t) > 0.01) moving = true
      }
      write()

      if (moving) {
        raf = requestAnimationFrame(step)
      } else {
        // Snap exactly onto the target and stop the loop until the next move.
        for (const c of all) {
          c.p = c.t
          c.v = 0
        }
        write()
        raf = 0
        last = 0
      }
    }
    const ensureRunning = () => {
      if (!raf) {
        last = 0
        raf = requestAnimationFrame(step)
      }
    }

    const onMove = (e: PointerEvent) => {
      const r = el.getBoundingClientRect()
      if (r.width === 0 || r.height === 0) return
      const rx = (e.clientX - r.left) / r.width
      const ry = (e.clientY - r.top) / r.height
      ch.x.t = Math.max(-1, Math.min(1, rx * 2 - 1))
      ch.y.t = Math.max(-1, Math.min(1, ry * 2 - 1))
      ch.px.t = Math.max(0, Math.min(100, rx * 100))
      ch.py.t = Math.max(0, Math.min(100, ry * 100))
      ensureRunning()
    }

    let bound = false
    const enable = () => {
      if (bound) return
      bound = true
      el.addEventListener('pointermove', onMove)
    }
    const disable = () => {
      if (!bound) return
      bound = false
      el.removeEventListener('pointermove', onMove)
    }

    const sync = () => (fine.matches && !reduce.matches ? enable() : disable())
    sync()
    fine.addEventListener('change', sync)
    reduce.addEventListener('change', sync)

    return () => {
      fine.removeEventListener('change', sync)
      reduce.removeEventListener('change', sync)
      disable()
      if (raf) cancelAnimationFrame(raf)
    }
  }, [])

  return ref
}

export default usePointerField
