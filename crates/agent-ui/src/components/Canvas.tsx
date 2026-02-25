import React, { useRef, useCallback, useEffect } from 'react'
import { ViewportState } from '../types'
import { MIN_SCALE, MAX_SCALE } from '../constants'

interface Props {
  viewport: ViewportState
  onViewport: (vp: ViewportState) => void
  showGrid: boolean
  children: React.ReactNode
}

export function Canvas({ viewport, onViewport, showGrid, children }: Props) {
  const dragging = useRef(false)
  const lastPos = useRef({ x: 0, y: 0 })
  const containerRef = useRef<HTMLDivElement>(null)

  const onMouseDown = useCallback((e: React.MouseEvent) => {
    if (e.target !== e.currentTarget) return
    if (e.button !== 0) return
    dragging.current = true
    lastPos.current = { x: e.clientX, y: e.clientY }
    e.preventDefault()
  }, [])

  const onMouseMove = useCallback((e: React.MouseEvent) => {
    if (!dragging.current) return
    const dx = e.clientX - lastPos.current.x
    const dy = e.clientY - lastPos.current.y
    lastPos.current = { x: e.clientX, y: e.clientY }
    onViewport({ ...viewport, x: viewport.x + dx, y: viewport.y + dy })
  }, [viewport, onViewport])

  const onMouseUp = useCallback(() => { dragging.current = false }, [])

  const handleWheel = useCallback((e: WheelEvent) => {
    e.preventDefault()
    const el = containerRef.current
    if (!el) return
    if (e.ctrlKey || e.metaKey) {
      const rect = el.getBoundingClientRect()
      const mx = e.clientX - rect.left
      const my = e.clientY - rect.top
      const factor = e.deltaY < 0 ? 1.1 : 0.9
      const newScale = Math.min(MAX_SCALE, Math.max(MIN_SCALE, viewport.scale * factor))
      const newX = mx - (mx - viewport.x) * (newScale / viewport.scale)
      const newY = my - (my - viewport.y) * (newScale / viewport.scale)
      onViewport({ x: newX, y: newY, scale: newScale })
    } else {
      onViewport({ ...viewport, x: viewport.x - e.deltaX, y: viewport.y - e.deltaY })
    }
  }, [viewport, onViewport])

  useEffect(() => {
    const el = containerRef.current
    if (!el) return
    el.addEventListener('wheel', handleWheel, { passive: false })
    return () => el.removeEventListener('wheel', handleWheel)
  }, [handleWheel])

  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (!e.ctrlKey && !e.metaKey) return
      if (e.key === '=') { e.preventDefault(); onViewport({ ...viewport, scale: Math.min(MAX_SCALE, viewport.scale * 1.2) }) }
      else if (e.key === '-') { e.preventDefault(); onViewport({ ...viewport, scale: Math.max(MIN_SCALE, viewport.scale / 1.2) }) }
      else if (e.key === '0') { e.preventDefault(); onViewport({ x: 0, y: 0, scale: 1 }) }
    }
    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
  }, [viewport, onViewport])

  return (
    <div
      ref={containerRef}
      className={`canvas-bg${showGrid ? ' grid' : ''}`}
      style={{ backgroundPosition: `${viewport.x % 50}px ${viewport.y % 50}px`, cursor: dragging.current ? 'grabbing' : 'default' }}
      onMouseDown={onMouseDown}
      onMouseMove={onMouseMove}
      onMouseUp={onMouseUp}
      onMouseLeave={onMouseUp}
    >
      <div
        className="canvas-transform"
        style={{ transform: `translate(${viewport.x}px,${viewport.y}px) scale(${viewport.scale})` }}
      >
        {children}
      </div>

      <div className="zoom-controls">
        <button className="zoom-btn" onClick={() => onViewport({ ...viewport, scale: Math.min(MAX_SCALE, viewport.scale * 1.2) })}>+</button>
        <button className="zoom-btn" title="Reset" onClick={() => onViewport({ x: 0, y: 0, scale: 1 })}>⌖</button>
        <button className="zoom-btn" onClick={() => onViewport({ ...viewport, scale: Math.max(MIN_SCALE, viewport.scale / 1.2) })}>−</button>
      </div>
    </div>
  )
}
