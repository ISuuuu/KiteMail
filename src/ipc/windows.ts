import { invoke, isTauri } from "@tauri-apps/api/core";

/**
 * 窗口管理 IPC（WP-01 新增命令，不属于既有偏好/账号契约）。
 * 浏览器预览（非 Tauri）下全部安全返回 false / 抛错由调用方兜底。
 */

/**
 * 打开（或激活已存在的）独立设置窗口。
 * `tab`（WP-10）：入口定位载体（如账号管理入口传 "mailAccounts"），仅新建窗口时生效；
 * 重复打开仅激活既有窗口（Rust 侧单例）。
 */
export async function openSettingsWindow(tab?: string): Promise<boolean> {
  if (!isTauri()) return false;
  await invoke("open_settings_window", { tab: tab ?? null });
  return true;
}
