import { useState, useRef, useEffect } from 'react'
import ReactMarkdown from 'react-markdown'
import { LocalChatMessage, ToolCallItem } from '../../types'
import { streamChat } from '../../services/api'

interface Props {
  history: LocalChatMessage[]
  sessionId?: string
  onHistoryUpdate: (history: LocalChatMessage[]) => void
}

export function ChatCard({ history, onHistoryUpdate }: Props) {
  const [input, setInput] = useState('')
  const [streaming, setStreaming] = useState(false)
  const [streamBuf, setStreamBuf] = useState('')
  const [localHistory, setLocalHistory] = useState<LocalChatMessage[]>(history)
  const bottomRef = useRef<HTMLDivElement>(null)

  useEffect(() => { setLocalHistory(history) }, [history])

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: 'smooth' })
  }, [localHistory, streamBuf])

  const send = async () => {
    const text = input.trim()
    if (!text || streaming) return
    setInput('')

    const userMsg: LocalChatMessage = { role: 'user', content: text }
    const updated = [...localHistory, userMsg]
    setLocalHistory(updated)
    setStreaming(true)
    setStreamBuf('')

    const apiMessages = updated.map(m => ({ role: m.role, content: m.content }))
    let finalContent = ''
    const pendingTools: ToolCallItem[] = []

    await streamChat(apiMessages, event => {
      if (event.type === 'token') {
        finalContent += event.content
        setStreamBuf(finalContent)
      } else if (event.type === 'tool_start') {
        pendingTools.push({ name: event.name, status: 'running' })
      } else if (event.type === 'tool_result') {
        const t = pendingTools.find(p => p.status === 'running')
        if (t) { t.status = event.isError ? 'error' : 'done'; t.output = event.content; t.isError = event.isError }
      } else if (event.type === 'error') {
        finalContent = finalContent || `Error: ${event.message}`
        setStreamBuf(finalContent)
      }
    })

    const assistantMsg: LocalChatMessage = {
      role: 'assistant',
      content: finalContent,
      toolCalls: pendingTools.length > 0 ? pendingTools : undefined,
    }
    const final = [...updated, assistantMsg]
    setLocalHistory(final)
    onHistoryUpdate(final)
    setStreamBuf('')
    setStreaming(false)
  }

  return (
    <div style={{ display: 'flex', flexDirection: 'column', height: '100%' }}>
      <div className="chat-messages">
        {localHistory.map((msg, i) => (
          <div key={i}>
            <div className={`chat-bubble ${msg.role}`}>
              {msg.role === 'assistant'
                ? <div className="md"><ReactMarkdown>{msg.content}</ReactMarkdown></div>
                : msg.content}
            </div>
            {msg.toolCalls?.map((tc, j) => (
              <ToolCall key={j} item={tc} />
            ))}
          </div>
        ))}
        {streamBuf && (
          <div className="chat-bubble assistant streaming">
            <div className="md"><ReactMarkdown>{streamBuf}</ReactMarkdown></div>
          </div>
        )}
        {streaming && !streamBuf && (
          <div className="chat-bubble assistant" style={{ color: 'var(--text-muted)' }}>●●●</div>
        )}
        <div ref={bottomRef} />
      </div>

      <div className="chat-input-row">
        <textarea
          value={input}
          onChange={e => setInput(e.target.value)}
          onKeyDown={e => { if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); void send() } }}
          placeholder="Message the agent… (Enter to send)"
          disabled={streaming}
        />
        <button className="chat-send-btn" onClick={() => void send()} disabled={streaming || !input.trim()}>↑</button>
      </div>
    </div>
  )
}

function ToolCall({ item }: { item: ToolCallItem }) {
  const [open, setOpen] = useState(false)
  return (
    <div className="chat-tool">
      <div className="chat-tool-hd" onClick={() => setOpen(o => !o)}>
        <span className="chat-tool-name">⚙ {item.name}</span>
        <span className={`chat-tool-status ${item.status}`}>{item.status}</span>
      </div>
      {open && item.output && (
        <div className="chat-tool-body">{item.output.slice(0, 800)}{item.output.length > 800 ? '…' : ''}</div>
      )}
    </div>
  )
}
