import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import './index.css'
import App from './App.tsx'
import Settings from './Settings.tsx'

const path = window.location.pathname
const isSettings = path.startsWith('/settings') || path === '/logs' || path === '/jobs' || path === '/ui'

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    {isSettings ? <Settings /> : <App />}
  </StrictMode>,
)
