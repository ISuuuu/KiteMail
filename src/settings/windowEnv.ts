/**
 * 设置窗运行环境检测。
 *
 * 独立设置窗由 Rust 侧以 `index.html?window=settings` 创建，URL 参数为首选判定；
 * `window.__TAURI_INTERNALS__` 存在时兜底读取 Tauri 窗口 label（防 URL 被改写）。
 */

export const SETTINGS_WINDOW_URL_PARAM = "window";
export const SETTINGS_WINDOW_URL_VALUE = "settings";
export const SETTINGS_WINDOW_LABEL = "settings";

/** 是否存在 Tauri IPC 桥（浏览器 `pnpm dev` 预览时为 false）。 */
export function isTauriRuntime(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

function tauriWindowLabel(): string | null {
  if (!isTauriRuntime()) return null;
  try {
    const internals = (
      window as unknown as {
        __TAURI_INTERNALS__?: { metadata?: { currentWindow?: { label?: string } } };
      }
    ).__TAURI_INTERNALS__;
    return internals?.metadata?.currentWindow?.label ?? null;
  } catch {
    return null;
  }
}

/** 当前 WebView 是否承载独立设置窗内容。 */
export function isSettingsWindow(): boolean {
  if (typeof window === "undefined") return false;
  try {
    const query = new URLSearchParams(window.location.search);
    if (query.get(SETTINGS_WINDOW_URL_PARAM) === SETTINGS_WINDOW_URL_VALUE) {
      return true;
    }
  } catch {
    /* URL 解析失败时回退 label 检测 */
  }
  return tauriWindowLabel() === SETTINGS_WINDOW_LABEL;
}
