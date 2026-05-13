import { useState, useEffect, useRef } from 'react'
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogFooter } from "@/components/ui/dialog"
import { Textarea } from "@/components/ui/textarea"
import { ScrollArea } from "@/components/ui/scroll-area"
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip"
import { Settings, Send, SquareSquare, Moon, Sun, Trash2, Wrench, Activity, ChevronRight, ChevronDown, Plus, MessageSquare, Paperclip, X, Menu } from 'lucide-react'
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
  const fileInputRef = useRef<HTMLInputElement>(null)

  const [toolsVisible, setToolsVisible] = useState(localStorage.getItem('toolsVisible') !== 'false')
  const [traceVisible, setTraceVisible] = useState(localStorage.getItem('traceVisible') === 'true')
  const [sidebarOpen, setSidebarOpen] = useState(true)
  const [theme, setTheme] = useState(localStorage.getItem('theme') || 'light')
  const [traces, setTraces] = useState<any[]>([])
  const [usage, setUsage] = useState<number | null>(null)

  useEffect(() => {
    if (theme === 'dark') document.documentElement.classList.add('dark')
    else document.documentElement.classList.remove('dark')
    localStorage.setItem('theme', theme)
  }, [theme])

  useEffect(() => {
    localStorage.setItem('toolsVisible', toolsVisible.toString())
  }, [toolsVisible])

  useEffect(() => {
    localStorage.setItem('traceVisible', traceVisible.toString())
  }, [traceVisible])

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
    fetch(`/settings/api/skills`).then(r=>r.json()).then(d => setSkills(d.skills || []))
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

  const handleFileSelect = (e: React.ChangeEvent<HTMLInputElement>) => {
    const files = e.target.files
    if (!files) return
    for (let i = 0; i < files.length; i++) {
      const file = files[i]
      if (file.type.startsWith('image/')) {
        const reader = new FileReader()
        reader.onload = (evt) => {
          setImages(prev => [...prev, { type: file.type, data: evt.target?.result, name: file.name }])
        }
        reader.readAsDataURL(file)
      }
    }
    if (fileInputRef.current) fileInputRef.current.value = ''
  }

  const toggleToolOpen = (id: string) => {
    setMessages(prev => prev.map(m => m.id === id && m.role === 'tool_call' ? { ...m, isOpen: !m.isOpen } : m))
  }

  const activeAgentLimit = agents.find(a => a.id === activeAgent)?.context_window || '?'

  return (
    <TooltipProvider>
      <div className="flex h-screen bg-background text-foreground font-sans overflow-hidden">
        
        {/* Left Sidebar */}
        <div className={`${sidebarOpen ? 'w-[260px] border-r flex flex-col bg-muted/20' : 'w-0 hidden'} transition-all duration-300 overflow-hidden shrink-0`}>
          <div className="p-3">
            <Button className="w-full justify-start gap-2 h-10 px-3 bg-background hover:bg-muted border shadow-sm" variant="outline" onClick={() => setIsModalOpen(true)}>
              <Plus className="w-4 h-4" />
              <span className="font-semibold">New chat</span>
            </Button>
          </div>
          
          <ScrollArea className="flex-1 px-3">
            <div className="flex flex-col gap-1 pb-4 mt-2">
              <div className="text-xs font-semibold text-muted-foreground px-2 py-1 mb-1">Today</div>
              {sessions.map(s => (
                <div key={s.key} className="relative group">
                  <Button 
                    variant={activeSession === s.key ? "secondary" : "ghost"} 
                    className={`w-full justify-start px-2 h-9 font-normal pr-8 ${activeSession === s.key ? 'bg-muted hover:bg-muted/80' : 'hover:bg-muted/50'}`}
                    onClick={() => {
                      setActiveSession(s.key)
                      if(ws) ws.send(JSON.stringify({ jsonrpc: '2.0', method: 'session.history', params: { agentId: activeAgent, sessionKey: s.key }, id: 'history' }))
                    }}
                  >
                    <MessageSquare className="w-4 h-4 mr-2 opacity-70 shrink-0" />
                    <span className="truncate">{s.key.startsWith('ws_') ? s.key.slice(3) : s.key.startsWith('ws') ? s.key.slice(2) : s.key}</span>
                  </Button>
                  <Button
                    variant="ghost"
                    size="icon"
                    className="absolute right-1 top-1 h-7 w-7 opacity-0 group-hover:opacity-100 hover:text-red-500 hover:bg-red-500/10 transition-all"
                    onClick={(e) => {
                      e.stopPropagation()
                      if (confirm(`Delete chat "${s.key.startsWith('ws_') ? s.key.slice(3) : s.key.startsWith('ws') ? s.key.slice(2) : s.key}"?`)) {
                        ws?.send(JSON.stringify({ jsonrpc: '2.0', method: 'session.clear', params: { agentId: activeAgent, sessionKey: s.key }, id: 'clear-session' }))
                        if (activeSession === s.key) {
                          setActiveSession('ws_default')
                          setMessages([])
                        }
                        // Refresh list
                        setTimeout(() => {
                          ws?.send(JSON.stringify({ jsonrpc: '2.0', method: 'session.list', params: { agentId: activeAgent }, id: 'sessions' }))
                        }, 100)
                      }
                    }}
                  >
                    <Trash2 className="w-3.5 h-3.5" />
                  </Button>
                </div>
              ))}
            </div>
          </ScrollArea>
          
          <div className="p-3 border-t bg-background/50 flex flex-col gap-1">
            <Button variant="ghost" className="w-full justify-start h-10 px-2 font-normal" asChild>
              <a href="/settings" target="_blank" rel="noopener noreferrer">
                <Settings className="w-4 h-4 mr-3 opacity-70" /> Settings
              </a>
            </Button>
          </div>
        </div>

        {/* Main Content */}
        <div className="flex-1 flex flex-col min-w-0 relative bg-background">
          
          {/* Top Bar */}
          <header className="h-14 flex items-center justify-between px-4 sticky top-0 bg-background/95 backdrop-blur z-10">
            <div className="flex items-center gap-2">
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button variant="ghost" size="icon" className="h-9 w-9 text-muted-foreground hover:text-foreground" onClick={() => setSidebarOpen(!sidebarOpen)}>
                    <Menu className="w-5 h-5" />
                  </Button>
                </TooltipTrigger>
                <TooltipContent side="right">Toggle sidebar</TooltipContent>
              </Tooltip>
              
              <Select value={activeAgent} onValueChange={(val) => {
                setActiveAgent(val)
                if(ws) ws.send(JSON.stringify({ jsonrpc: '2.0', method: 'session.list', params: { agentId: val }, id: 'sessions' }))
              }}>
                <SelectTrigger className="w-[180px] h-9 border-none bg-transparent hover:bg-muted/50 focus:ring-0 shadow-none font-semibold text-base px-2">
                  <SelectValue placeholder="Agent" />
                </SelectTrigger>
                <SelectContent>
                  {agents.map(a => <SelectItem key={a.id} value={a.id} className="cursor-pointer">{a.name}</SelectItem>)}
                </SelectContent>
              </Select>
            </div>
            
            <div className="flex items-center gap-1.5">
              <Tooltip>
                <TooltipTrigger asChild>
                  <div className="flex items-center gap-1.5 px-2 py-1 bg-muted/50 rounded-md text-xs font-mono text-muted-foreground mr-2 cursor-help">
                    <div className={`w-1.5 h-1.5 rounded-full ${status === 'connected' ? 'bg-green-500' : 'bg-red-500'}`}></div>
                    {usage !== null ? `${usage}/${activeAgentLimit}` : '—'}
                  </div>
                </TooltipTrigger>
                <TooltipContent>Tokens used / Context window</TooltipContent>
              </Tooltip>

              <Tooltip>
                <TooltipTrigger asChild>
                  <Button variant={toolsVisible ? "secondary" : "ghost"} size="icon" className="h-9 w-9 rounded-full" onClick={() => setToolsVisible(!toolsVisible)}>
                    <Wrench className="w-4 h-4" />
                  </Button>
                </TooltipTrigger>
                <TooltipContent>Toggle Tools Display</TooltipContent>
              </Tooltip>

              <Tooltip>
                <TooltipTrigger asChild>
                  <Button variant={traceVisible ? "secondary" : "ghost"} size="icon" className="h-9 w-9 rounded-full" onClick={() => setTraceVisible(!traceVisible)}>
                    <Activity className="w-4 h-4" />
                  </Button>
                </TooltipTrigger>
                <TooltipContent>Toggle Trace Panel</TooltipContent>
              </Tooltip>

              <Tooltip>
                <TooltipTrigger asChild>
                  <Button variant="ghost" size="icon" className="h-9 w-9 rounded-full" onClick={() => {
                    if(confirm('Clear session history?')) {
                      ws?.send(JSON.stringify({ jsonrpc: '2.0', method: 'session.clear', params: { agentId: activeAgent, sessionKey: activeSession }, id: 'clear' }))
                    }
                  }}>
                    <Trash2 className="w-4 h-4 text-muted-foreground hover:text-red-500" />
                  </Button>
                </TooltipTrigger>
                <TooltipContent>Clear Chat</TooltipContent>
              </Tooltip>

              <Tooltip>
                <TooltipTrigger asChild>
                  <Button variant="ghost" size="icon" className="h-9 w-9 rounded-full" onClick={() => setTheme(theme === 'dark' ? 'light' : 'dark')}>
                    {theme === 'dark' ? <Moon className="w-4 h-4 text-muted-foreground" /> : <Sun className="w-4 h-4 text-muted-foreground" />}
                  </Button>
                </TooltipTrigger>
                <TooltipContent>Toggle Theme</TooltipContent>
              </Tooltip>
            </div>
          </header>

          {/* Chat Messages */}
          <ScrollArea className="flex-1" ref={scrollRef}>
            <div className="max-w-3xl mx-auto flex flex-col gap-6 py-6 px-4 pb-48">
              {messages.length === 0 && !sending && (
                <div className="flex flex-col items-center justify-center h-[50vh] text-center opacity-50">
                  <div className="w-16 h-16 rounded-2xl bg-primary text-primary-foreground flex items-center justify-center mb-6 shadow-sm">
                    <span className="text-3xl font-bold font-serif">r</span>
                  </div>
                  <h2 className="text-2xl font-semibold mb-2">How can I help you today?</h2>
                </div>
              )}
              
              {messages.map((m, i) => {
                if (m.role === 'user') {
                  return (
                    <div key={i} className="flex justify-end mb-2">
                      <div className="max-w-[85%] rounded-2xl bg-muted px-5 py-3.5 text-[15px] leading-relaxed">
                        <div className="whitespace-pre-wrap">{m.text}</div>
                        {m.images && m.images.length > 0 && (
                          <div className="flex gap-2 mt-3 flex-wrap">
                            {m.images.map((img: any, idx: number) => (
                              <img key={idx} src={`data:${img.mimeType};base64,${img.data}`} className="max-w-[300px] max-h-[300px] rounded-lg object-cover border border-border/50 shadow-sm" />
                            ))}
                          </div>
                        )}
                      </div>
                    </div>
                  )
                } else if (m.role === 'assistant') {
                  return (
                    <div key={i} className="flex justify-start mb-4">
                      <div className="max-w-full w-full pr-4">
                        <div className="markdown-body text-[15px] leading-relaxed" dangerouslySetInnerHTML={renderMd(m.text)} />
                      </div>
                    </div>
                  )
                } else if (m.role === 'tool_call' && toolsVisible) {
                  return (
                    <div key={m.id} className="max-w-full w-full mb-4">
                      <div className="border rounded-xl bg-muted/30 overflow-hidden font-mono text-[13px]">
                        <div className="flex justify-between items-center px-4 py-2.5 bg-muted/50 cursor-pointer hover:bg-muted transition-colors" onClick={() => toggleToolOpen(m.id)}>
                          <div className="flex items-center gap-2 text-muted-foreground font-semibold">
                            {m.isOpen ? <ChevronDown className="w-4 h-4"/> : <ChevronRight className="w-4 h-4"/>}
                            {m.tool}
                          </div>
                          <div className={`${m.status === 'running' ? 'text-blue-500 animate-pulse' : m.status === 'error' ? 'text-red-500' : 'text-green-600'}`}>
                            {m.status}
                          </div>
                        </div>
                        {m.isOpen && (
                          <div className="p-4 border-t border-border/50 text-muted-foreground whitespace-pre-wrap break-all bg-background/50">
                            <div className="mb-4"><strong className="text-foreground">Input:</strong><br/>{JSON.stringify(m.input, null, 2)}</div>
                            {m.output && <div><strong className="text-foreground">Output:</strong><br/>{m.output}</div>}
                            {m.error && <div className="text-red-500"><strong className="text-foreground">Error:</strong><br/>{m.error}</div>}
                            {m.resultImages && m.resultImages.length > 0 && (
                              <div className="flex gap-2 mt-3 flex-wrap">
                                {m.resultImages.map((img: any, idx: number) => (
                                  <img key={idx} src={`data:${img.mimeType};base64,${img.data}`} className="max-w-[200px] max-h-[200px] rounded-lg border shadow-sm" />
                                ))}
                              </div>
                            )}
                          </div>
                        )}
                      </div>
                    </div>
                  )
                } else if (m.role === 'error') {
                  return (
                    <div key={i} className="max-w-full w-full p-4 border border-red-500/30 bg-red-500/10 text-red-600 rounded-xl my-2 text-[15px]">
                      Error: {m.text}
                    </div>
                  )
                }
                return null
              })}
            </div>
          </ScrollArea>

          {/* Input Area */}
          <div className="absolute bottom-0 left-0 right-0 bg-gradient-to-t from-background via-background/90 to-transparent pt-10 pb-6 px-4">
            <div className="max-w-3xl mx-auto">
              <div className="relative bg-muted/40 border rounded-[24px] p-2 shadow-sm focus-within:ring-1 focus-within:ring-border focus-within:bg-background transition-all">
                {images.length > 0 && (
                  <div className="flex gap-3 overflow-x-auto p-2 pb-0">
                    {images.map((img, i) => (
                      <div key={i} className="relative group shrink-0">
                        <div className="w-16 h-16 rounded-xl overflow-hidden border bg-background shadow-sm">
                          <img src={img.data} className="w-full h-full object-cover" />
                        </div>
                        <button onClick={() => setImages(images.filter((_, idx) => idx !== i))} className="absolute -top-2 -right-2 bg-muted-foreground/80 hover:bg-destructive text-white rounded-full w-6 h-6 flex items-center justify-center text-xs opacity-0 group-hover:opacity-100 transition-all shadow-sm">
                          <X className="w-3 h-3" />
                        </button>
                      </div>
                    ))}
                  </div>
                )}
                
                <div className="flex items-end gap-2 px-1">
                  <input type="file" accept="image/*" multiple ref={fileInputRef} className="hidden" onChange={handleFileSelect} />
                  <Tooltip>
                    <TooltipTrigger asChild>
                      <Button variant="ghost" size="icon" className="shrink-0 rounded-full h-10 w-10 text-muted-foreground hover:text-foreground mb-0.5" onClick={() => fileInputRef.current?.click()}>
                        <Paperclip className="w-5 h-5" />
                      </Button>
                    </TooltipTrigger>
                    <TooltipContent>Attach images</TooltipContent>
                  </Tooltip>

                  <Textarea 
                    value={input}
                    onChange={e => setInput(e.target.value)}
                    onKeyDown={e => { if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); sendMsg() } }}
                    onPaste={handlePaste}
                    placeholder="Message Robin..."
                    className="flex-1 min-h-[44px] max-h-[200px] resize-none border-0 focus-visible:ring-0 bg-transparent px-2 py-3 shadow-none text-[15px]"
                  />
                  
                  {sending ? (
                    <Tooltip>
                      <TooltipTrigger asChild>
                        <Button variant="default" size="icon" className="shrink-0 rounded-full h-10 w-10 mb-0.5 bg-foreground hover:bg-foreground/90" onClick={() => {
                          ws?.send(JSON.stringify({ jsonrpc: '2.0', method: 'chat.abort', params: {}, id: 'abort' }))
                          setSending(false)
                        }}>
                          <SquareSquare className="w-4 h-4 fill-background text-background" />
                        </Button>
                      </TooltipTrigger>
                      <TooltipContent>Stop generating</TooltipContent>
                    </Tooltip>
                  ) : (
                    <Tooltip>
                      <TooltipTrigger asChild>
                        <Button size="icon" className={`shrink-0 rounded-full h-10 w-10 mb-0.5 transition-all ${input.trim() || images.length > 0 ? 'bg-primary hover:bg-primary/90 text-primary-foreground' : 'bg-muted text-muted-foreground'}`} onClick={sendMsg} disabled={!input.trim() && images.length === 0}>
                          <Send className="w-4 h-4" />
                        </Button>
                      </TooltipTrigger>
                      <TooltipContent>Send message</TooltipContent>
                    </Tooltip>
                  )}
                </div>
              </div>
              <div className="text-center text-xs text-muted-foreground mt-3">
                Robin can make mistakes. Consider verifying important information.
              </div>
            </div>
          </div>
        </div>

        {/* Trace Panel (Right Sidebar) */}
        {traceVisible && (
          <div className="w-[340px] border-l bg-[#1e1e1e] text-[#d4d4d4] flex flex-col font-mono text-[11px] shrink-0 shadow-[-4px_0_24px_rgba(0,0,0,0.1)] z-20 absolute right-0 top-0 bottom-0">
            <div className="p-3 border-b border-[#333] flex justify-between items-center bg-black text-white font-semibold">
              <div className="flex items-center gap-2">
                <Activity className="w-4 h-4 text-[#4ade80]" />
                <span>Live Trace</span>
              </div>
              <div className="flex items-center gap-2">
                <button className="px-2 py-1 text-xs border border-[#333] rounded hover:bg-[#333] transition-colors" onClick={() => setTraces([])}>Clear</button>
                <button className="p-1 hover:bg-[#333] rounded text-gray-400" onClick={() => setTraceVisible(false)}>
                  <X className="w-4 h-4" />
                </button>
              </div>
            </div>
            <ScrollArea className="flex-1 p-2">
              <div className="flex flex-col gap-1 pb-4">
                {traces.map((r, i) => {
                  const t = new Date(r.timestamp)
                  const timeStr = `${t.getHours().toString().padStart(2,'0')}:${t.getMinutes().toString().padStart(2,'0')}:${t.getSeconds().toString().padStart(2,'0')}.${t.getMilliseconds().toString().padStart(3,'0')}`
                  const isSlow = r.duration_ms && r.duration_ms > 1500
                  return (
                    <div key={i} className="hover:bg-[#2a2a2a] p-1.5 rounded transition-colors">
                      <div className="flex justify-between items-start gap-2">
                        <div className="text-[#888] shrink-0 w-[80px]">{timeStr}</div>
                        <div className="flex-1 text-[#4ade80] break-words" title={r.event}>{r.event}</div>
                        {r.duration_ms !== undefined && (
                          <div className={`w-[50px] text-right shrink-0 ${isSlow ? 'text-red-500 font-bold' : 'text-[#aaa]'}`}>{r.duration_ms}ms</div>
                        )}
                      </div>
                      {r.metadata && Object.keys(r.metadata).length > 0 && (
                        <div className="text-[#666] mt-1 pl-[88px] break-all text-[10px]">
                          {JSON.stringify(r.metadata)}
                        </div>
                      )}
                    </div>
                  )
                })}
                {traces.length === 0 && (
                  <div className="text-center text-[#666] mt-10">No trace events yet.<br/>Send a message to see performance data.</div>
                )}
              </div>
            </ScrollArea>
          </div>
        )}
      </div>

      {/* New Session Modal */}
      <Dialog open={isModalOpen} onOpenChange={setIsModalOpen}>
        <DialogContent className="sm:max-w-[425px]">
          <DialogHeader><DialogTitle>New Chat</DialogTitle></DialogHeader>
          <div className="space-y-5 py-4">
            <div className="space-y-2">
              <label className="text-sm font-medium">Chat Name</label>
              <Input value={newSessionName} onChange={e => setNewSessionName(e.target.value)} placeholder="Leave empty for timestamp" />
            </div>
            <div className="space-y-2">
              <label className="text-sm font-medium">Attach Skills (optional)</label>
              <div className="border rounded-md p-3 max-h-[160px] overflow-y-auto flex flex-col gap-3 bg-muted/30">
                {skills.map(s => (
                  <label key={s.name} className="flex items-center gap-3 cursor-pointer p-1 hover:bg-background rounded transition-colors">
                    <input type="checkbox" value={s.name} className="skill-cb w-4 h-4 rounded border-gray-300 text-primary focus:ring-primary" />
                    <span className="text-sm font-medium leading-none">{s.name}</span>
                  </label>
                ))}
              </div>
            </div>
          </div>
          <DialogFooter>
            <Button variant="ghost" onClick={() => setIsModalOpen(false)}>Cancel</Button>
            <Button onClick={() => {
              const cbs = document.querySelectorAll('.skill-cb:checked') as NodeListOf<HTMLInputElement>
              const selected = Array.from(cbs).map(cb => cb.value)
              const reqId = selected.length > 0 ? 'session-new-skills:' + selected.join(',') : 'session-new'
              ws?.send(JSON.stringify({ jsonrpc: '2.0', method: 'session.new', params: { agentId: activeAgent, name: newSessionName }, id: reqId }))
              setIsModalOpen(false)
              if (selected.length > 0) setInput(`Please use the \`${selected.join('`, `')}\` skills to \n\n`)
            }}>Create Chat</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </TooltipProvider>
  )
}