import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

// FLIMMER_API overrides the backend the dev server proxies to (default: the app's own port).
const apiTarget = process.env.FLIMMER_API ?? 'http://127.0.0.1:43300'

export default defineConfig({
  plugins: [react()],
  server: {
    port: 5173,
    strictPort: true,
    proxy: {
      '/api': apiTarget,
    },
  },
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    target: 'baseline-widely-available',
    sourcemap: false,
    rolldownOptions: {
      output: {
        codeSplitting: {
          groups: [
            { name: 'vendor-react', test: /node_modules[\\/](react|react-dom|scheduler)[\\/]/ },
            { name: 'vendor-router', test: /node_modules[\\/]react-router/ },
            { name: 'vendor-query', test: /node_modules[\\/]@tanstack/ },
            { name: 'vendor-hls', test: /node_modules[\\/]hls\.js/ },
          ],
        },
      },
    },
  },
  optimizeDeps: {
    include: ['react', 'react-dom', 'react-router', '@tanstack/react-query'],
  },
})
