import { useEffect, useRef } from 'react'
import '@xterm/xterm/css/xterm.css'
import { Terminal } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import { createTerminalSocket } from '../../services/api'

export function TerminalCard() {
  const divRef = useRef<HTMLDivElement>(null)
  const termRef = useRef<Terminal | null>(null)
  const wsRef = useRef<WebSocket | null>(null)

  useEffect(() => {
    if (!divRef.current) return

    const term = new Terminal({
      theme: { background: '#0d0d0d', foreground: '#e6edf3', cursor: '#58a6ff' },
      fontSize: 13,
      fontFamily: "'SFMono-Regular', Consolas, 'Liberation Mono', Menlo, monospace",
      cursorBlink: true,
    })
    const fit = new FitAddon()
    term.loadAddon(fit)
    term.open(divRef.current)
    fit.fit()
    termRef.current = term

    const ws = createTerminalSocket()
    ws.binaryType = 'arraybuffer'
    wsRef.current = ws

    ws.onopen = () => {
      const { cols, rows } = term
      ws.send(JSON.stringify({ type: 'resize', cols, rows }))
    }

    ws.onmessage = e => {
      if (e.data instanceof ArrayBuffer) {
        term.write(new Uint8Array(e.data))
      } else {
        try {
          const msg = JSON.parse(e.data as string) as { type: string; message?: string }
          if (msg.type === 'error') term.write(`\r\n\x1b[31mError: ${msg.message}\x1b[0m\r\n`)
        } catch { /* ignore */ }
      }
    }

    ws.onerror = () => term.write('\r\n\x1b[31mWebSocket error\x1b[0m\r\n')
    ws.onclose = () => term.write('\r\n\x1b[2mConnection closed\x1b[0m\r\n')

    term.onData(data => {
      if (ws.readyState === WebSocket.OPEN) {
        const bytes = new TextEncoder().encode(data)
        const b64 = btoa(String.fromCharCode(...bytes))
        ws.send(JSON.stringify({ type: 'input', data: b64 }))
      }
    })

    const ro = new ResizeObserver(() => {
      fit.fit()
      if (ws.readyState === WebSocket.OPEN) {
        ws.send(JSON.stringify({ type: 'resize', cols: term.cols, rows: term.rows }))
      }
    })
    if (divRef.current) ro.observe(divRef.current)

    return () => {
      ro.disconnect()
      term.dispose()
      ws.close()
    }
  }, [])

  return <div ref={divRef} className="terminal-wrap" />
}
