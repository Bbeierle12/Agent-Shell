import { CardData } from '../types'

interface Props { cards: CardData[] }

export function ConnectionLines({ cards }: Props) {
  const lines: React.ReactNode[] = []

  for (const card of cards) {
    if (!card.groupId) continue
    const parent = cards.find(c => c.id === card.groupId)
    if (!parent) continue
    const x1 = parent.x + parent.width / 2
    const y1 = parent.y + parent.height / 2
    const x2 = card.x + card.width / 2
    const y2 = card.y + card.height / 2
    lines.push(
      <line
        key={`${parent.id}-${card.id}`}
        x1={x1} y1={y1} x2={x2} y2={y2}
        stroke="rgba(88,166,255,0.25)"
        strokeWidth={1.5}
        strokeDasharray="4 4"
      />
    )
  }

  if (lines.length === 0) return null

  return (
    <svg className="conn-lines-svg" style={{ width: 10000, height: 10000, left: -5000, top: -5000 }}>
      {lines}
    </svg>
  )
}
