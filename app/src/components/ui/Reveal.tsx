import {
  useEffect,
  useRef,
  useState,
  type CSSProperties,
  type ElementType,
  type ReactNode,
} from 'react'

export interface RevealProps {
  children: ReactNode
  /** Stagger, in ms, applied as the CSS transition-delay once revealed. */
  delay?: number
  /** Element to render. Defaults to a plain `div`. */
  as?: ElementType
  className?: string
}

/**
 * Scroll-reveal wrapper: the child starts dimmed + nudged down (the `.reveal`
 * utility) and settles into place the first time it scrolls into view. Uses one
 * IntersectionObserver per instance and disconnects after firing, so it costs
 * nothing once revealed. `prefers-reduced-motion` is handled in CSS (the utility
 * lands elements in their final state), and we also reveal immediately when the
 * observer is unavailable, so content is never stuck hidden.
 */
export function Reveal({ children, delay = 0, as, className = '' }: RevealProps) {
  const Tag: ElementType = as ?? 'div'
  const ref = useRef<HTMLElement>(null)
  const [revealed, setRevealed] = useState(false)

  useEffect(() => {
    if (revealed) return
    const el = ref.current
    if (!el || typeof IntersectionObserver === 'undefined') {
      setRevealed(true)
      return
    }
    const io = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting)) {
          setRevealed(true)
          io.disconnect()
        }
      },
      { threshold: 0.12, rootMargin: '0px 0px -8% 0px' },
    )
    io.observe(el)
    return () => io.disconnect()
  }, [revealed])

  return (
    <Tag
      ref={ref}
      data-revealed={revealed ? 'true' : 'false'}
      style={delay ? ({ '--reveal-delay': `${delay}ms` } as CSSProperties) : undefined}
      className={`reveal ${className}`}
    >
      {children}
    </Tag>
  )
}

export default Reveal
