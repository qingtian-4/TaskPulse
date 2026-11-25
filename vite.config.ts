import { sveltekit } from '@sveltejs/kit/vite';
import adapter from '@sveltejs/adapter-static';
import tailwindcss from '@tailwindcss/vite';
import { defineConfig, loadEnv } from 'vite';
import packageInfo from './package.json' with { type: 'json' };

export default defineConfig(({mode}) => ({
  plugins: [
    sveltekit({
      adapter: adapter({ fallback: 'index.html' }),
      paths: { relative: false },
      // All embedded pages share a stable startup identifier. Desktop updates
      // are delivered by Tauri, so there is no web deployment to poll for.
      version: { name: packageInfo.version, pollInterval: 0 },
    }),
    tailwindcss(),
  ],
  clearScreen: false,
  server: {
    host: '127.0.0.1',
    port: Number(loadEnv(mode, '.', 'TASKPULSE_DEV_PORT').TASKPULSE_DEV_PORT || 5173),
    strictPort: true,
    // Rust/installer/browser evidence is not frontend source. Watching it
    // caused reloads during native builds and isolated acceptance runs.
    watch: { ignored: ['**/src-tauri/target/**', '**/build/**', '**/dist/**'] },
  },
}));
