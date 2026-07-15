import { useEffect, useRef } from 'react'

/**
 * Tracks the pointer inside a container and writes its position as CSS custom
 * properties on that element, so descendants can react purely in CSS (a
 * `.drift` parallax layer, a `.cursor-orb` spotlight) with ZERO React
 * re-renders per pointer move. Writes are coalesced to one rAF per frame.
 *
 * Properties set on the ref'd element:
 *   --pointer-x / --pointer-y   : -1..1, offset from the element center
 *   --pointer-px / --pointer-py : 0%..100%, raw position (for the spotlight)
 *
 * Gated to fine-pointer + hover devices, and disabled under
 * `prefers-reduced-motion` (re-evaluated live if the user toggles either).
 * When the cursor leaves (or the effect is disabled) the field HOLDS its last
 * position rather than recentering.
 */
export function usePointerField<T extends HTMLElement>() {
  const ref = useRef<T>(null)

  useEffect(() => {
    const el = ref.current
    if (!el) return

    const fine = window.matchMedia('(hover: hover) and (pointer: fine)')
    const reduce = window.matchMedia('(prefers-reduced-motion: reduce)')

    let raf = 0
    let x = 0
    let y = 0
    let px = 50
    let py = 30

    const flush = () => {
      raf = 0
      el.style.setProperty('--pointer-x', x.toFixed(3))
      el.style.setProperty('--pointer-y', y.toFixed(3))
      el.style.setProperty('--pointer-px', `${px.toFixed(2)}%`)
      el.style.setProperty('--pointer-py', `${py.toFixed(2)}%`)
    }
    const schedule = () => {
      if (!raf) raf = requestAnimationFrame(flush)
    }

    const onMove = (e: PointerEvent) => {
      const r = el.getBoundingClientRect()
      if (r.width === 0 || r.height === 0) return
      const rx = (e.clientX - r.left) / r.width
      const ry = (e.clientY - r.top) / r.height
      x = Math.max(-1, Math.min(1, rx * 2 - 1))
      y = Math.max(-1, Math.min(1, ry * 2 - 1))
      px = Math.max(0, Math.min(100, rx * 100))
      py = Math.max(0, Math.min(100, ry * 100))
      schedule()
    }
    // Intentionally no pointerleave handler: when the cursor exits we HOLD the
    // last position (the orb + parallax freeze where they were) instead of
    // recentering, so the hero keeps its last-hovered composition.
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
