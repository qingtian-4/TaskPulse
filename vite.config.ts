import { sveltekit } from '@sveltejs/kit/vite';
import tailwindcss from '@tailwindcss/vite';
import { defineConfig, loadEnv } from 'vite';

export default defineConfig(({mode}) => ({
  plugins: [sveltekit(), tailwindcss()],
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
