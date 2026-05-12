import { useState, useEffect, useRef } from 'react'
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogFooter } from "@/components/ui/dialog"
import { Textarea } from "@/components/ui/textarea"
import { ScrollArea } from "@/components/ui/scroll-area"
import { Settings, Send, SquareSquare, Moon, Sun, Trash2, Wrench, Activity, ChevronRight, ChevronDown } from 'lucide-react'
import DOMPurify from 'dompurify'
import { marked } from 'marked'

// Helper for rendering markdown
const renderMd = (text: string) => {
  return { __html: DOMPurify.sanitize(marked.parse(text) as string) }
}

export default function App() {
  const [agents, setAgents] = useState<{id: string, name: string, context_window?: number}[]>([])
  const [sessions, setSessions] = useState<{key: string, entryCount: number}[]>([])
  const [skills, setSkills] = useState<{name: string}[]>([])
  
  const [activeAgent, setActiveAgent] = useState('default')
  const [activeSession, setActiveSession] = useState('ws_default')
  
  const [messages, setMessages] = useState<any[]>([])
  const [input, setInput] = useState('')
  const [images, setImages] = useState<any[]>([])
  
  const [isModalOpen, setIsModalOpen] = useState(false)
  const [newSessionName, setNewSessionName] = useState('')
  
  const [ws, setWs] = useState<WebSocket | null>(null)
  const [sending, setSending] = useState(false)
  const [status, setStatus] = useState('connecting...')
  const scrollRef = useRef<HTMLDivElement>(null)

  const [toolsVisible, setToolsVisible] = useState(true)
  const [traceVisible, setTraceVisible] = useState(false)
  const [theme, setTheme] = useState(localStorage.getItem('theme') || 'light')
  const [traces, setTraces] = useState<any[]>([])
  const [usage, setUsage] = useState<number | null>(null)

  useEffect(() => {
    if (theme === 'dark') document.documentElement.classList.add('dark')
    else document.documentElement.classList.remove('dark')
    localStorage.setItem('theme', theme)
  }, [theme])

  const connect = () => {
    setStatus('connecting...')
    const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:'
    const host = window.location.host === 'localhost:5173' ? '127.0.0.1:18789' : window.location.host
    const socket = new WebSocket(`${protocol}//${host}/ws`)
    
    socket.onopen = () => {
      setStatus('connected')
      socket.send(JSON.stringify({ jsonrpc: '2.0', method: 'agent.status', params: {}, id: 'agents' }))
    }
    socket.onclose = () => {
      setStatus('disconnected')
      setTimeout(connect, 3000)
    }
    
    socket.onmessage = (e) => {
      try {
        const resp = JSON.parse(e.data)
        if (resp.error) {
          setMessages(p => [...p, { id: Math.random(), role: 'error', text: resp.error.message }])
          setSending(false)
          return
        }
        
        if (resp.id === 'agents') {
          const loadedAgents = resp.result.agents || []
          setAgents(loadedAgents.length > 0 ? loadedAgents : [{id: 'default', name: 'default'}])
          socket.send(JSON.stringify({ jsonrpc: '2.0', method: 'session.list', params: { agentId: activeAgent }, id: 'sessions' }))
        } else if (resp.id === 'sessions') {
          const loadedSessions = resp.result.sessions || []
          setSessions(loadedSessions)
          if (loadedSessions.length === 0) setSessions([{key: 'ws_default', entryCount: 0}])
          socket.send(JSON.stringify({ jsonrpc: '2.0', method: 'session.history', params: { agentId: activeAgent, sessionKey: activeSession }, id: 'history' }))
        } else if (resp.id === 'history') {
          const entries = resp.result.entries || []
          const loadedMsgs = entries.map((en: any) => {
            if (en.type === 'message') {
              return { id: Math.random(), role: en.role, text: en.text, images: en.images }
            } else if (en.type === 'tool_call') {
              return { id: en.id, role: 'tool_call', tool: en.tool, input: en.input, status: 'running', isOpen: false }
            } else if (en.type === 'tool_result') {
              return { id: en.tool_call_id, role: 'tool_result', output: en.output, error: en.error, images: en.images }
            }
            return null
          }).filter(Boolean)

          // Merge tool_calls and tool_results
          const merged: any[] = []
          loadedMsgs.forEach((m: any) => {
            if (m.role === 'tool_call') merged.push(m)
            else if (m.role === 'tool_result') {
              const tc = merged.find(x => x.id === m.id && x.role === 'tool_call')
              if (tc) {
                tc.status = m.error ? 'error' : 'done'
                tc.output = m.output
                tc.error = m.error
                tc.resultImages = m.images
              }
            } else merged.push(m)
          })
          setMessages(merged)
        } else if (resp.result?.type === 'text_delta') {
          setMessages(prev => {
            const last = prev[prev.length - 1]
            if (last && last.role === 'assistant' && !last.final) {
              const updated = [...prev]
              updated[updated.length - 1] = { ...last, text: last.text + resp.result.text }
              return updated
            }
            return [...prev, { id: Math.random(), role: 'assistant', text: resp.result.text, final: false }]
          })
        } else if (resp.result?.type === 'tool_call_start') {
          setMessages(prev => [...prev, { id: resp.result.id, role: 'tool_call', tool: resp.result.tool, input: resp.result.input, status: 'running', isOpen: false }])
        } else if (resp.result?.type === 'tool_result') {
          setMessages(prev => {
            const updated = [...prev]
            const tc = updated.find(x => x.id === resp.result.id && x.role === 'tool_call')
            if (tc) {
              tc.status = resp.result.error ? 'error' : 'done'
              tc.output = resp.result.output
              tc.error = resp.result.error
              tc.resultImages = resp.result.images
            }
            return updated
          })
        } else if (resp.result?.type === 'trace') {
           setTraces(p => [...p, resp.result])
        } else if (resp.result?.type === 'done' || resp.result?.type === 'aborted') {
          setSending(false)
          if (resp.result.usage) setUsage(resp.result.usage)
          setMessages(prev => {
            const last = prev[prev.length - 1]
            if (last && last.role === 'assistant') {
              const updated = [...prev]
              updated[updated.length - 1] = { ...last, final: true }
              return updated
            }
            return prev
          })
        } else if (resp.id && resp.id.toString().startsWith('session-new')) {
           setActiveSession(resp.result.sessionKey)
           socket.send(JSON.stringify({ jsonrpc: '2.0', method: 'session.list', params: { agentId: activeAgent }, id: 'sessions' }))
        } else if (resp.id === 'clear') {
           setMessages([])
           setTraces([])
           setUsage(null)
        }
      } catch (err) {
        console.error(err)
      }
    }
    setWs(socket)
  }

  useEffect(() => {
    connect()
    const apiHost = window.location.host === 'localhost:5173' ? 'http://127.0.0.1:18789' : ''
    fetch(`${apiHost}/settings/api/skills`).then(r=>r.json()).then(d => setSkills(d.skills || []))
    return () => { if(ws) ws.close() }
  }, [])

  useEffect(() => {
    if (scrollRef.current) {
      const el = scrollRef.current.querySelector('[data-radix-scroll-area-viewport]')
      if (el) el.scrollTop = el.scrollHeight
    }
  }, [messages])

  const sendMsg = () => {
    if (!input.trim() && images.length === 0) return
    if (!ws || ws.readyState !== WebSocket.OPEN) return
    
    const msgImages = images.map(img => ({ mimeType: img.type, data: img.data.split(',')[1] }))
    setMessages(prev => [...prev, { id: Math.random(), role: 'user', text: input, images: msgImages }])
    
    ws.send(JSON.stringify({
      jsonrpc: '2.0',
      method: 'chat.send',
      params: { agentId: activeAgent, sessionKey: activeSession, text: input, images: msgImages },
      id: Date.now()
    }))
    
    setInput('')
    setImages([])
    setSending(true)
  }

  const handlePaste = (e: React.ClipboardEvent) => {
    const items = e.clipboardData?.items
    if (!items) return
    for (let i = 0; i < items.length; i++) {
      if (items[i].type.indexOf('image') !== -1) {
        const file = items[i].getAsFile()
        if (file) {
          const reader = new FileReader()
          reader.onload = (evt) => {
            setImages(prev => [...prev, { type: file.type, data: evt.target?.result, name: file.name }])
          }
          reader.readAsDataURL(file)
        }
      }
    }
  }

  const toggleToolOpen = (id: string) => {
    setMessages(prev => prev.map(m => m.id === id && m.role === 'tool_call' ? { ...m, isOpen: !m.isOpen } : m))
  }

  const activeAgentLimit = agents.find(a => a.id === activeAgent)?.context_window || '?'

  return (
    <div className="flex flex-col h-screen bg-background text-foreground font-sans overflow-hidden">
      {/* Header */}
      <header className="flex items-center gap-4 px-6 py-3 border-b bg-background z-10 shrink-0">
        <h1 className="text-xl font-bold tracking-tight m-0">robin</h1>
        <Select value={activeAgent} onValueChange={(val) => {
          setActiveAgent(val)
          if(ws) ws.send(JSON.stringify({ jsonrpc: '2.0', method: 'session.list', params: { agentId: val }, id: 'sessions' }))
        }}>
          <SelectTrigger className="w-[160px] h-8 text-sm"><SelectValue placeholder="Agent" /></SelectTrigger>
          <SelectContent>
            {agents.map(a => <SelectItem key={a.id} value={a.id}>{a.name}</SelectItem>)}
          </SelectContent>
        </Select>
        <Select value={activeSession} onValueChange={(val) => {
          setActiveSession(val)
          if(ws) ws.send(JSON.stringify({ jsonrpc: '2.0', method: 'session.history', params: { agentId: activeAgent, sessionKey: val }, id: 'history' }))
        }}>
          <SelectTrigger className="w-[180px] h-8 text-sm"><SelectValue placeholder="Session" /></SelectTrigger>
          <SelectContent>
            {sessions.map(s => <SelectItem key={s.key} value={s.key}>{s.key} ({s.entryCount})</SelectItem>)}
          </SelectContent>
        </Select>
        <Button variant="outline" size="sm" className="h-8" onClick={() => setIsModalOpen(true)}>+ New</Button>
        <div className="flex-1"></div>
        
        <div className="text-xs text-muted-foreground bg-muted px-3 py-1 rounded-full border font-mono">
          {usage !== null ? `${usage} / ${activeAgentLimit}` : '—'}
        </div>
        
        <Button variant={toolsVisible ? "secondary" : "ghost"} size="sm" className="h-8 gap-2" onClick={() => setToolsVisible(!toolsVisible)}>
          <Wrench className="w-4 h-4" /> Tools
        </Button>
        <Button variant={traceVisible ? "secondary" : "ghost"} size="sm" className="h-8 gap-2" onClick={() => setTraceVisible(!traceVisible)}>
          <Activity className="w-4 h-4" /> Trace
        </Button>
        <Button variant="ghost" size="sm" className="h-8 gap-2" onClick={() => window.open('/settings', '_blank')}>
          <Settings className="w-4 h-4" /> Settings
        </Button>
        <Button variant="ghost" size="sm" className="h-8 gap-2" onClick={() => {
          if(confirm('Clear session history?')) {
            ws?.send(JSON.stringify({ jsonrpc: '2.0', method: 'session.clear', params: { agentId: activeAgent, sessionKey: activeSession }, id: 'clear' }))
          }
        }}>
          <Trash2 className="w-4 h-4" /> Clear
        </Button>
        <Button variant="ghost" size="icon" className="h-8 w-8" onClick={() => setTheme(theme === 'dark' ? 'light' : 'dark')}>
          {theme === 'dark' ? <Moon className="w-4 h-4" /> : <Sun className="w-4 h-4" />}
        </Button>
        <div className="flex items-center gap-2 text-xs text-muted-foreground ml-2">
          <div className={`w-2 h-2 rounded-full ${status === 'connected' ? 'bg-green-500' : 'bg-red-500'}`}></div>
          {status}
        </div>
      </header>

      {/* Main Layout */}
      <div className="flex-1 flex overflow-hidden relative">
        
        {/* Chat Area */}
        <div className="flex-1 flex flex-col min-w-0">
          <ScrollArea className="flex-1 px-4 py-6" ref={scrollRef}>
            <div className="max-w-3xl mx-auto flex flex-col gap-6 pb-4">
              {messages.map((m, i) => {
                if (m.role === 'user') {
                  return (
                    <div key={i} className="flex justify-end">
                      <div className="max-w-[85%] rounded-2xl rounded-tr-sm p-4 bg-primary text-primary-foreground shadow-sm">
                        <div className="whitespace-pre-wrap">{m.text}</div>
                        {m.images && m.images.length > 0 && (
                          <div className="flex gap-2 mt-3 flex-wrap">
                            {m.images.map((img: any, idx: number) => (
                              <img key={idx} src={`data:${img.mimeType};base64,${img.data}`} className="max-w-[250px] max-h-[250px] rounded object-cover border border-primary/20" />
                            ))}
                          </div>
                        )}
                      </div>
                    </div>
                  )
                } else if (m.role === 'assistant') {
                  return (
                    <div key={i} className="flex justify-start">
                      <div className="max-w-full rounded-2xl p-4 markdown-body w-full" dangerouslySetInnerHTML={renderMd(m.text)} />
                    </div>
                  )
                } else if (m.role === 'tool_call' && toolsVisible) {
                  return (
                    <div key={m.id} className="max-w-3xl w-full mx-auto border rounded-lg bg-muted/30 overflow-hidden font-mono text-sm my-2">
                      <div className="flex justify-between items-center px-4 py-2 bg-muted/50 cursor-pointer hover:bg-muted transition-colors" onClick={() => toggleToolOpen(m.id)}>
                        <div className="flex items-center gap-2 text-muted-foreground font-semibold">
                          {m.isOpen ? <ChevronDown className="w-4 h-4"/> : <ChevronRight className="w-4 h-4"/>}
                          {m.tool}
                        </div>
                        <div className={`${m.status === 'running' ? 'text-blue-500' : m.status === 'error' ? 'text-red-500' : 'text-green-500'}`}>
                          {m.status}
                        </div>
                      </div>
                      {m.isOpen && (
                        <div className="p-4 border-t text-muted-foreground whitespace-pre-wrap break-all">
                          <div className="mb-4"><strong className="text-foreground">Input:</strong><br/>{JSON.stringify(m.input, null, 2)}</div>
                          {m.output && <div><strong className="text-foreground">Output:</strong><br/>{m.output}</div>}
                          {m.error && <div className="text-red-500"><strong className="text-foreground">Error:</strong><br/>{m.error}</div>}
                          {m.resultImages && m.resultImages.length > 0 && (
                            <div className="flex gap-2 mt-2 flex-wrap">
                              {m.resultImages.map((img: any, idx: number) => (
                                <img key={idx} src={`data:${img.mimeType};base64,${img.data}`} className="max-w-[200px] max-h-[200px] rounded border" />
                              ))}
                            </div>
                          )}
                        </div>
                      )}
                    </div>
                  )
                } else if (m.role === 'error') {
                  return (
                    <div key={i} className="max-w-3xl w-full mx-auto p-4 border border-red-500/50 bg-red-500/10 text-red-500 rounded-lg my-2">
                      Error: {m.text}
                    </div>
                  )
                }
                return null
              })}
            </div>
          </ScrollArea>

          {/* Input Area */}
          <div className="p-4 border-t bg-background shrink-0">
            <div className="max-w-3xl mx-auto flex flex-col gap-3">
              {images.length > 0 && (
                <div className="flex gap-3 overflow-x-auto pb-1">
                  {images.map((img, i) => (
                    <div key={i} className="relative group shrink-0">
                      <img src={img.data} className="h-16 w-16 object-cover rounded-md border shadow-sm" />
                      <button onClick={() => setImages(images.filter((_, idx) => idx !== i))} className="absolute -top-2 -right-2 bg-destructive text-white rounded-full w-5 h-5 flex items-center justify-center text-xs opacity-0 group-hover:opacity-100 transition-opacity shadow-sm">×</button>
                    </div>
                  ))}
                </div>
              )}
              <div className="flex gap-3 items-end">
                <Textarea 
                  value={input}
                  onChange={e => setInput(e.target.value)}
                  onKeyDown={e => { if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); sendMsg() } }}
                  onPaste={handlePaste}
                  placeholder="Type a message... (paste or drop an image to analyze)"
                  className="min-h-[60px] max-h-[300px] resize-y shadow-sm"
                />
                <div className="flex flex-col gap-2 shrink-0">
                  {sending ? (
                    <Button variant="destructive" className="h-10 px-6" onClick={() => {
                      ws?.send(JSON.stringify({ jsonrpc: '2.0', method: 'chat.abort', params: {}, id: 'abort' }))
                      setSending(false)
                    }}><SquareSquare className="w-4 h-4 mr-2"/> Stop</Button>
                  ) : (
                    <Button className="h-10 px-6" onClick={sendMsg} disabled={!input.trim() && images.length === 0}><Send className="w-4 h-4 mr-2"/> Send</Button>
                  )}
                </div>
              </div>
            </div>
          </div>
        </div>

        {/* Trace Panel */}
        {traceVisible && (
          <div className="w-[400px] border-l bg-[#1e1e1e] text-[#d4d4d4] flex flex-col font-mono text-xs shrink-0 shadow-[-4px_0_15px_rgba(0,0,0,0.1)] z-20 absolute right-0 top-0 bottom-0">
            <div className="p-3 border-b border-[#333] flex justify-between items-center bg-black text-white font-semibold">
              <span>Live trace</span>
              <button className="px-2 py-1 border border-[#333] rounded hover:bg-[#333] transition-colors" onClick={() => setTraces([])}>clear</button>
            </div>
            <ScrollArea className="flex-1 p-2">
              <div className="flex flex-col gap-1">
                {traces.map((r, i) => {
                  const t = new Date(r.timestamp)
                  const timeStr = `${t.getHours().toString().padStart(2,'0')}:${t.getMinutes().toString().padStart(2,'0')}:${t.getSeconds().toString().padStart(2,'0')}`
                  const isSlow = r.duration_ms && r.duration_ms > 1500
                  return (
                    <div key={i} className="hover:bg-[#222] p-1 rounded">
                      <div className="flex justify-between">
                        <div className="text-[#888] w-[50px] shrink-0">{timeStr}</div>
                        <div className="flex-1 text-[#4ade80] truncate mx-2" title={r.event}>{r.event}</div>
                        {r.duration_ms !== undefined && (
                          <div className={`w-[60px] text-right shrink-0 ${isSlow ? 'text-red-500 font-bold' : 'text-[#aaa]'}`}>{r.duration_ms}ms</div>
                        )}
                      </div>
                      {r.metadata && Object.keys(r.metadata).length > 0 && (
                        <div className="text-[#666] mt-0.5 pl-[58px] break-all text-[10px]">
                          {JSON.stringify(r.metadata)}
                        </div>
                      )}
                    </div>
                  )
                })}
              </div>
            </ScrollArea>
          </div>
        )}
      </div>

      {/* New Session Modal */}
      <Dialog open={isModalOpen} onOpenChange={setIsModalOpen}>
        <DialogContent className="sm:max-w-[425px]">
          <DialogHeader><DialogTitle>New Session</DialogTitle></DialogHeader>
          <div className="space-y-4 py-4">
            <div className="space-y-2">
              <label className="text-sm font-medium">Session Name</label>
              <Input value={newSessionName} onChange={e => setNewSessionName(e.target.value)} placeholder="Leave empty for timestamp" />
            </div>
            <div className="space-y-2">
              <label className="text-sm font-medium">Attach Skills (optional)</label>
              <div className="border rounded-md p-3 max-h-[160px] overflow-y-auto flex flex-col gap-3 bg-muted/50">
                {skills.map(s => (
                  <label key={s.name} className="flex items-center gap-3 cursor-pointer">
                    <input type="checkbox" value={s.name} className="skill-cb w-4 h-4 rounded border-gray-300 text-primary focus:ring-primary" />
                    <span className="text-sm font-medium leading-none peer-disabled:cursor-not-allowed peer-disabled:opacity-70">{s.name}</span>
                  </label>
                ))}
              </div>
            </div>
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setIsModalOpen(false)}>Cancel</Button>
            <Button onClick={() => {
              const cbs = document.querySelectorAll('.skill-cb:checked') as NodeListOf<HTMLInputElement>
              const selected = Array.from(cbs).map(cb => cb.value)
              const reqId = selected.length > 0 ? 'session-new-skills:' + selected.join(',') : 'session-new'
              ws?.send(JSON.stringify({ jsonrpc: '2.0', method: 'session.new', params: { agentId: activeAgent, name: newSessionName }, id: reqId }))
              setIsModalOpen(false)
              if (selected.length > 0) setInput(`Please use the \`${selected.join('`, `')}\` skills to \n\n`)
            }}>Create</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  )
}
