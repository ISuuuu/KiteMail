import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";

function resolveDevPort(): number {
  const raw = process.env.KITEMAIL_DEV_PORT ?? process.env.TAURI_DEV_PORT ?? "1420";
  const parsed = Number(raw);
  if (Number.isInteger(parsed) && parsed > 0 && parsed < 65536) {
    return parsed;
  }
  return 1420;
}

const devPort = resolveDevPort();

export default defineConfig({
  plugins: [vue()],
  clearScreen: false,
  build: {
    outDir: "dist-web",
  },
  server: {
    host: "127.0.0.1",
    port: devPort,
    strictPort: true,
    hmr: {
      protocol: "ws",
      host: "127.0.0.1",
      port: devPort,
    },
  },
});
