import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/card"
import { Activity, Server, Link as LinkIcon } from 'lucide-react'

export default function UI({ config }: { config: any }) {
  const agents = config?.agents || []
  
  return (
    <div className="space-y-6">
      <div className="flex justify-between items-center">
        <p className="text-sm text-muted-foreground">System control panel and running agent status.</p>
      </div>

      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2"><Server className="w-5 h-5" /> Registered Agents</CardTitle>
          <CardDescription>Agents configured and available in the current runtime.</CardDescription>
        </CardHeader>
        <CardContent>
          <div className="border rounded-md">
            <table className="w-full text-sm">
              <thead>
                <tr className="border-b bg-muted/50">
                  <th className="text-left p-3 font-medium">ID</th>
                  <th className="text-left p-3 font-medium">Name</th>
                  <th className="text-left p-3 font-medium">Model</th>
                  <th className="text-left p-3 font-medium">Workspace</th>
                  <th className="text-left p-3 font-medium">Sandbox</th>
                </tr>
              </thead>
              <tbody>
                {agents.length === 0 ? (
                  <tr><td colSpan={5} className="p-4 text-center text-muted-foreground">No agents configured.</td></tr>
                ) : agents.map((a: any) => (
                  <tr key={a.id} className="border-b last:border-0 hover:bg-muted/20">
                    <td className="p-3 font-medium">{a.id}</td>
                    <td className="p-3">{a.name}</td>
                    <td className="p-3 font-mono text-xs text-primary">{a.model}</td>
                    <td className="p-3 font-mono text-xs">{a.workspace || '-'}</td>
                    <td className="p-3">{a.sandbox ? 'Enabled' : 'Disabled'}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2"><LinkIcon className="w-5 h-5" /> Quick Links</CardTitle>
          <CardDescription>Direct links to internal gateway endpoints.</CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
            <a href="/health" target="_blank" className="flex items-center p-3 border rounded-md hover:bg-muted/50 transition-colors">
              <Activity className="w-4 h-4 mr-3 text-green-500" />
              <div>
                <div className="font-medium">Health Check</div>
                <div className="text-xs text-muted-foreground">/health</div>
              </div>
            </a>
            <a href="/metrics" target="_blank" className="flex items-center p-3 border rounded-md hover:bg-muted/50 transition-colors">
              <Activity className="w-4 h-4 mr-3 text-blue-500" />
              <div>
                <div className="font-medium">Prometheus Metrics</div>
                <div className="text-xs text-muted-foreground">/metrics</div>
              </div>
            </a>
          </div>
          <div className="p-3 bg-muted/20 rounded-md text-sm font-mono border">
            WebSocket endpoint: ws://127.0.0.1:18789/ws
          </div>
        </CardContent>
      </Card>
    </div>
  )
}
