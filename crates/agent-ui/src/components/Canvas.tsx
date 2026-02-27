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
  const transformRef = useRef<HTMLDivElement>(null)

  // Mutable viewport mirror — mousemove reads this instead of stale closure
  // values, avoiding re-creation of handlers and React re-renders per frame.
  const vpRef = useRef(viewport)
  vpRef.current = viewport

  // ── Direct-DOM helpers (zero React re-renders) ──────────────────────
  const applyDOM = useCallback((vp: ViewportState) => {
    const t = transformRef.current
    const c = containerRef.current
    if (t) t.style.transform = `translate(${vp.x}px,${vp.y}px) scale(${vp.scale})`
    if (c) c.style.backgroundPosition = `${vp.x % 50}px ${vp.y % 50}px`
  }, [])

  // Helper: apply viewport to DOM + ref, then flush to React state
  const commitViewport = useCallback((vp: ViewportState) => {
    vpRef.current = vp
    applyDOM(vp)
    onViewport(vp)
  }, [onViewport, applyDOM])

  // ── Pan via global mousemove (DOM-only until mouseup) ───────────────
  const onMouseDown = useCallback((e: React.MouseEvent) => {
    if (e.target !== e.currentTarget) return
    if (e.button !== 0) return
    dragging.current = true
    lastPos.current = { x: e.clientX, y: e.clientY }
    e.preventDefault()

    const onMove = (ev: MouseEvent) => {
      if (!dragging.current) return
      const dx = ev.clientX - lastPos.current.x
      const dy = ev.clientY - lastPos.current.y
      lastPos.current = { x: ev.clientX, y: ev.clientY }

      const vp = vpRef.current
      const next = { ...vp, x: vp.x + dx, y: vp.y + dy }
      vpRef.current = next
      applyDOM(next)            // DOM-only, no React re-render
    }

    const onUp = () => {
      dragging.current = false
      window.removeEventListener('mousemove', onMove)
      window.removeEventListener('mouseup', onUp)
      onViewport(vpRef.current) // single flush to React state
    }

    window.addEventListener('mousemove', onMove)
    window.addEventListener('mouseup', onUp)
  }, [onViewport, applyDOM])

  // ── Zoom (wheel) ────────────────────────────────────────────────────
  const handleWheel = useCallback((e: WheelEvent) => {
    e.preventDefault()
    const el = containerRef.current
    if (!el) return
    const vp = vpRef.current

    let next: ViewportState
    if (e.ctrlKey || e.metaKey) {
      const rect = el.getBoundingClientRect()
      const mx = e.clientX - rect.left
      const my = e.clientY - rect.top
      const factor = e.deltaY < 0 ? 1.1 : 0.9
      const newScale = Math.min(MAX_SCALE, Math.max(MIN_SCALE, vp.scale * factor))
      next = { x: mx - (mx - vp.x) * (newScale / vp.scale), y: my - (my - vp.y) * (newScale / vp.scale), scale: newScale }
    } else {
      next = { ...vp, x: vp.x - e.deltaX, y: vp.y - e.deltaY }
    }

    commitViewport(next)
  }, [commitViewport])

  useEffect(() => {
    const el = containerRef.current
    if (!el) return
    el.addEventListener('wheel', handleWheel, { passive: false })
    return () => el.removeEventListener('wheel', handleWheel)
  }, [handleWheel])

  // ── Keyboard zoom ───────────────────────────────────────────────────
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (!e.ctrlKey && !e.metaKey) return
      const vp = vpRef.current
      let next: ViewportState | null = null
      if (e.key === '=') { e.preventDefault(); next = { ...vp, scale: Math.min(MAX_SCALE, vp.scale * 1.2) } }
      else if (e.key === '-') { e.preventDefault(); next = { ...vp, scale: Math.max(MIN_SCALE, vp.scale / 1.2) } }
      else if (e.key === '0') { e.preventDefault(); next = { x: 0, y: 0, scale: 1 } }
      if (next) commitViewport(next)
    }
    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
  }, [commitViewport])

  return (
    <div
      ref={containerRef}
      className={`canvas-bg${showGrid ? ' grid' : ''}`}
      style={{ backgroundPosition: `${viewport.x % 50}px ${viewport.y % 50}px`, cursor: dragging.current ? 'grabbing' : 'default' }}
      onMouseDown={onMouseDown}
    >
      <div
        ref={transformRef}
        className="canvas-transform"
        style={{ transform: `translate(${viewport.x}px,${viewport.y}px) scale(${viewport.scale})` }}
      >
        {children}
      </div>

      <div className="zoom-controls">
        <button className="zoom-btn" onClick={() => commitViewport({ ...vpRef.current, scale: Math.min(MAX_SCALE, vpRef.current.scale * 1.2) })}>+</button>
        <button className="zoom-btn" title="Reset" onClick={() => commitViewport({ x: 0, y: 0, scale: 1 })}>⌖</button>
        <button className="zoom-btn" onClick={() => commitViewport({ ...vpRef.current, scale: Math.max(MIN_SCALE, vpRef.current.scale / 1.2) })}>−</button>
      </div>
    </div>
  )
}
