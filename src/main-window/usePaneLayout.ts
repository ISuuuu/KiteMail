import { ref, computed, onUnmounted, getCurrentInstance } from "vue";
import type { MainPreferencesDto, MainPreferencesPatch, ReaderPosition } from "../ipc/types";
import { updateMainPreferences } from "../ipc/main-window";

// --- 布局常量（精确对齐 crates/app/src/app.rs） ---
export const SIDEBAR_MIN_WIDTH = 180;
export const SIDEBAR_MAX_WIDTH = 280;
export const SIDEBAR_DEFAULT_WIDTH = 240;
export const LIST_MIN_WIDTH = 280;
export const LIST_DEFAULT_WIDTH = 360;
export const READER_MIN_WIDTH = 441;
export const BELOW_LIST_MIN_HEIGHT = 240;
export const BELOW_LIST_DEFAULT_HEIGHT = 360;
export const BELOW_READER_MIN_HEIGHT = 180;
// 自动窄轨断点（<= 1094px）
export const AUTO_RAIL_BREAKPOINT = 1094;
// 列表迟滞：>= 600 进入单行表格，< 560 退回卡片
export const LIST_WIDE_ON = 600;
export const LIST_WIDE_OFF = 560;
// 防抖时长
export const SAVE_DEBOUNCE_MS = 600;

export const KNOWN_THEMES = [
  "system",
  "default",
  "midnight",
  "tidal",
  "rose",
  "earth",
  "forest",
] as const;

/** 单行表格 / 卡片模式迟滞计算 */
export function listWideNext(currentWide: boolean, width: number): boolean {
  if (!currentWide && width >= LIST_WIDE_ON) return true;
  if (currentWide && width < LIST_WIDE_OFF) return false;
  return currentWide;
}

/** 窗口自动窄轨计算（<= 1094px 进入窄轨） */
export function autoRailNext(windowWidth: number): boolean {
  return windowWidth <= AUTO_RAIL_BREAKPOINT;
}

/** 未知阅读布局键回落为 right */
export function validateReaderPosition(raw: string): ReaderPosition {
  if (raw === "right" || raw === "below" || raw === "off") return raw;
  return "right";
}

export function clampSidebarWidth(width: number): number {
  return Math.round(Math.min(SIDEBAR_MAX_WIDTH, Math.max(SIDEBAR_MIN_WIDTH, width)));
}

export function clampListWidth(width: number, maxListWidth?: number): number {
  const minClamped = Math.max(LIST_MIN_WIDTH, Math.round(width));
  if (maxListWidth !== undefined && maxListWidth >= LIST_MIN_WIDTH) {
    return Math.min(Math.round(maxListWidth), minClamped);
  }
  return minClamped;
}

export function clampBelowHeight(height: number, maxBelowHeight?: number): number {
  const minClamped = Math.max(BELOW_LIST_MIN_HEIGHT, Math.round(height));
  if (maxBelowHeight !== undefined && maxBelowHeight >= BELOW_LIST_MIN_HEIGHT) {
    return Math.min(Math.round(maxBelowHeight), minClamped);
  }
  return minClamped;
}

