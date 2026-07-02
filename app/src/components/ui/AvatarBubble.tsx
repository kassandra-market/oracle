import type { HTMLAttributes } from 'react'

export interface AvatarBubbleProps extends HTMLAttributes<HTMLDivElement> {
  /** Optional image URL. When absent, an initials-on-warm-gradient placeholder renders (offline-safe). */
  src?: string
  /** Name used for alt text + initials fallback. */
  name: string
  /** Diameter in px. Delphi avatars are 70px. */
  size?: number
  /** Overlay a cobalt VerifiedDot in the lower-right — the only true blue on the page. */
  verified?: boolean
}

function initials(name: string): string {
  const parts = name.trim().split(/\s+/).filter(Boolean)
  if (parts.length === 0) return '?'
  if (parts.length === 1) return parts[0].slice(0, 2).toUpperCase()
  return (parts[0][0] + parts[parts.length - 1][0]).toUpperCase()
}

/**
 * Delphi verified check. A cobalt (#1da1f2) circle with a white check, pinned
 * lower-right. This is the ONLY true blue permitted in the system.
 */
export function VerifiedDot({ size = 22 }: { size?: number }) {
  return (
    <span
      className="absolute bottom-0 right-0 grid place-items-center rounded-avatar bg-cobalt ring-2 ring-parchment"
      style={{ width: size, height: size }}
      aria-hidden="true"
    >
      <svg width={size * 0.6} height={size * 0.6} viewBox="0 0 24 24" fill="none">
        <path
          d="M5 12.5l4.2 4.2L19 7"
          stroke="#ffffff"
          strokeWidth="2.6"
          strokeLinecap="round"
          strokeLinejoin="round"
        />
      </svg>
    </span>
  )
}

/**
 * Delphi avatar bubble. A 70px circle (radius token `avatar`), no border. Shows
 * an image when `src` is given, otherwise an offline-safe initials-on-warm-
 * gradient placeholder. Optional cobalt VerifiedDot overlay.
 */
export function AvatarBubble({
  src,
  name,
  size = 70,
  verified = false,
  className = '',
  ...rest
}: AvatarBubbleProps) {
  return (
    <div
      className={`relative inline-block ${className}`}
      style={{ width: size, height: size }}
      {...rest}
    >
      {src ? (
        <img
          src={src}
          alt={name}
          className="h-full w-full rounded-avatar object-cover"
        />
      ) : (
        <div
          className="grid h-full w-full place-items-center rounded-avatar font-inter font-medium text-sepia"
          style={{
            background: 'linear-gradient(135deg, #fed0b3 0%, #f65726 100%)',
            fontSize: size * 0.34,
          }}
          role="img"
          aria-label={name}
        >
          {initials(name)}
        </div>
      )}
      {verified ? <VerifiedDot size={Math.max(18, size * 0.3)} /> : null}
    </div>
  )
}

export default AvatarBubble
