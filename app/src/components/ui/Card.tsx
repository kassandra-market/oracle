import type { HTMLAttributes, ReactNode } from 'react'

export interface CardProps extends HTMLAttributes<HTMLDivElement> {
  children: ReactNode
}

/**
 * Delphi content card. Pure-card #ffffff surface, 16px radius, 24px padding,
 * a single 1px pebble hairline border. Flat — NO heavy drop shadow (the only
 * shadow in the system is the chestnut button's peach bloom).
 */
export function Card({ className = '', children, ...rest }: CardProps) {
  return (
    <div
      className={`bg-pure-card rounded-card border border-pebble p-6 ${className}`}
      {...rest}
    >
      {children}
    </div>
  )
}

export default Card
