import { useState, useEffect, useCallback, useRef, memo } from 'react'
import { v4 as uuid } from 'uuid'
import { Canvas } from './components/Canvas'
import { Card } from './components/Card'
import { Toolbar } from './components/Toolbar'
import { Sidebar } from './components/Sidebar'
import { ConnectionLines } from './components/ConnectionLines'
import { SettingsModal } from './components/SettingsModal'
import { ConfirmationModal } from './components/ConfirmationModal'
import { CardData, CardType, CardSnapshot, ViewportState, AppSettings, ApiConfig } from './types'
import { DEFAULT_CARD_SIZES, GRID_SIZE } from './constants'
import { healthCheck, getConfig, createSession, getAuthToken } from './services/api'
import { loadCanvasState, saveCanvasState } from './services/storage'

interface CanvasState { cards: CardData[]; viewport: ViewportState; settings: AppSettings }

const DEFAULT_SETTINGS: AppSettings = {
  theme: 'dark', showGrid: true, snapToGrid: false, authToken: getAuthToken(),
}

function createSnapshot(card: CardData): CardSnapshot {
  return { x: card.x, y: card.y, width: card.width, height: card.height, title: card.title, content: card.content, chatHistory: card.chatHistory, sessionId: card.sessionId, groupId: card.groupId, isCollapsed: card.isCollapsed }
}

function snapToGrid(v: number): number { return Math.round(v / GRID_SIZE) * GRID_SIZE }

// ── Memoized Card to avoid re-rendering all cards on viewport/sibling changes ─
const MemoizedCard = memo(Card)