export function usePaneLayout() {
  const prefs = ref<MainPreferencesDto>({
    themeId: "system",
    darkMode: false,
    readerPosition: "right",
    sidebarWidth: SIDEBAR_DEFAULT_WIDTH,
    listWidth: LIST_DEFAULT_WIDTH,
    listBelowHeight: BELOW_LIST_DEFAULT_HEIGHT,
    sidebarCollapsed: false,
    compactList: false,
    showAvatars: true,
    listGroupByDate: true,
    readerZoomPercent: 100,
  });

  // 窗口宽高（由 ResizeObserver 更新）
  const windowWidth = ref(typeof window !== "undefined" ? window.innerWidth : 1280);
  const windowHeight = ref(typeof window !== "undefined" ? window.innerHeight : 800);

  // `system` 主题下是否跟随 OS prefers-color-scheme
  const followSystemScheme = ref(true);

  // 浮层遮挡状态：菜单/下拉打开时为 true，通知原生 WebView2 隐藏
  const activeOverlays = ref<Set<string>>(new Set());
  const overlayOccluded = computed(() => activeOverlays.value.size > 0);

  function setOverlayOpen(id: string, open: boolean) {
    const next = new Set(activeOverlays.value);
    if (open) {
      next.add(id);
    } else {
      next.delete(id);
    }
    activeOverlays.value = next;
  }

  function clearOverlays() {
    if (activeOverlays.value.size > 0) {
      activeOverlays.value = new Set();
    }
  }

  // 自动窄轨（<= 1094px，不落盘，回到宽窗时自动恢复用户手动偏好）
  const autoRail = computed(() => autoRailNext(windowWidth.value));

  // 是否处于图标窄轨（自动或手动折叠）
  const isRail = computed(() => autoRail.value || prefs.value.sidebarCollapsed);

  // 实际侧栏视觉宽度（窄轨 60px）
  const sidebarVisualWidth = computed(() => (isRail.value ? 60 : prefs.value.sidebarWidth));

  // 列表宽模式迟滞
  const listWide = ref(prefs.value.listWidth >= LIST_WIDE_ON);

  // 生效的亮暗模式（同步 DOM data-color-mode）
  const effectiveDark = ref(false);

  // --- 防抖落盘 ---
  let saveTimer: ReturnType<typeof setTimeout> | null = null;
  let pendingPatch: MainPreferencesPatch = {};

  function scheduleSave(patch: MainPreferencesPatch) {
    Object.assign(pendingPatch, patch);
    if (saveTimer) clearTimeout(saveTimer);
    saveTimer = setTimeout(() => {
      const p = pendingPatch;
      pendingPatch = {};
      saveTimer = null;
      updateMainPreferences(p).catch(() => {
        /* 忽略网络/预览落盘异常 */
      });
    }, SAVE_DEBOUNCE_MS);
  }

  function flushSave() {
    if (saveTimer) {
      clearTimeout(saveTimer);
      saveTimer = null;
    }
    if (Object.keys(pendingPatch).length > 0) {
      const p = pendingPatch;
      pendingPatch = {};
      updateMainPreferences(p).catch(() => {
        /* 忽略 */
      });
    }
  }

  function syncDocumentTheme() {
    if (typeof document === "undefined") return;
    const root = document.documentElement;
    const id = KNOWN_THEMES.includes(prefs.value.themeId as (typeof KNOWN_THEMES)[number])
      ? prefs.value.themeId
      : "system";

    let dark = prefs.value.darkMode;
    if (id === "system" && followSystemScheme.value && typeof window !== "undefined" && window.matchMedia) {
      dark = window.matchMedia("(prefers-color-scheme: dark)").matches;
    }
    effectiveDark.value = dark;
    root.setAttribute("data-theme", id);
    root.setAttribute("data-color-mode", dark ? "dark" : "light");
  }

  // 监听系统深浅色变更
  let mediaQuery: MediaQueryList | null = null;
  function onSystemSchemeChange() {
    if (prefs.value.themeId === "system" && followSystemScheme.value) {
      syncDocumentTheme();
    }
  }

  if (typeof window !== "undefined") {
    window.addEventListener("beforeunload", flushSave);
    if (window.matchMedia) {
      mediaQuery = window.matchMedia("(prefers-color-scheme: dark)");
      mediaQuery.addEventListener("change", onSystemSchemeChange);
    }
  }

  if (getCurrentInstance()) {
    onUnmounted(() => {
      flushSave();
      if (typeof window !== "undefined") {
        window.removeEventListener("beforeunload", flushSave);
      }
      mediaQuery?.removeEventListener("change", onSystemSchemeChange);
    });
  }

  // --- 操作方法 ---

  function applyPrefs(p: MainPreferencesDto) {
    const nextListWidth = clampListWidth(p.listWidth ?? LIST_DEFAULT_WIDTH);
    prefs.value = {
      ...prefs.value,
      ...p,
      themeId: KNOWN_THEMES.includes(p.themeId as (typeof KNOWN_THEMES)[number])
        ? p.themeId
        : "system",
      sidebarWidth: clampSidebarWidth(p.sidebarWidth ?? SIDEBAR_DEFAULT_WIDTH),
      listWidth: nextListWidth,
      listBelowHeight: clampBelowHeight(p.listBelowHeight ?? BELOW_LIST_DEFAULT_HEIGHT),
      readerPosition: validateReaderPosition(p.readerPosition),
      readerZoomPercent: Math.min(250, Math.max(50, p.readerZoomPercent ?? 100)),
    };
    listWide.value = prefs.value.listWidth >= LIST_WIDE_ON;
    followSystemScheme.value = prefs.value.themeId === "system";
    syncDocumentTheme();
  }

  function onSidebarResize(newWidth: number) {
    const clamped = clampSidebarWidth(newWidth);
    if (clamped === prefs.value.sidebarWidth) return;
    prefs.value = { ...prefs.value, sidebarWidth: clamped };
    scheduleSave({ sidebarWidth: clamped });
  }

  function onListResize(newWidth: number) {
    const maxAllowed = Math.max(
      LIST_MIN_WIDTH,
      windowWidth.value - sidebarVisualWidth.value - READER_MIN_WIDTH - 10
    );
    const clamped = clampListWidth(newWidth, maxAllowed);
    if (clamped === prefs.value.listWidth) return;
    prefs.value = { ...prefs.value, listWidth: clamped };
    listWide.value = listWideNext(listWide.value, clamped);
    scheduleSave({ listWidth: clamped });
  }

  function onBelowResize(newHeight: number) {
    const maxAllowed = Math.max(
      BELOW_LIST_MIN_HEIGHT,
      windowHeight.value - BELOW_READER_MIN_HEIGHT - 10
    );
    const clamped = clampBelowHeight(newHeight, maxAllowed);
    if (clamped === prefs.value.listBelowHeight) return;
    prefs.value = { ...prefs.value, listBelowHeight: clamped };
    scheduleSave({ listBelowHeight: clamped });
  }

  function toggleSidebarCollapsed() {
    const next = !prefs.value.sidebarCollapsed;
    prefs.value = { ...prefs.value, sidebarCollapsed: next };
    updateMainPreferences({ sidebarCollapsed: next }).catch(() => {
      /* 忽略 */
    });
  }

  function setReaderPosition(pos: ReaderPosition) {
    const valid = validateReaderPosition(pos);
    if (prefs.value.readerPosition === valid) return;
    prefs.value = { ...prefs.value, readerPosition: valid };
    if (valid !== "right") {
      // 非右侧阅读模式时中栏铺满窗口剩余宽度，同步判断宽表格模式
      const avail = Math.max(LIST_MIN_WIDTH, windowWidth.value - sidebarVisualWidth.value - 5);
      listWide.value = listWideNext(listWide.value, avail);
    } else {
      listWide.value = listWideNext(listWide.value, prefs.value.listWidth);
    }
    updateMainPreferences({ readerPosition: valid }).catch(() => {
      /* 忽略 */
    });
  }

  function setTheme(themeId: string, darkMode?: boolean) {
    const validTheme = KNOWN_THEMES.includes(themeId as (typeof KNOWN_THEMES)[number])
      ? themeId
      : "system";
    if (darkMode !== undefined) {
      followSystemScheme.value = false;
      prefs.value = { ...prefs.value, themeId: validTheme, darkMode };
      syncDocumentTheme();
      updateMainPreferences({ themeId: validTheme, darkMode }).catch(() => {
        /* 忽略 */
      });
    } else {
      followSystemScheme.value = validTheme === "system";
      prefs.value = { ...prefs.value, themeId: validTheme };
      syncDocumentTheme();
      updateMainPreferences({ themeId: validTheme }).catch(() => {
        /* 忽略 */
      });
    }
  }

  function toggleDarkMode() {
    followSystemScheme.value = false;
    const nextDark = !effectiveDark.value;
    prefs.value = { ...prefs.value, darkMode: nextDark };
    syncDocumentTheme();
    updateMainPreferences({ darkMode: nextDark }).catch(() => {
      /* 忽略 */
    });
  }

  function toggleFollowSystem() {
    followSystemScheme.value = true;
    prefs.value = { ...prefs.value, themeId: "system" };
    syncDocumentTheme();
    updateMainPreferences({ themeId: "system" }).catch(() => {
      /* 忽略 */
    });
  }

  function setWindowSize(width: number, height: number) {
    windowWidth.value = width;
    windowHeight.value = height;
    if (prefs.value.readerPosition !== "right") {
      const avail = Math.max(LIST_MIN_WIDTH, width - sidebarVisualWidth.value - 5);
      listWide.value = listWideNext(listWide.value, avail);
    }
  }

  function setWindowWidth(width: number) {
    setWindowSize(width, windowHeight.value);
  }

  return {
    prefs,
    windowWidth,
    windowHeight,
    effectiveDark,
    followSystemScheme,
    overlayOccluded,
    autoRail,
    isRail,
    listWide,
    sidebarVisualWidth,
    applyPrefs,
    onSidebarResize,
    onListResize,
    onBelowResize,
    toggleSidebarCollapsed,
    setReaderPosition,
    setTheme,
    toggleDarkMode,
    toggleFollowSystem,
    setWindowWidth,
    setWindowSize,
    setOverlayOpen,
    clearOverlays,
    flushSave,
  };
}

export type UsePaneLayoutReturn = ReturnType<typeof usePaneLayout>;
