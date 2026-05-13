import { useState, useEffect } from 'react'
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Textarea } from "@/components/ui/textarea"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"
import { Switch } from "@/components/ui/switch"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/card"
import { Trash2, Plus, Save } from 'lucide-react'

export default function ConfigEditor({ initialConfig, onSave }: { initialConfig: any, onSave: (cfg: any) => void }) {
  const [cfg, setCfg] = useState<any>(initialConfig || {})

  useEffect(() => {
    setCfg(initialConfig || {})
  }, [initialConfig])

  const update = (path: (string | number)[], value: any) => {
    setCfg((prev: any) => {
      const next = JSON.parse(JSON.stringify(prev))
      let curr = next
      for (let i = 0; i < path.length - 1; i++) {
        if (curr[path[i]] === undefined) curr[path[i]] = typeof path[i+1] === 'number' ? [] : {}
        curr = curr[path[i]]
      }
      curr[path[path.length - 1]] = value
      return next
    })
  }

  const get = (path: (string | number)[], fallback: any = '') => {
    let curr = cfg
    for (let i = 0; i < path.length; i++) {
      if (curr === undefined || curr === null) return fallback
      curr = curr[path[i]]
    }
    return curr === undefined ? fallback : curr
  }

  const removeArrayItem = (path: (string | number)[], index: number) => {
    setCfg((prev: any) => {
      const next = JSON.parse(JSON.stringify(prev))
      let curr = next
      for (let i = 0; i < path.length; i++) curr = curr[path[i]]
      if (Array.isArray(curr)) curr.splice(index, 1)
      return next
    })
  }

  const addArrayItem = (path: (string | number)[], item: any) => {
    setCfg((prev: any) => {
      const next = JSON.parse(JSON.stringify(prev))
      let curr = next
      for (let i = 0; i < path.length - 1; i++) {
        if (curr[path[i]] === undefined) curr[path[i]] = {}
        curr = curr[path[i]]
      }
      if (!Array.isArray(curr[path[path.length - 1]])) curr[path[path.length - 1]] = []
      curr[path[path.length - 1]].push(item)
      return next
    })
  }

  return (
    <div className="space-y-4">
      <div className="flex justify-between items-center">
        <p className="text-sm text-muted-foreground">Modify Robin configuration via form inputs. Some changes require a restart.</p>
        <Button onClick={() => onSave(cfg)}><Save className="w-4 h-4 mr-2" /> Save Configuration</Button>
      </div>

      <Tabs defaultValue="gateway" className="w-full">
        <TabsList className="flex flex-wrap h-auto">
          <TabsTrigger value="gateway">Gateway</TabsTrigger>
          <TabsTrigger value="agents">Agents</TabsTrigger>
          <TabsTrigger value="providers">Providers</TabsTrigger>
          <TabsTrigger value="security">Security</TabsTrigger>
          <TabsTrigger value="memory">Memory</TabsTrigger>
          <TabsTrigger value="mcp">MCP</TabsTrigger>
        </TabsList>

        <TabsContent value="gateway" className="space-y-4 mt-4">
          <Card>
            <CardHeader><CardTitle>Gateway Server</CardTitle></CardHeader>
            <CardContent className="space-y-4">
              <div className="grid gap-2">
                <label className="text-sm font-medium">Host</label>
                <Input value={get(['gateway', 'host'], '127.0.0.1')} onChange={e => update(['gateway', 'host'], e.target.value)} />
              </div>
              <div className="grid gap-2">
                <label className="text-sm font-medium">Port</label>
                <Input type="number" value={get(['gateway', 'port'], 18789)} onChange={e => update(['gateway', 'port'], parseInt(e.target.value))} />
              </div>
              <div className="grid gap-2">
                <label className="text-sm font-medium">Auth Token</label>
                <Input type="password" placeholder="Optional bearer token" value={get(['gateway', 'auth', 'token'], '')} onChange={e => update(['gateway', 'auth', 'token'], e.target.value)} />
              </div>
            </CardContent>
          </Card>
        </TabsContent>

        <TabsContent value="agents" className="space-y-4 mt-4">
          <div className="flex justify-between items-center">
            <h3 className="font-semibold">Registered Agents</h3>
            <Button variant="outline" size="sm" onClick={() => addArrayItem(['agents', 'list'], { id: 'new_agent', name: 'New Agent', model: '', system_prompt: '' })}>
              <Plus className="w-4 h-4 mr-1"/> Add Agent
            </Button>
          </div>
          {get(['agents', 'list'], []).map((agent: any, i: number) => (
            <Card key={i}>
              <CardContent className="space-y-4 pt-6">
                <div className="flex justify-end"><Button variant="ghost" size="sm" className="text-red-500" onClick={() => removeArrayItem(['agents', 'list'], i)}><Trash2 className="w-4 h-4"/></Button></div>
                <div className="grid grid-cols-2 gap-4">
                  <div className="grid gap-2"><label className="text-sm font-medium">ID</label><Input value={agent.id || ''} onChange={e => update(['agents', 'list', i, 'id'], e.target.value)} /></div>
                  <div className="grid gap-2"><label className="text-sm font-medium">Name</label><Input value={agent.name || ''} onChange={e => update(['agents', 'list', i, 'name'], e.target.value)} /></div>
                  <div className="grid gap-2"><label className="text-sm font-medium">Model</label><Input value={agent.model || ''} onChange={e => update(['agents', 'list', i, 'model'], e.target.value)} /></div>
                  <div className="grid gap-2"><label className="text-sm font-medium">Context Window</label><Input type="number" value={agent.context_window || ''} onChange={e => update(['agents', 'list', i, 'context_window'], parseInt(e.target.value))} /></div>
                </div>
                <div className="grid gap-2">
                  <label className="text-sm font-medium">System Prompt</label>
                  <Textarea className="h-32" value={agent.system_prompt || ''} onChange={e => update(['agents', 'list', i, 'system_prompt'], e.target.value)} />
                </div>
              </CardContent>
            </Card>
          ))}
        </TabsContent>

        <TabsContent value="providers" className="space-y-4 mt-4">
          <Card>
            <CardHeader><CardTitle>LLM Providers</CardTitle><CardDescription>Edit providers in the raw JSON below (UI simplifies this for now)</CardDescription></CardHeader>
            <CardContent>
              <Textarea 
                className="h-64 font-mono" 
                value={JSON.stringify(get(['providers'], {}), null, 2)}
                onChange={e => {
                  try { update(['providers'], JSON.parse(e.target.value)) } catch(err) {}
                }}
              />
            </CardContent>
          </Card>
        </TabsContent>

        <TabsContent value="security" className="space-y-4 mt-4">
          <Card>
            <CardHeader><CardTitle>Security & Execution</CardTitle></CardHeader>
            <CardContent className="space-y-4">
              <div className="grid gap-2">
                <label className="text-sm font-medium">Exec Approvals Level</label>
                <Select value={get(['security', 'execApprovals', 'level'], 'full')} onValueChange={v => update(['security', 'execApprovals', 'level'], v)}>
                  <SelectTrigger><SelectValue /></SelectTrigger>
                  <SelectContent>
                    <SelectItem value="full">Full (Auto-execute all)</SelectItem>
                    <SelectItem value="allowlist">Allowlist (Only allowed commands)</SelectItem>
                    <SelectItem value="deny">Deny (Manual approval for all)</SelectItem>
                  </SelectContent>
                </Select>
              </div>
              <div className="grid gap-2">
                <label className="text-sm font-medium">Allowlist (comma separated)</label>
                <Input value={get(['security', 'execApprovals', 'allowlist'], []).join(', ')} onChange={e => update(['security', 'execApprovals', 'allowlist'], e.target.value.split(',').map((s: string) => s.trim()))} />
              </div>
            </CardContent>
          </Card>
        </TabsContent>

        <TabsContent value="memory" className="space-y-4 mt-4">
          <Card>
            <CardHeader><CardTitle>Long-term Memory</CardTitle></CardHeader>
            <CardContent className="space-y-4">
              <div className="flex items-center justify-between">
                <label className="text-sm font-medium">Enable Memory</label>
                <Switch checked={get(['memory', 'enabled'], true)} onCheckedChange={v => update(['memory', 'enabled'], v)} />
              </div>
              <div className="grid gap-2">
                <label className="text-sm font-medium">Embedding Provider</label>
                <Input value={get(['memory', 'embeddingProvider'], '')} onChange={e => update(['memory', 'embeddingProvider'], e.target.value)} />
              </div>
              <div className="grid gap-2">
                <label className="text-sm font-medium">Embedding Model</label>
                <Input value={get(['memory', 'embeddingModel'], 'nomic-embed-text')} onChange={e => update(['memory', 'embeddingModel'], e.target.value)} />
              </div>
            </CardContent>
          </Card>
        </TabsContent>

        <TabsContent value="mcp" className="space-y-4 mt-4">
          <Card>
            <CardHeader><CardTitle>MCP Servers</CardTitle><CardDescription>Edit MCP server configurations as JSON array.</CardDescription></CardHeader>
            <CardContent>
              <Textarea 
                className="h-64 font-mono" 
                value={JSON.stringify(get(['mcp_servers'], []), null, 2)}
                onChange={e => {
                  try { update(['mcp_servers'], JSON.parse(e.target.value)) } catch(err) {}
                }}
              />
            </CardContent>
          </Card>
        </TabsContent>
      </Tabs>
    </div>
  )
}
