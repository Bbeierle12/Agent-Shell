import {
  MessageSquare, History, StickyNote, BarChart2,
  Terminal, BookOpen, GitBranch, Puzzle, Settings,
} from 'lucide-react'
import { CardType } from '../types'

interface Props {
  connStatus: 'ok' | 'err' | 'checking'
  onAddCard: (type: CardType) => void
  onSettings: () => void
}

const ITEMS: { type: CardType; icon: React.ReactNode; label: string }[] = [
  { type: CardType.CHAT,      icon: <MessageSquare size={16} />, label: 'Chat' },
  { type: CardType.SESSION,   icon: <History size={16} />,       label: 'Session' },
  { type: CardType.NOTE,      icon: <StickyNote size={16} />,    label: 'Note' },
  { type: CardType.ANALYTICS, icon: <BarChart2 size={16} />,     label: 'Analytics' },
  { type: CardType.TERMINAL,  icon: <Terminal size={16} />,      label: 'Terminal' },
  { type: CardType.SKILLS,    icon: <BookOpen size={16} />,      label: 'Skills' },
  { type: CardType.CONTEXT,   icon: <GitBranch size={16} />,     label: 'Context' },
  { type: CardType.PLUGINS,   icon: <Puzzle size={16} />,        label: 'Plugins' },
]

export function Sidebar({ connStatus, onAddCard, onSettings }: Props) {
  return (
    <div className="sidebar">
      {ITEMS.map(item => (
        <button
          key={item.type}
          className="sidebar-btn"
          onClick={() => onAddCard(item.type)}
          title={item.label}
        >
          {item.icon}
          <span className="sidebar-tooltip">{item.label}</span>
        </button>
      ))}

      <div className="sidebar-spacer" />

      <button className="sidebar-btn" onClick={onSettings} title="Settings">
        <Settings size={16} />
        <span className="sidebar-tooltip">Settings</span>
      </button>

      <div className={`conn-dot ${connStatus}`} title={connStatus === 'ok' ? 'Connected' : connStatus === 'err' ? 'Disconnected' : 'Checking…'} />
    </div>
  )
}
