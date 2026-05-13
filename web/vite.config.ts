import path from "path"
import react from "@vitejs/plugin-react"
import { defineConfig } from "vite"
import { viteSingleFile } from "vite-plugin-singlefile"

export default defineConfig({
  plugins: [react(), viteSingleFile()],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
  server: {
    proxy: {
      '/settings/api': {
        target: 'http://127.0.0.1:18789',
        changeOrigin: true
      },
      '/ws': {
        target: 'ws://127.0.0.1:18789',
        ws: true
      }
    }
  }
})