// ── App ──────────────────────────────────────────────────────────────────
export default function App() {
  const [cards, setCards] = useState<CardData[]>([])
  const [viewport, setViewport] = useState<ViewportState>({ x: 0, y: 0, scale: 1 })
  const [settings, setSettings] = useState<AppSettings>(DEFAULT_SETTINGS)
  const [serverConfig, setServerConfig] = useState<ApiConfig | null>(null)
  const [connStatus, setConnStatus] = useState<'ok' | 'err' | 'checking'>('checking')
  const [isSelectionMode, setIsSelectionMode] = useState(false)
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set())
  const [settingsOpen, setSettingsOpen] = useState(false)
  const [confirmModal, setConfirmModal] = useState<{ open: boolean; title: string; message: string; onConfirm: () => void }>({ open: false, title: '', message: '', onConfirm: () => {} })
  const saveTimer = useRef<ReturnType<typeof setTimeout> | null>(null)

  // ── Load state from IndexedDB (with localStorage migration) ────────
  useEffect(() => {
    loadCanvasState<CanvasState>().then(state => {
      if (state) {
        setCards(state.cards ?? [])
        setViewport(state.viewport ?? { x: 0, y: 0, scale: 1 })
        setSettings({ ...DEFAULT_SETTINGS, ...(state.settings ?? {}) })
      }
    })
  }, [])

  // ── Apply theme to document root ─────────────────────────────────────
  useEffect(() => {
    const root = document.documentElement
    if (settings.theme === 'light') {
      root.classList.add('theme-light')
    } else {
      root.classList.remove('theme-light')
    }
  }, [settings.theme])

  // ── Persist state with debounce (IndexedDB, no 5 MB quota limit) ───
  useEffect(() => {
    if (saveTimer.current) clearTimeout(saveTimer.current)
    saveTimer.current = setTimeout(() => {
      saveCanvasState({ cards, viewport, settings })
    }, 800)
    return () => { if (saveTimer.current) clearTimeout(saveTimer.current) }
  }, [cards, settings])

  // Persist viewport separately with a longer debounce so panning/zooming
  // doesn't trigger card re-renders or excessive IndexedDB writes.
  const vpSaveTimer = useRef<ReturnType<typeof setTimeout> | null>(null)
  useEffect(() => {
    if (vpSaveTimer.current) clearTimeout(vpSaveTimer.current)
    vpSaveTimer.current = setTimeout(() => {
      saveCanvasState({ cards, viewport, settings })
    }, 2000)
    return () => { if (vpSaveTimer.current) clearTimeout(vpSaveTimer.current) }
  }, [viewport])

  // ── Health check + config load ────────────────────────────────────────
  useEffect(() => {
    healthCheck().then(() => {
      setConnStatus('ok')
      getConfig().then(setServerConfig).catch(() => {})
    }).catch(() => setConnStatus('err'))
  }, [])

  // ── Keyboard shortcuts ────────────────────────────────────────────────
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        setSettingsOpen(false)
        setIsSelectionMode(false)
        setSelectedIds(new Set())
      }
    }
    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
  }, [])

  // ── Card CRUD ─────────────────────────────────────────────────────────
  const addCard = useCallback((partial: Partial<CardData>): CardData => {
    const type = partial.type ?? CardType.CHAT
    const size = DEFAULT_CARD_SIZES[type]
    // Place at center of viewport
    const cx = (-viewport.x + window.innerWidth / 2) / viewport.scale - size.w / 2
    const cy = (-viewport.y + window.innerHeight / 2) / viewport.scale - size.h / 2
    const x = settings.snapToGrid ? snapToGrid(cx) : cx
    const y = settings.snapToGrid ? snapToGrid(cy) : cy

    const card: CardData = {
      id: uuid(),
      type,
      x, y,
      width: size.w,
      height: size.h,
      zIndex: Date.now(),
      ...partial,
    }
    const snap = createSnapshot(card)
    card.history = [snap]
    card.historyIndex = 0

    setCards(prev => [...prev, card])
    return card
  }, [viewport, settings.snapToGrid])

  const updateCard = useCallback((id: string, updates: Partial<CardData>, saveCheckpoint = false) => {
    setCards(prev => prev.map(c => {
      if (c.id !== id) return c
      const updated: CardData = { ...c, ...updates }
      if (saveCheckpoint) {
        const snap = createSnapshot(updated)
        const oldHistory = c.history ?? [createSnapshot(c)]
        const oldIdx = c.historyIndex ?? oldHistory.length - 1
        const newHistory = [...oldHistory.slice(0, oldIdx + 1), snap]
        updated.history = newHistory
        updated.historyIndex = newHistory.length - 1
      }
      return updated
    }))
  }, [])

  const removeCard = useCallback((id: string) => {
    setCards(prev => prev.filter(c => c.id !== id && c.groupId !== id))
    setSelectedIds(prev => { const next = new Set(prev); next.delete(id); return next })
  }, [])

  const navigateHistory = useCallback((id: string, dir: -1 | 1) => {
    setCards(prev => prev.map(c => {
      if (c.id !== id || !c.history) return c
      const newIdx = (c.historyIndex ?? 0) + dir
      if (newIdx < 0 || newIdx >= c.history.length) return c
      return { ...c, ...c.history[newIdx], historyIndex: newIdx }
    }))
  }, [])

  const bringToFront = useCallback((id: string) => {
    setCards(prev => prev.map(c => c.id === id ? { ...c, zIndex: Date.now() } : c))
  }, [])

  // ── Selection ─────────────────────────────────────────────────────────
  const handleSelect = useCallback((id: string) => {
    setSelectedIds(prev => {
      const next = new Set(prev)
      if (next.has(id)) { next.delete(id) } else { next.add(id) }
      return next
    })
  }, [])

  const deleteSelected = useCallback(() => {
    selectedIds.forEach(id => removeCard(id))
    setSelectedIds(new Set())
    setIsSelectionMode(false)
  }, [selectedIds, removeCard])

  const groupSelected = useCallback(() => {
    if (selectedIds.size < 2) return
    const island = addCard({ type: CardType.ISLAND, title: 'Island' })
    setCards(prev => prev.map(c =>
      selectedIds.has(c.id) && c.id !== island.id ? { ...c, groupId: island.id } : c
    ))
    setSelectedIds(new Set())
    setIsSelectionMode(false)
  }, [selectedIds, addCard])

  // ── Delete with confirmation ──────────────────────────────────────────
  const requestDelete = useCallback((id: string) => {
    setConfirmModal({
      open: true,
      title: 'Delete Card',
      message: 'Are you sure you want to delete this card?',
      onConfirm: () => {
        removeCard(id)
        setConfirmModal(m => ({ ...m, open: false }))
      },
    })
  }, [removeCard])

  // ── Add card handlers ─────────────────────────────────────────────────
  const handleAddCard = useCallback(async (type: CardType) => {
    if (type === CardType.CHAT) {
      try {
        const session = await createSession(`chat-${new Date().toISOString().slice(0, 16)}`)
        addCard({ type, title: session.name, sessionId: session.id })
      } catch {
        addCard({ type, title: 'Chat' })
      }
    } else {
      const labels: Partial<Record<CardType, string>> = {
        [CardType.SESSION]: 'Session Viewer', [CardType.NOTE]: 'Note',
        [CardType.ANALYTICS]: 'Analytics', [CardType.TERMINAL]: 'Terminal',
        [CardType.SKILLS]: 'Skills', [CardType.CONTEXT]: 'Context',
        [CardType.PLUGINS]: 'Plugins', [CardType.ISLAND]: 'Island',
      }
      addCard({ type, title: labels[type] ?? type })
    }
  }, [addCard])

  // ── Omnibar submit → new CHAT card ────────────────────────────────────
  const handleOmniSubmit = useCallback(async (query: string) => {
    // Check if a single chat card is already selected; if so, it will handle its own input
    // Otherwise create a new chat card
    let sessionId: string | undefined
    try {
      const session = await createSession(`chat-${new Date().toISOString().slice(0, 16)}`)
      sessionId = session.id
    } catch { /* proceed without session */ }

    addCard({
      type: CardType.CHAT,
      title: query.slice(0, 40),
      sessionId,
      chatHistory: [{ role: 'user', content: query }],
    })
  }, [addCard])

  // ── Visible cards (hide cards inside collapsed islands) ───────────────
  const collapsedIslandIds = new Set(cards.filter(c => c.type === CardType.ISLAND && c.isCollapsed).map(c => c.id))
  const visibleCards = cards.filter(c => !c.groupId || !collapsedIslandIds.has(c.groupId))

  return (
    <>
      <Canvas viewport={viewport} onViewport={setViewport} showGrid={settings.showGrid}>
        <ConnectionLines cards={visibleCards} />
        {visibleCards.map(card => (
          <MemoizedCard
            key={card.id}
            data={card}
            isSelected={selectedIds.has(card.id)}
            isSelectionMode={isSelectionMode}
            onUpdate={updateCard}
            onDelete={requestDelete}
            onSelect={handleSelect}
            onBringToFront={bringToFront}
            navigateHistory={navigateHistory}
          />
        ))}
      </Canvas>

      <Sidebar
        connStatus={connStatus}
        onAddCard={type => void handleAddCard(type)}
        onSettings={() => setSettingsOpen(true)}
      />

      <Toolbar
        onAddCard={type => void handleAddCard(type)}
        onOmniSubmit={query => void handleOmniSubmit(query)}
        isSelectionMode={isSelectionMode}
        onToggleSelection={() => { setIsSelectionMode(m => !m); setSelectedIds(new Set()) }}
        selectedCount={selectedIds.size}
        onDeleteSelected={deleteSelected}
        onGroupSelected={groupSelected}
      />

      {settingsOpen && (
        <SettingsModal
          settings={settings}
          config={serverConfig}
          onUpdate={updates => setSettings(s => ({ ...s, ...updates }))}
          onClose={() => setSettingsOpen(false)}
          onResetCanvas={() => setCards([])}
        />
      )}

      {confirmModal.open && (
        <ConfirmationModal
          title={confirmModal.title}
          message={confirmModal.message}
          onConfirm={confirmModal.onConfirm}
          onCancel={() => setConfirmModal(m => ({ ...m, open: false }))}
        />
      )}
    </>
  )
}
