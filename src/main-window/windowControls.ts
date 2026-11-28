/**
 * Frameless main-window shell helpers for Tauri 2.
 *
 * Designed for shared use by MainWindow, FolderSidebar, MessageListPane, and ReaderPane
 * headers so `data-tauri-drag-region` dragging, double-click maximize/restore, native
 * Windows system menu (Alt+Space / titlebar right-click), and close-to-tray work
 * consistently while degrading safely in browser preview & unit tests.
 */

import { isTauri } from "../ipc/main-window";

const INTERACTIVE_SELECTOR =
  'button, a, input, select, textarea, label, summary, [role="button"], [role="menuitem"], [role="option"], [data-no-drag]';

/** Returns true when a pointer target sits inside an interactive control that must not drag the window. */
export function isInteractiveDragTarget(target: EventTarget | null): boolean {
  if (!target || typeof (target as Element).closest !== "function") return false;
  return Boolean((target as Element).closest(INTERACTIVE_SELECTOR));
}

async function getTauriCurrentWindow() {
  if (!isTauri()) return null;
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  return getCurrentWindow();
}

export async function startWindowDragging(): Promise<void> {
  const win = await getTauriCurrentWindow();
  if (!win) return;
  await win.startDragging();
}

export async function toggleWindowMaximize(): Promise<void> {
  const win = await getTauriCurrentWindow();
  if (!win) return;
  await win.toggleMaximize();
}

export async function minimizeWindow(): Promise<void> {
  const win = await getTauriCurrentWindow();
  if (!win) return;
  await win.minimize();
}

/** Triggers Tauri CloseRequested (which respects `close_to_minimize` hide-to-tray vs clean quit). */
export async function closeMainWindow(): Promise<void> {
  const win = await getTauriCurrentWindow();
  if (!win) return;
  await win.close();
}

export async function isWindowMaximized(): Promise<boolean> {
  const win = await getTauriCurrentWindow();
  if (!win) return false;
  return win.isMaximized();
}

export async function showWindowSystemMenu(): Promise<void> {
  if (!isTauri()) return;
  const { invoke } = await import("@tauri-apps/api/core");
  await invoke("show_window_system_menu");
}

/** Handle mousedown on a custom topbar drag area when not using native `data-tauri-drag-region`. */
export function onDragRegionMouseDown(e: MouseEvent): void {
  if (e.button !== 0 || isInteractiveDragTarget(e.target)) return;
  if (e.detail === 2) {
    e.preventDefault();
    void toggleWindowMaximize().catch(() => {});
    return;
  }
  if (e.detail === 1) {
    e.preventDefault();
    void startWindowDragging().catch(() => {});
  }
}

/** Handle double-click on a custom topbar drag area to maximize/restore. */
export function onDragRegionDblClick(e: MouseEvent): void {
  if (isInteractiveDragTarget(e.target)) return;
  e.preventDefault();
  void toggleWindowMaximize().catch(() => {});
}

/** Open the native Windows system menu when right-clicking a topbar drag region. */
export function onDragRegionContextMenu(e: MouseEvent): void {
  const target = e.target as Element | null;
  if (!target || typeof target.closest !== "function") return;
  if (!target.closest("[data-tauri-drag-region]")) return;
  if (isInteractiveDragTarget(target)) return;
  e.preventDefault();
  void showWindowSystemMenu().catch(() => {});
}
