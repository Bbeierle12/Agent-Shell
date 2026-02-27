import { CardType } from './types'

export const MIN_SCALE = 0.15
export const MAX_SCALE = 3
export const GRID_SIZE = 50

export const DEFAULT_CARD_SIZES: Record<CardType, { w: number; h: number }> = {
  [CardType.CHAT]:      { w: 420, h: 560 },
  [CardType.SESSION]:   { w: 440, h: 520 },
  [CardType.NOTE]:      { w: 320, h: 300 },
  [CardType.ANALYTICS]: { w: 480, h: 600 },
  [CardType.TERMINAL]:  { w: 620, h: 420 },
  [CardType.SKILLS]:    { w: 400, h: 520 },
  [CardType.CONTEXT]:   { w: 380, h: 440 },
  [CardType.PLUGINS]:   { w: 380, h: 460 },
  [CardType.ISLAND]:    { w: 200, h: 60  },
}

export const CARD_COLORS: Record<CardType, string> = {
  [CardType.CHAT]:      '#4a9eff',
  [CardType.SESSION]:   '#a78bfa',
  [CardType.NOTE]:      '#fbbf24',
  [CardType.ANALYTICS]: '#34d399',
  [CardType.TERMINAL]:  '#6b7280',
  [CardType.SKILLS]:    '#f97316',
  [CardType.CONTEXT]:   '#38bdf8',
  [CardType.PLUGINS]:   '#e879f9',
  [CardType.ISLAND]:    '#6b7280',
}
