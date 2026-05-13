import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import './index.css'
import App from './App.tsx'
import Settings from './Settings.tsx'

const isSettings = window.location.pathname.startsWith('/settings')

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    {isSettings ? <Settings /> : <App />}
  </StrictMode>,
)
