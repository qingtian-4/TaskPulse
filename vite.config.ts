import { sveltekit } from '@sveltejs/kit/vite';
import tailwindcss from '@tailwindcss/vite';
import { defineConfig } from 'vite';

export default defineConfig({
  plugins: [sveltekit(), tailwindcss()],
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    // Rust/installer/browser evidence is not frontend source. Watching it
    // caused reloads during native builds and isolated acceptance runs.
    watch: { ignored: ['**/src-tauri/target/**', '**/build/**', '**/dist/**'] },
  },
});
