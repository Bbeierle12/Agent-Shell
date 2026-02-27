import React, { useRef, useCallback, Suspense, lazy } from 'react'
import { CardData, CardType, LocalChatMessage } from '../types'
import { CARD_COLORS } from '../constants'
import { NoteCard } from './cards/NoteCard'
import { SessionCard } from './cards/SessionCard'
import { SkillsCard } from './cards/SkillsCard'
import { ContextCard } from './cards/ContextCard'
import { PluginsCard } from './cards/PluginsCard'

// Lazy-load heavy components (xterm ~500 kB, react-markdown ~200 kB)
const ChatCard = lazy(() => import('./cards/ChatCard').then(m => ({ default: m.ChatCard })))
const TerminalCard = lazy(() => import('./cards/TerminalCard').then(m => ({ default: m.TerminalCard })))
const AnalyticsCard = lazy(() => import('./cards/AnalyticsCard').then(m => ({ default: m.AnalyticsCard })))

interface Props {
  data: CardData
  isSelected: boolean
  isSelectionMode: boolean
  onUpdate: (id: string, updates: Partial<CardData>, checkpoint?: boolean) => void
  onDelete: (id: string) => void
  onSelect: (id: string) => void
  onBringToFront: (id: string) => void
  navigateHistory: (id: string, dir: -1 | 1) => void
}

