import { useState, useEffect, useRef } from 'react'
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogFooter } from "@/components/ui/dialog"
import { Textarea } from "@/components/ui/textarea"
import { ScrollArea } from "@/components/ui/scroll-area"
import { Settings, Send, SquareSquare } from 'lucide-react'
import DOMPurify from 'dompurify'
import { marked } from 'marked'

// Helper for rendering markdown
const renderMd = (text: string) => {
  return { __html: DOMPurify.sanitize(marked.parse(text) as string) }
}

export default function App() {
  const [agents, setAgents] = useState<{id: string, name: string}[]>([])
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
  const scrollRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    // Determine WS URL
    const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:'
    const host = window.location.host === 'localhost:5173' ? '127.0.0.1:18789' : window.location.host
    const socket = new WebSocket(`${protocol}//${host}/ws`)
    
    socket.onopen = () => {
      console.log('WS Connected')
      socket.send(JSON.stringify({ jsonrpc: '2.0', method: 'agent.status', params: {}, id: 'agents' }))
    }
    
    socket.onmessage = (e) => {
      try {
        const resp = JSON.parse(e.data)
        
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
            }
            return { id: Math.random(), role: 'tool', ...en }
          }).filter((en: any) => en.role === 'user' || en.role === 'assistant')
          setMessages(loadedMsgs)
        } else if (resp.result?.type === 'text_delta') {
          setMessages(prev => {
            const last = prev[prev.length - 1]
            if (last && last.role === 'assistant' && !last.final) {
              return [...prev.slice(0, -1), { ...last, text: last.text + resp.result.text }]
            }
            return [...prev, { id: Math.random(), role: 'assistant', text: resp.result.text, final: false }]
          })
        } else if (resp.result?.type === 'done') {
          setSending(false)
          setMessages(prev => {
            const last = prev[prev.length - 1]
            if (last && last.role === 'assistant') return [...prev.slice(0, -1), { ...last, final: true }]
            return prev
          })
        } else if (resp.id && resp.id.toString().startsWith('session-new')) {
           setActiveSession(resp.result.sessionKey)
           socket.send(JSON.stringify({ jsonrpc: '2.0', method: 'session.list', params: { agentId: activeAgent }, id: 'sessions' }))
        }
      } catch (err) {
        console.error(err)
      }
    }
    
    setWs(socket)
    
    // Fetch skills
    const apiHost = window.location.host === 'localhost:5173' ? 'http://127.0.0.1:18789' : ''
    fetch(`${apiHost}/settings/api/skills`).then(r=>r.json()).then(d => setSkills(d.skills || []))
    
    return () => socket.close()
  }, [])

  useEffect(() => {
    if (scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight
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

  return (
    <div className="flex flex-col h-screen bg-background text-foreground font-sans">
      {/* Header */}
      <header className="flex items-center gap-4 p-4 border-b">
        <h1 className="text-xl font-bold tracking-tight">robin</h1>
        <Select value={activeAgent} onValueChange={setActiveAgent}>
          <SelectTrigger className="w-[180px]"><SelectValue placeholder="Agent" /></SelectTrigger>
          <SelectContent>
            {agents.map(a => <SelectItem key={a.id} value={a.id}>{a.name}</SelectItem>)}
          </SelectContent>
        </Select>
        <Select value={activeSession} onValueChange={(val) => {
          setActiveSession(val)
          if(ws) ws.send(JSON.stringify({ jsonrpc: '2.0', method: 'session.history', params: { agentId: activeAgent, sessionKey: val }, id: 'history' }))
        }}>
          <SelectTrigger className="w-[200px]"><SelectValue placeholder="Session" /></SelectTrigger>
          <SelectContent>
            {sessions.map(s => <SelectItem key={s.key} value={s.key}>{s.key} ({s.entryCount})</SelectItem>)}
          </SelectContent>
        </Select>
        <Button variant="outline" onClick={() => setIsModalOpen(true)}>+ New</Button>
        <div className="flex-1"></div>
        <Button variant="ghost" size="icon" title="Settings" onClick={() => window.open('/settings', '_blank')}><Settings className="w-4 h-4" /></Button>
      </header>

      {/* Messages */}
      <ScrollArea className="flex-1 p-4" ref={scrollRef}>
        <div className="max-w-4xl mx-auto space-y-6">
          {messages.map((m, i) => (
            <div key={i} className={`flex ${m.role === 'user' ? 'justify-end' : 'justify-start'}`}>
              <div className={`max-w-[80%] rounded-lg p-4 ${m.role === 'user' ? 'bg-primary text-primary-foreground' : 'bg-muted'}`}>
                {m.role === 'user' ? (
                  <>
                    <div>{m.text}</div>
                    {m.images && m.images.length > 0 && (
                      <div className="flex gap-2 mt-2 flex-wrap">
                        {m.images.map((img: any, idx: number) => (
                          <img key={idx} src={`data:${img.mimeType};base64,${img.data}`} className="w-48 rounded border" />
                        ))}
                      </div>
                    )}
                  </>
                ) : (
                  <div className="markdown-body" dangerouslySetInnerHTML={renderMd(m.text)} />
                )}
              </div>
            </div>
          ))}
        </div>
      </ScrollArea>

      {/* Input Area */}
      <div className="p-4 border-t bg-background">
        <div className="max-w-4xl mx-auto flex flex-col gap-2">
          {images.length > 0 && (
            <div className="flex gap-2 mb-2 overflow-x-auto">
              {images.map((img, i) => (
                <div key={i} className="relative">
                  <img src={img.data} className="h-16 rounded border" />
                  <button onClick={() => setImages(images.filter((_, idx) => idx !== i))} className="absolute -top-2 -right-2 bg-destructive text-white rounded-full w-5 h-5 flex items-center justify-center text-xs">×</button>
                </div>
              ))}
            </div>
          )}
          <div className="flex gap-2">
            <Textarea 
              value={input}
              onChange={e => setInput(e.target.value)}
              onKeyDown={e => { if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); sendMsg() } }}
              onPaste={handlePaste}
              placeholder="Type a message... (paste images here)"
              className="min-h-[60px] resize-none"
            />
            <div className="flex flex-col justify-end gap-2">
              {sending ? (
                <Button variant="destructive" onClick={() => {
                  ws?.send(JSON.stringify({ jsonrpc: '2.0', method: 'chat.abort', params: {}, id: 'abort' }))
                  setSending(false)
                }}><SquareSquare className="w-4 h-4 mr-2"/> Stop</Button>
              ) : (
                <Button onClick={sendMsg}><Send className="w-4 h-4 mr-2"/> Send</Button>
              )}
            </div>
          </div>
        </div>
      </div>

      {/* New Session Modal */}
      <Dialog open={isModalOpen} onOpenChange={setIsModalOpen}>
        <DialogContent>
          <DialogHeader><DialogTitle>New Session</DialogTitle></DialogHeader>
          <div className="space-y-4">
            <div>
              <label className="text-sm font-medium">Session Name</label>
              <Input value={newSessionName} onChange={e => setNewSessionName(e.target.value)} placeholder="Leave empty for timestamp" />
            </div>
            <div>
              <label className="text-sm font-medium">Attach Skills</label>
              <div className="border rounded-md p-2 max-h-[120px] overflow-y-auto flex flex-col gap-2">
                {skills.map(s => (
                  <label key={s.name} className="flex items-center gap-2 cursor-pointer">
                    <input type="checkbox" value={s.name} className="skill-cb" />
                    <span className="text-sm">{s.name}</span>
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
