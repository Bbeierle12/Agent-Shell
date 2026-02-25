import { useState } from 'react'
import {
  MessageSquare, StickyNote, BarChart2, Terminal,
  BookOpen, GitBranch, Puzzle, History, BoxSelect, Trash2, FolderPlus,
} from 'lucide-react'
import { CardType } from '../types'

interface Props {
  onAddCard: (type: CardType) => void
  onOmniSubmit: (query: string) => void
  isSelectionMode: boolean
  onToggleSelection: () => void
  selectedCount: number
  onDeleteSelected: () => void
  onGroupSelected: () => void
}

const CARD_BTNS: { type: CardType; icon: React.ReactNode; label: string }[] = [
  { type: CardType.CHAT,      icon: <MessageSquare size={15} />, label: 'Chat' },
  { type: CardType.SESSION,   icon: <History size={15} />,       label: 'Session' },
  { type: CardType.NOTE,      icon: <StickyNote size={15} />,    label: 'Note' },
  { type: CardType.ANALYTICS, icon: <BarChart2 size={15} />,     label: 'Analytics' },
  { type: CardType.TERMINAL,  icon: <Terminal size={15} />,      label: 'Terminal' },
  { type: CardType.SKILLS,    icon: <BookOpen size={15} />,      label: 'Skills' },
  { type: CardType.CONTEXT,   icon: <GitBranch size={15} />,     label: 'Context' },
  { type: CardType.PLUGINS,   icon: <Puzzle size={15} />,        label: 'Plugins' },
]

export function Toolbar({ onAddCard, onOmniSubmit, isSelectionMode, onToggleSelection, selectedCount, onDeleteSelected, onGroupSelected }: Props) {
  const [query, setQuery] = useState('')

  const handleSubmit = (e?: React.FormEvent) => {
    e?.preventDefault()
    if (!query.trim()) return
    onOmniSubmit(query.trim())
    setQuery('')
  }

  return (
    <div className="toolbar">
      {/* Omnibar */}
      <form className="toolbar-omnibar" onSubmit={handleSubmit}>
        <input
          value={query}
          onChange={e => setQuery(e.target.value)}
          placeholder="Ask the agent… (Enter)"
        />
        <button type="submit" style={{ background: 'none', border: 'none', cursor: 'pointer', color: 'var(--accent)', fontSize: 16 }}>↑</button>
      </form>

      <div className="toolbar-divider" />

      {/* Card type buttons */}
      {CARD_BTNS.map(b => (
        <button key={b.type} className="toolbar-btn" onClick={() => onAddCard(b.type)} title={b.label}>
          {b.icon}
          <span>{b.label}</span>
        </button>
      ))}

      <div className="toolbar-divider" />

      {/* Selection mode */}
      <button className={`toolbar-btn${isSelectionMode ? ' active' : ''}`} onClick={onToggleSelection} title="Selection mode">
        <BoxSelect size={15} />
        <span>Select</span>
      </button>

      {isSelectionMode && selectedCount > 0 && (
        <>
          <button className="toolbar-btn" onClick={onGroupSelected} title="Group into Island">
            <FolderPlus size={15} />
            <span>Island</span>
          </button>
          <button className="toolbar-btn" onClick={onDeleteSelected} title="Delete selected" style={{ color: 'var(--error)' }}>
            <Trash2 size={15} />
            <span>{selectedCount}</span>
          </button>
        </>
      )}
    </div>
  )
}