export function Card({ data, isSelected, isSelectionMode, onUpdate, onDelete, onSelect, onBringToFront, navigateHistory }: Props) {
  const cardRef = useRef<HTMLDivElement>(null)
  const dragStart = useRef<{ mx: number; my: number; cx: number; cy: number } | null>(null)
  const resizeStart = useRef<{ mx: number; my: number; cw: number; ch: number } | null>(null)
  const hasMoved = useRef(false)

  // ── Drag: DOM-only during mousemove, flush to React on mouseup ──────
  const onHeaderMouseDown = useCallback((e: React.MouseEvent) => {
    if (isSelectionMode) return
    if ((e.target as HTMLElement).tagName === 'BUTTON' || (e.target as HTMLElement).tagName === 'INPUT') return
    e.stopPropagation()
    onBringToFront(data.id)
    dragStart.current = { mx: e.clientX, my: e.clientY, cx: data.x, cy: data.y }
    hasMoved.current = false

    const onMove = (ev: MouseEvent) => {
      if (!dragStart.current) return
      const dx = ev.clientX - dragStart.current.mx
      const dy = ev.clientY - dragStart.current.my
      hasMoved.current = true
      // Direct DOM update — no React re-render per frame
      const el = cardRef.current
      if (el) {
        el.style.left = `${dragStart.current.cx + dx}px`
        el.style.top = `${dragStart.current.cy + dy}px`
      }
    }
    const onUp = (ev: MouseEvent) => {
      if (dragStart.current && hasMoved.current) {
        const dx = ev.clientX - dragStart.current.mx
        const dy = ev.clientY - dragStart.current.my
        // Single flush: position + history checkpoint
        onUpdate(data.id, { x: dragStart.current.cx + dx, y: dragStart.current.cy + dy }, true)
      }
      dragStart.current = null
      window.removeEventListener('mousemove', onMove)
      window.removeEventListener('mouseup', onUp)
    }
    window.addEventListener('mousemove', onMove)
    window.addEventListener('mouseup', onUp)
  }, [data, isSelectionMode, onUpdate, onBringToFront])

  // ── Resize: DOM-only during mousemove, flush to React on mouseup ────
  const onResizeMouseDown = useCallback((e: React.MouseEvent) => {
    e.stopPropagation()
    e.preventDefault()
    resizeStart.current = { mx: e.clientX, my: e.clientY, cw: data.width, ch: data.height }

    const onMove = (ev: MouseEvent) => {
      if (!resizeStart.current) return
      const newW = Math.max(240, resizeStart.current.cw + ev.clientX - resizeStart.current.mx)
      const newH = Math.max(160, resizeStart.current.ch + ev.clientY - resizeStart.current.my)
      // Direct DOM update — no React re-render per frame
      const el = cardRef.current
      if (el) {
        el.style.width = `${newW}px`
        el.style.height = `${newH}px`
      }
    }
    const onUp = (ev: MouseEvent) => {
      if (resizeStart.current) {
        const newW = Math.max(240, resizeStart.current.cw + ev.clientX - resizeStart.current.mx)
        const newH = Math.max(160, resizeStart.current.ch + ev.clientY - resizeStart.current.my)
        onUpdate(data.id, { width: newW, height: newH }, true)
      }
      resizeStart.current = null
      window.removeEventListener('mousemove', onMove)
      window.removeEventListener('mouseup', onUp)
    }
    window.addEventListener('mousemove', onMove)
    window.addEventListener('mouseup', onUp)
  }, [data, onUpdate])

  const onCardClick = useCallback((e: React.MouseEvent) => {
    if (!isSelectionMode) return
    e.stopPropagation()
    onSelect(data.id)
  }, [isSelectionMode, onSelect, data.id])

  if (data.type === CardType.ISLAND) {
    return (
      <div
        ref={cardRef}
        className="island-card"
        style={{ left: data.x, top: data.y, width: data.width, height: data.height, zIndex: data.zIndex }}
        onMouseDown={onHeaderMouseDown}
      >
        <span className="island-label">{data.title || 'Island'}</span>
      </div>
    )
  }

  const histLen = data.history?.length ?? 0
  const histIdx = data.historyIndex ?? 0

  return (
    <div
      ref={cardRef}
      className={`card${isSelected ? ' selected' : ''}`}
      style={{ left: data.x, top: data.y, width: data.width, height: data.height, zIndex: data.zIndex }}
      onClick={onCardClick}
      onMouseDown={() => !isSelectionMode && onBringToFront(data.id)}
    >
      {/* Header */}
      <div className="card-header" onMouseDown={onHeaderMouseDown}>
        <div className="card-type-dot" style={{ background: CARD_COLORS[data.type] }} />
        <span className="card-title">{data.title || data.type}</span>
        <div className="card-header-btns">
          {histLen > 1 && (
            <>
              <button className="card-btn" onClick={() => navigateHistory(data.id, -1)} disabled={histIdx === 0} title="Undo">←</button>
              <button className="card-btn" onClick={() => navigateHistory(data.id, 1)} disabled={histIdx === histLen - 1} title="Redo">→</button>
            </>
          )}
          <button className="card-btn" onClick={() => onDelete(data.id)} title="Delete">×</button>
        </div>
      </div>

      {/* Body */}
      <div className="card-body">
        <Suspense fallback={<div className="card-inner" style={{ color: 'var(--text-muted)' }}>Loading…</div>}>
          {data.type === CardType.CHAT && (
            <ChatCard
              history={data.chatHistory ?? []}
              sessionId={data.sessionId}
              onHistoryUpdate={h => onUpdate(data.id, { chatHistory: h as LocalChatMessage[] }, true)}
            />
          )}
          {data.type === CardType.SESSION && <SessionCard />}
          {data.type === CardType.NOTE && (
            <NoteCard
              content={data.content ?? ''}
              onChange={text => onUpdate(data.id, { content: text })}
            />
          )}
          {data.type === CardType.ANALYTICS && <AnalyticsCard />}
          {data.type === CardType.TERMINAL && <TerminalCard />}
          {data.type === CardType.SKILLS && <SkillsCard />}
          {data.type === CardType.CONTEXT && <ContextCard />}
          {data.type === CardType.PLUGINS && <PluginsCard />}
        </Suspense>
      </div>

      {/* Resize handle */}
      <div className="card-resize" onMouseDown={onResizeMouseDown} title="Resize">
        <svg width="10" height="10" viewBox="0 0 10 10" fill="currentColor">
          <path d="M10 0 L10 10 L0 10 Z" opacity="0.5"/>
        </svg>
      </div>
    </div>
  )
}
