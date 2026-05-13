import { useState, useEffect, useRef } from 'react'
import { ScrollArea } from "@/components/ui/scroll-area"
import { Play, Pause, Trash2 } from 'lucide-react'
import { Button } from "@/components/ui/button"

export default function Logs() {
  const [logs, setLogs] = useState<string[]>([])
  const [isPaused, setIsPaused] = useState(false)
  const scrollRef = useRef<HTMLDivElement>(null)
  const eventSourceRef = useRef<EventSource | null>(null)

  useEffect(() => {
    const connect = () => {
      const es = new EventSource('/logs/stream')
      es.onmessage = (e) => {
        setLogs(prev => {
          const next = [...prev, e.data]
          if (next.length > 2000) return next.slice(next.length - 2000)
          return next
        })
      }
      eventSourceRef.current = es
    }

    if (!isPaused) {
      connect()
    } else if (eventSourceRef.current) {
      eventSourceRef.current.close()
      eventSourceRef.current = null
    }

    return () => {
      if (eventSourceRef.current) {
        eventSourceRef.current.close()
      }
    }
  }, [isPaused])

  useEffect(() => {
    if (!isPaused && scrollRef.current) {
      const el = scrollRef.current.querySelector('[data-radix-scroll-area-viewport]')
      if (el) el.scrollTop = el.scrollHeight
    }
  }, [logs, isPaused])

  return (
    <div className="flex flex-col h-full space-y-4">
      <div className="flex justify-between items-center">
        <p className="text-sm text-muted-foreground">Real-time system logs from the Robin gateway.</p>
        <div className="flex gap-2">
          <Button variant="outline" size="sm" onClick={() => setIsPaused(!isPaused)}>
            {isPaused ? <Play className="w-4 h-4 mr-2" /> : <Pause className="w-4 h-4 mr-2" />}
            {isPaused ? 'Resume' : 'Pause'}
          </Button>
          <Button variant="outline" size="sm" onClick={() => setLogs([])}>
            <Trash2 className="w-4 h-4 mr-2" /> Clear
          </Button>
        </div>
      </div>
      <ScrollArea className="flex-1 border rounded-md bg-[#1e1e1e] p-4 font-mono text-[13px] text-[#d4d4d4]" ref={scrollRef}>
        {logs.map((log, i) => {
          let color = '#d4d4d4'
          if (log.includes(' ERROR ')) color = '#ef4444'
          else if (log.includes(' WARN ')) color = '#f59e0b'
          else if (log.includes(' INFO ')) color = '#3b82f6'
          else if (log.includes(' DEBUG ')) color = '#10b981'
          
          return (
            <div key={i} style={{ color }} className="whitespace-pre-wrap break-all mb-1">
              {log}
            </div>
          )
        })}
        {logs.length === 0 && <div className="text-muted-foreground italic">Waiting for logs...</div>}
      </ScrollArea>
    </div>
  )
}
