import { useState, useEffect } from 'react'
import ReactMarkdown from 'react-markdown'
import { ApiSession, ApiMessage } from '../../types'
import { listSessions, getSessionMessages } from '../../services/api'

export function SessionCard() {
  const [sessions, setSessions] = useState<ApiSession[]>([])
  const [selected, setSelected] = useState<string>('')
  const [messages, setMessages] = useState<ApiMessage[]>([])
  const [loading, setLoading] = useState(false)

  useEffect(() => {
    listSessions().then(s => {
      setSessions(s)
      if (s.length > 0 && !selected) setSelected(s[0].id)
    }).catch(() => {})
  }, [])

  useEffect(() => {
    if (!selected) return
    setLoading(true)
    getSessionMessages(selected)
      .then(setMessages)
      .catch(() => setMessages([]))
      .finally(() => setLoading(false))
  }, [selected])

  return (
    <div className="card-inner" style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
      <select
        value={selected}
        onChange={e => setSelected(e.target.value)}
        style={{ background: 'var(--bg)', border: '1px solid var(--border)', color: 'var(--text)', padding: '5px 8px', borderRadius: 6, fontSize: 12, width: '100%' }}
      >
        {sessions.map(s => (
          <option key={s.id} value={s.id}>{s.name} ({s.message_count} msgs)</option>
        ))}
      </select>

      <div style={{ flex: 1, overflowY: 'auto', display: 'flex', flexDirection: 'column', gap: 8 }}>
        {loading && <span style={{ color: 'var(--text-muted)', fontSize: 12 }}>Loading…</span>}
        {messages.filter(m => m.role === 'user' || m.role === 'assistant').map(msg => (
          <div key={msg.id} style={{
            padding: '7px 10px', borderRadius: 8, fontSize: 12, lineHeight: 1.5,
            background: msg.role === 'user' ? 'var(--surface2)' : 'var(--bg)',
            border: '1px solid var(--border)',
          }}>
            <div style={{ fontSize: 10, color: 'var(--text-muted)', textTransform: 'uppercase', marginBottom: 4, letterSpacing: '0.5px' }}>{msg.role}</div>
            {msg.role === 'assistant'
              ? <div className="md"><ReactMarkdown>{msg.content}</ReactMarkdown></div>
              : <span>{msg.content}</span>}
          </div>
        ))}
        {!loading && messages.length === 0 && (
          <span style={{ color: 'var(--text-muted)', fontSize: 12 }}>No messages in this session.</span>
        )}
      </div>
    </div>
  )
}
