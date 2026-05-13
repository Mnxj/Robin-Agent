import { useState, useEffect } from 'react'
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogFooter } from "@/components/ui/dialog"
import { Trash2, Play, Pause, Plus } from 'lucide-react'

export default function Jobs() {
  const [jobs, setJobs] = useState<any[]>([])
  const [ws, setWs] = useState<WebSocket | null>(null)
  
  // Modal state
  const [isModalOpen, setIsModalOpen] = useState(false)
  const [jobName, setJobName] = useState('')
  const [jobSchedule, setJobSchedule] = useState('')
  const [jobPrompt, setJobPrompt] = useState('')

  useEffect(() => {
    const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:'
    const host = window.location.host === 'localhost:5173' ? '127.0.0.1:18789' : window.location.host
    const socket = new WebSocket(`${protocol}//${host}/ws`)
    
    socket.onopen = () => {
      socket.send(JSON.stringify({ jsonrpc: '2.0', method: 'jobs.list', params: {}, id: 'jobs-list' }))
    }
    
    socket.onmessage = (e) => {
      try {
        const resp = JSON.parse(e.data)
        if (resp.id === 'jobs-list') {
          setJobs(resp.result.jobs || [])
        } else if (['jobs-add', 'jobs-remove', 'jobs-pause', 'jobs-resume'].includes(resp.id)) {
          // Refresh list on any action
          socket.send(JSON.stringify({ jsonrpc: '2.0', method: 'jobs.list', params: {}, id: 'jobs-list' }))
        }
      } catch (err) {}
    }
    
    setWs(socket)
    return () => socket.close()
  }, [])

  const addJob = () => {
    if (!jobName || !jobSchedule || !jobPrompt) return
    ws?.send(JSON.stringify({
      jsonrpc: '2.0',
      method: 'jobs.add',
      params: { name: jobName, schedule: jobSchedule, prompt: jobPrompt },
      id: 'jobs-add'
    }))
    setIsModalOpen(false)
    setJobName(''); setJobSchedule(''); setJobPrompt('')
  }

  const toggleJob = (name: string, paused: boolean) => {
    ws?.send(JSON.stringify({
      jsonrpc: '2.0',
      method: paused ? 'jobs.resume' : 'jobs.pause',
      params: { name },
      id: paused ? 'jobs-resume' : 'jobs-pause'
    }))
  }

  const removeJob = (name: string) => {
    if (!confirm(`Delete cron job ${name}?`)) return
    ws?.send(JSON.stringify({
      jsonrpc: '2.0',
      method: 'jobs.remove',
      params: { name },
      id: 'jobs-remove'
    }))
  }

  return (
    <div className="space-y-4">
      <div className="flex justify-between items-center">
        <p className="text-sm text-muted-foreground">Manage scheduled Cron Jobs running in the background.</p>
        <Button onClick={() => setIsModalOpen(true)}><Plus className="w-4 h-4 mr-2" /> New Job</Button>
      </div>
      
      <div className="border rounded-md">
        <table className="w-full text-sm">
          <thead>
            <tr className="border-b bg-muted/50">
              <th className="text-left p-3 font-medium">Status</th>
              <th className="text-left p-3 font-medium">Name</th>
              <th className="text-left p-3 font-medium">Schedule (Cron)</th>
              <th className="text-left p-3 font-medium">Prompt</th>
              <th className="text-right p-3 font-medium">Actions</th>
            </tr>
          </thead>
          <tbody>
            {jobs.length === 0 ? (
              <tr><td colSpan={5} className="p-4 text-center text-muted-foreground">No cron jobs configured.</td></tr>
            ) : jobs.map(j => (
              <tr key={j.name} className={`border-b last:border-0 ${j.paused ? 'opacity-50 bg-muted/20' : ''}`}>
                <td className="p-3">
                  <div className={`w-2 h-2 rounded-full ${!j.paused ? 'bg-green-500' : 'bg-gray-400'}`}></div>
                </td>
                <td className="p-3 font-medium">{j.name}</td>
                <td className="p-3 font-mono text-xs">{j.schedule}</td>
                <td className="p-3 truncate max-w-[200px]" title={j.prompt}>{j.prompt}</td>
                <td className="p-3 text-right space-x-2">
                  <Button variant="ghost" size="sm" onClick={() => toggleJob(j.name, j.paused)}>
                    {!j.paused ? <Pause className="w-4 h-4" /> : <Play className="w-4 h-4" />}
                  </Button>
                  <Button variant="ghost" size="sm" className="text-red-500 hover:text-red-600" onClick={() => removeJob(j.name)}>
                    <Trash2 className="w-4 h-4" />
                  </Button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>

      <Dialog open={isModalOpen} onOpenChange={setIsModalOpen}>
        <DialogContent>
          <DialogHeader><DialogTitle>Create Cron Job</DialogTitle></DialogHeader>
          <div className="space-y-4 py-4">
            <div className="space-y-2">
              <label className="text-sm font-medium">Job Name</label>
              <Input value={jobName} onChange={e => setJobName(e.target.value)} placeholder="e.g. daily_summary" />
            </div>
            <div className="space-y-2">
              <label className="text-sm font-medium">Cron Schedule</label>
              <Input value={jobSchedule} onChange={e => setJobSchedule(e.target.value)} placeholder="e.g. 0 0 8 * * * *" />
              <p className="text-xs text-muted-foreground">Format: sec min hour day month dow year</p>
            </div>
            <div className="space-y-2">
              <label className="text-sm font-medium">Prompt</label>
              <Input value={jobPrompt} onChange={e => setJobPrompt(e.target.value)} placeholder="Task for the agent to execute" />
            </div>
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setIsModalOpen(false)}>Cancel</Button>
            <Button onClick={addJob}>Create Job</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  )
}
