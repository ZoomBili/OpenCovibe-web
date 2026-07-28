import { sveltekit } from "@sveltejs/kit/vite";
import { defineConfig } from "vite";

export default defineConfig({
  plugins: [sveltekit()],
  build: {
    target: ["safari16", "chrome105", "edge105"],
  },
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: false,
    fs: {
      allow: [".", "messages"],
    },
    watch: {
      // Ignore non-frontend paths to prevent CLI agent file operations
      // from triggering page reloads during active sessions.
      ignored: [
        "**/src-tauri/**",
        "**/node_modules/**",
        "**/.git/**",
        "**/build/**",
        "**/target/**",
        "**/apps/**",
        "**/packages/**",
        "**/.next/**",
        "**/dist/**",
        "**/.claude/**",
        "**/.opencovibe/**",
        "**/tmp/**",
        "**/memory/**",
      ],
    },
  },
});
