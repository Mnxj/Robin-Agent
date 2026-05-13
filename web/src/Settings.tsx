import { useState, useEffect } from 'react'
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Textarea } from "@/components/ui/textarea"
import { ScrollArea } from "@/components/ui/scroll-area"
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogFooter } from "@/components/ui/dialog"
import { Trash2, Upload, FileText, Settings as SettingsIcon, BrainCircuit, Wrench } from 'lucide-react'

import ConfigEditor from './ConfigEditor'

export default function Settings() {
  const [activeTab, setActiveTab] = useState('config')
  const [configObj, setConfigObj] = useState<any>({})
  const [skills, setSkills] = useState<any[]>([])
  const [memories, setMemories] = useState<any[]>([])
  const [tools, setTools] = useState<any[]>([])
  const [status, setStatus] = useState({ msg: '', isError: false })
  
  // Memory Editor State
  const [memModalOpen, setMemModalOpen] = useState(false)
  const [memId, setMemId] = useState('')
  const [memContent, setMemContent] = useState('')
  const [isMemEditing, setIsMemEditing] = useState(false)

  // Skill Viewer State
  const [skillModalOpen, setSkillModalOpen] = useState(false)
  const [skillContent, setSkillContent] = useState('')
  const [skillName, setSkillName] = useState('')

  const showStatus = (msg: string, isError = false) => {
    setStatus({ msg, isError })
    setTimeout(() => setStatus({ msg: '', isError: false }), 3000)
  }

  const loadConfig = async () => {
    try {
      const res = await fetch('/settings/api/config')
      const data = await res.json()
      setConfigObj(data)
    } catch (e: any) {
      showStatus('Failed to load config: ' + e.message, true)
    }
  }

  const saveConfig = async (newCfg: any) => {
    try {
      const res = await fetch('/settings/api/config', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(newCfg)
      })
      if (!res.ok) throw new Error('Failed to save')
      showStatus('Configuration saved successfully')
      setConfigObj(newCfg)
    } catch (e: any) {
      showStatus('Invalid JSON or save failed: ' + e.message, true)
    }
  }

  const loadSkills = async () => {
    try {
      const res = await fetch('/settings/api/skills')
      const data = await res.json()
      setSkills(data.skills || [])
    } catch (e: any) {
      showStatus('Failed to load skills: ' + e.message, true)
    }
  }

  const uploadSkill = async (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0]
    if (!file) return
    const fd = new FormData()
    fd.append('file', file)
    try {
      const res = await fetch('/settings/api/skills', { method: 'POST', body: fd })
      if (!res.ok) throw new Error('Upload failed')
      showStatus('Skill uploaded')
      loadSkills()
    } catch (err: any) {
      showStatus(err.message, true)
    }
  }

  const deleteSkill = async (name: string) => {
    if (!confirm(`Delete skill ${name}?`)) return
    try {
      await fetch(`/settings/api/skills/${encodeURIComponent(name)}`, { method: 'DELETE' })
      showStatus('Skill deleted')
      loadSkills()
    } catch (err: any) {
      showStatus(err.message, true)
    }
  }

  const viewSkill = async (name: string) => {
    try {
      const res = await fetch(`/settings/api/skills/${encodeURIComponent(name)}`)
      const text = await res.text()
      setSkillName(name)
      setSkillContent(text)
      setSkillModalOpen(true)
    } catch (err: any) {
      showStatus(err.message, true)
    }
  }

  const loadMemories = async () => {
    try {
      const res = await fetch('/settings/api/memory')
      if (!res.ok) throw new Error('Memory disabled or failed')
      const data = await res.json()
      setMemories(data.entries || [])
    } catch (e: any) {
      setMemories([])
    }
  }

  const saveMemory = async () => {
    if (!memId.trim()) return showStatus('ID is required', true)
    try {
      const res = await fetch('/settings/api/memory', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ id: memId, content: memContent })
      })
      if (!res.ok) throw new Error('Save failed')
      showStatus('Memory saved')
      setMemModalOpen(false)
      loadMemories()
    } catch (err: any) {
      showStatus(err.message, true)
    }
  }

  const deleteMemory = async (id: string) => {
    if (!confirm(`Delete memory ${id}?`)) return
    try {
      await fetch(`/settings/api/memory/${encodeURIComponent(id)}`, { method: 'DELETE' })
      showStatus('Memory deleted')
      loadMemories()
    } catch (err: any) {
      showStatus(err.message, true)
    }
  }

  const editMemory = async (id: string) => {
    try {
      const res = await fetch(`/settings/api/memory/${encodeURIComponent(id)}`)
      const text = await res.text()
      setMemId(id)
      setMemContent(text)
      setIsMemEditing(true)
      setMemModalOpen(true)
    } catch (err: any) {
      showStatus(err.message, true)
    }
  }

  const loadTools = async () => {
    try {
      const res = await fetch('/settings/api/tools')
      const data = await res.json()
      setTools(data.tools || [])
    } catch (e: any) {
      showStatus('Failed to load tools: ' + e.message, true)
    }
  }

  useEffect(() => {
    if (activeTab === 'config') loadConfig()
    if (activeTab === 'skills') loadSkills()
    if (activeTab === 'memory') loadMemories()
    if (activeTab === 'tools') loadTools()
  }, [activeTab])

  const tabs = [
    { id: 'config', name: 'Configuration', icon: <SettingsIcon className="w-4 h-4" /> },
    { id: 'skills', name: 'Skills', icon: <FileText className="w-4 h-4" /> },
    { id: 'memory', name: 'Memory', icon: <BrainCircuit className="w-4 h-4" /> },
    { id: 'tools', name: 'Tools', icon: <Wrench className="w-4 h-4" /> },
  ]

  return (
    <div className="flex h-screen bg-background text-foreground font-sans overflow-hidden">
      {/* Sidebar */}
      <div className="w-[240px] border-r flex flex-col bg-muted/20 shrink-0">
        <div className="p-4 border-b">
          <h2 className="text-xl font-bold flex items-center gap-2">
            <SettingsIcon className="w-5 h-5" /> Settings
          </h2>
        </div>
        <div className="flex-1 p-3 flex flex-col gap-1">
          {tabs.map(t => (
            <Button 
              key={t.id} 
              variant={activeTab === t.id ? "secondary" : "ghost"} 
              className={`w-full justify-start h-10 ${activeTab === t.id ? 'bg-muted' : ''}`}
              onClick={() => setActiveTab(t.id)}
            >
              {t.icon} <span className="ml-2">{t.name}</span>
            </Button>
          ))}
        </div>
        <div className="p-3 border-t">
          <Button variant="ghost" className="w-full justify-start" asChild>
            <a href="/chat">Back to Chat</a>
          </Button>
        </div>
      </div>

      {/* Main Content */}
      <div className="flex-1 flex flex-col min-w-0 relative">
        <header className="h-14 border-b flex items-center px-6 justify-between shrink-0 bg-background">
          <h3 className="font-semibold text-lg">{tabs.find(t => t.id === activeTab)?.name}</h3>
          {status.msg && (
            <div className={`text-sm px-3 py-1 rounded-full ${status.isError ? 'bg-red-500/10 text-red-500' : 'bg-green-500/10 text-green-500'}`}>
              {status.msg}
            </div>
          )}
        </header>
        
        <ScrollArea className="flex-1 p-6">
          <div className="max-w-4xl mx-auto space-y-6">
            
            {activeTab === 'config' && (
              <ConfigEditor initialConfig={configObj} onSave={saveConfig} />
            )}

            {activeTab === 'skills' && (
              <div className="space-y-4">
                <div className="flex justify-between items-center">
                  <div className="text-sm text-muted-foreground">Skills are markdown files loaded on the next chat turn.</div>
                  <div className="relative">
                    <Input type="file" accept=".md" className="absolute inset-0 opacity-0 cursor-pointer" onChange={uploadSkill} />
                    <Button><Upload className="w-4 h-4 mr-2" /> Upload .md</Button>
                  </div>
                </div>
                <div className="border rounded-md">
                  <table className="w-full text-sm">
                    <thead>
                      <tr className="border-b bg-muted/50">
                        <th className="text-left p-3 font-medium">Filename</th>
                        <th className="text-left p-3 font-medium">Description</th>
                        <th className="text-right p-3 font-medium">Size</th>
                        <th className="text-right p-3 font-medium">Actions</th>
                      </tr>
                    </thead>
                    <tbody>
                      {skills.length === 0 ? (
                        <tr><td colSpan={4} className="p-4 text-center text-muted-foreground">No skills found.</td></tr>
                      ) : skills.map(s => (
                        <tr key={s.filename} className="border-b last:border-0 hover:bg-muted/20">
                          <td className="p-3 font-mono text-primary">{s.filename}</td>
                          <td className="p-3">{s.description}</td>
                          <td className="p-3 text-right">{Math.round(s.size_bytes / 1024)} KB</td>
                          <td className="p-3 text-right space-x-2">
                            <Button variant="ghost" size="sm" onClick={() => viewSkill(s.filename)}>View</Button>
                            <Button variant="ghost" size="sm" className="text-red-500 hover:text-red-600" onClick={() => deleteSkill(s.filename)}><Trash2 className="w-4 h-4"/></Button>
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              </div>
            )}

            {activeTab === 'memory' && (
              <div className="space-y-4">
                <div className="flex justify-between items-center">
                  <div className="text-sm text-muted-foreground">Stored as Markdown in <code>~/.robin/memory/entries/</code>.</div>
                  <Button onClick={() => { setMemId(''); setMemContent(''); setIsMemEditing(false); setMemModalOpen(true); }}>
                    + New Entry
                  </Button>
                </div>
                <div className="border rounded-md">
                  <table className="w-full text-sm">
                    <thead>
                      <tr className="border-b bg-muted/50">
                        <th className="text-left p-3 font-medium">ID</th>
                        <th className="text-left p-3 font-medium">Title</th>
                        <th className="text-right p-3 font-medium">Size</th>
                        <th className="text-right p-3 font-medium">Actions</th>
                      </tr>
                    </thead>
                    <tbody>
                      {memories.length === 0 ? (
                        <tr><td colSpan={4} className="p-4 text-center text-muted-foreground">No memory entries found.</td></tr>
                      ) : memories.map(m => (
                        <tr key={m.id} className="border-b last:border-0 hover:bg-muted/20">
                          <td className="p-3 font-mono text-primary">{m.id}</td>
                          <td className="p-3">{m.title}</td>
                          <td className="p-3 text-right">{Math.round(m.bytes / 1024)} KB</td>
                          <td className="p-3 text-right space-x-2">
                            <Button variant="ghost" size="sm" onClick={() => editMemory(m.id)}>Edit</Button>
                            <Button variant="ghost" size="sm" className="text-red-500 hover:text-red-600" onClick={() => deleteMemory(m.id)}><Trash2 className="w-4 h-4"/></Button>
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              </div>
            )}

            {activeTab === 'tools' && (
              <div className="space-y-4">
                <div className="text-sm text-muted-foreground">Tools currently registered in the global tool registry.</div>
                <div className="grid gap-3">
                  {tools.map(t => (
                    <div key={t.name} className="border rounded-md p-4 bg-muted/10">
                      <div className="font-mono font-semibold text-primary mb-1">{t.name}</div>
                      <div className="text-sm text-muted-foreground whitespace-pre-wrap">{t.description}</div>
                    </div>
                  ))}
                  {tools.length === 0 && <div className="text-muted-foreground">No tools registered.</div>}
                </div>
              </div>
            )}

          </div>
        </ScrollArea>
      </div>

      {/* Skill View Modal */}
      <Dialog open={skillModalOpen} onOpenChange={setSkillModalOpen}>
        <DialogContent className="max-w-3xl max-h-[80vh] flex flex-col">
          <DialogHeader><DialogTitle>{skillName}</DialogTitle></DialogHeader>
          <ScrollArea className="flex-1 border rounded-md p-4 bg-muted/30 font-mono text-sm whitespace-pre-wrap">
            {skillContent}
          </ScrollArea>
        </DialogContent>
      </Dialog>

      {/* Memory Edit Modal */}
      <Dialog open={memModalOpen} onOpenChange={setMemModalOpen}>
        <DialogContent className="max-w-3xl max-h-[90vh] flex flex-col">
          <DialogHeader><DialogTitle>{isMemEditing ? 'Edit Memory' : 'New Memory Entry'}</DialogTitle></DialogHeader>
          <div className="flex flex-col gap-4 flex-1 overflow-hidden">
            <Input 
              value={memId} 
              onChange={e => setMemId(e.target.value)} 
              placeholder="Entry ID (e.g. coding_style)" 
              disabled={isMemEditing}
              className="font-mono"
            />
            <Textarea 
              value={memContent} 
              onChange={e => setMemContent(e.target.value)} 
              placeholder="# Title\n\nContent..." 
              className="flex-1 font-mono resize-none min-h-[300px]"
            />
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setMemModalOpen(false)}>Cancel</Button>
            <Button onClick={saveMemory}>Save Memory</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  )
}
