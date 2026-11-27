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
    watch: {
      // Rust 构建产物在 cargo 重链接时被 Windows 文件锁占用，
      // chokidar 监听会触发 EBUSY 使 dev server 崩溃，故排除。
      ignored: ["**/src-tauri/target/**"],
    },
    hmr: {
      protocol: "ws",
      host: "127.0.0.1",
      port: devPort,
    },
  },
});
