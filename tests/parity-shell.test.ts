import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";

import {
  AUTO_RAIL_BREAKPOINT,
  LIST_DEFAULT_WIDTH,
  LIST_MIN_WIDTH,
  LIST_WIDE_OFF,
  LIST_WIDE_ON,
  READER_MIN_WIDTH,
  SIDEBAR_DEFAULT_WIDTH,
  SIDEBAR_RAIL_WIDTH,
  SPLITTER_THICKNESS,
  usePaneLayout,
} from "../src/main-window/usePaneLayout.ts";
import { isInteractiveDragTarget } from "../src/main-window/windowControls.ts";
import { mainBootstrap, resetMockSession } from "../src/ipc/main-window.ts";

test("三栏默认尺寸、80px 窄轨、1094px 断点与客户区宽度约束对齐视觉基准", async () => {
  assert.equal(SIDEBAR_DEFAULT_WIDTH, 255);
  assert.equal(LIST_DEFAULT_WIDTH, 350);
  assert.equal(SIDEBAR_RAIL_WIDTH, 80);
  assert.equal(SPLITTER_THICKNESS, 1);
  assert.equal(AUTO_RAIL_BREAKPOINT, 1094);
  assert.equal(LIST_WIDE_ON, 600);
  assert.equal(LIST_WIDE_OFF, 560);
  assert.equal(READER_MIN_WIDTH, 441);

  resetMockSession({ locked: false });
  const boot = await mainBootstrap();
  assert.equal(boot.preferences.sidebarWidth, 255);
  assert.equal(boot.preferences.listWidth, 350);

  const layout = usePaneLayout();
  layout.applyPrefs(boot.preferences);
  layout.setWindowSize(1280, 840);

  // 1280 = 255 (sidebar) + 1 (splitter) + 350 (list) + 1 (splitter) + 673 (reader)
  assert.equal(layout.autoRail.value, false);
  assert.equal(layout.isRail.value, false);
  assert.equal(layout.sidebarVisualWidth.value, 255);
  assert.equal(layout.listVisualWidth.value, 350);
  assert.equal(layout.listWide.value, false);
  assert.equal(
    1280 - layout.sidebarVisualWidth.value - layout.listVisualWidth.value - SPLITTER_THICKNESS * 2,
    673
  );

  // 1094px 触发 80px 自动窄轨，1095px 恢复 255px
  layout.setWindowSize(1094, 840);
  assert.equal(layout.autoRail.value, true);
  assert.equal(layout.sidebarVisualWidth.value, 80);
  layout.setWindowSize(1095, 840);
  assert.equal(layout.autoRail.value, false);
  assert.equal(layout.sidebarVisualWidth.value, 255);

  // 宽窗下将列表拖宽到 520px，缩窗至 900px 时实际渲染受可用宽度约束（保证阅读区 >= 441px），
  // 同时保留用户偏好 520px；恢复宽窗后自动回到 520px。
  layout.setWindowSize(1400, 840);
  layout.onListResize(520);
  assert.equal(layout.prefs.value.listWidth, 520);
  assert.equal(layout.listVisualWidth.value, 520);

  layout.setWindowSize(900, 840);
  assert.equal(layout.sidebarVisualWidth.value, 80);
  assert.equal(layout.maxListWidth.value, 900 - 80 - READER_MIN_WIDTH - SPLITTER_THICKNESS * 2);
  assert.equal(layout.listVisualWidth.value, 377);
  assert.equal(layout.prefs.value.listWidth, 520);
  assert.ok(
    900 - layout.sidebarVisualWidth.value - layout.listVisualWidth.value - SPLITTER_THICKNESS * 2 >=
      READER_MIN_WIDTH
  );

  layout.setWindowSize(1400, 840);
  assert.equal(layout.listVisualWidth.value, 520);

  // 列表宽窄迟滞（600 进入 / <560 退出）随缩窗约束同步生效
  layout.onListResize(620);
  assert.equal(layout.listWide.value, true);
  layout.setWindowSize(1200, 840); // max = 1200 - 255 - 441 - 2 = 502 (< 560)
  assert.equal(layout.listVisualWidth.value, 502);
  assert.equal(layout.listWide.value, false);
  layout.setWindowSize(1300, 840); // max = 1300 - 255 - 441 - 2 = 602 (>= 600)
  assert.equal(layout.listVisualWidth.value, 602);
  assert.equal(layout.listWide.value, true);

  layout.flushSave();
  resetMockSession();
});

test("无边框 Tauri 窗口配置、权限与分隔件 1px 布局契约", () => {
  const root = process.cwd();
  const tauriConf = JSON.parse(
    fs.readFileSync(path.join(root, "src-tauri/tauri.conf.json"), "utf8")
  ) as {
    app: { windows: Array<Record<string, unknown>> };
  };
  const mainWin = tauriConf.app.windows[0];
  assert.equal(mainWin.decorations, false);
  assert.equal(mainWin.width, 1280);
  assert.equal(mainWin.height, 840);
  assert.equal(mainWin.minWidth, 900);
  assert.equal(mainWin.minHeight, 600);
  assert.equal(mainWin.resizable, true);

  const cap = JSON.parse(
    fs.readFileSync(path.join(root, "src-tauri/capabilities/main.json"), "utf8")
  ) as { permissions: string[] };
  for (const perm of [
    "core:window:allow-start-dragging",
    "core:window:allow-toggle-maximize",
    "core:window:allow-minimize",
    "core:window:allow-close",
    "allow-show-window-system-menu",
  ]) {
    assert.ok(cap.permissions.includes(perm), `missing capability permission: ${perm}`);
  }

  const splitterSrc = fs.readFileSync(
    path.join(root, "src/main-window/PaneSplitter.vue"),
    "utf8"
  );
  assert.match(splitterSrc, /\.km-splitter-vertical\s*\{[^}]*width:\s*1px;/);
  assert.match(splitterSrc, /\.km-splitter-horizontal\s*\{[^}]*height:\s*1px;/);
  assert.ok(splitterSrc.includes(".km-splitter-vertical::before"));

  const buttonTarget = { closest: (sel: string) => (sel.includes("button") ? {} : null) } as unknown as EventTarget;
  const plainTarget = { closest: () => null } as unknown as EventTarget;
  assert.equal(isInteractiveDragTarget(buttonTarget), true);
  assert.equal(isInteractiveDragTarget(plainTarget), false);
});

test("theme.css 包含主窗口亮暗原图取色语义变量", () => {
  const css = fs.readFileSync(path.join(process.cwd(), "src/styles/theme.css"), "utf8");
  const expectedHexes = [
    "#ebebed",
    "#2e2e32",
    "#fafafb",
    "#222226",
    "#f1f1f1",
    "#131315",
    "#ffffff",
    "#1d1d20",
    "#f4f4f5",
    "#29292d",
    "#3584e4",
    "#d8d8db",
    "#434347",
    "#d2e1f5",
    "#213046",
    "#d5f0e5",
    "#243f36",
  ];
  for (const hex of expectedHexes) {
    assert.ok(css.toLowerCase().includes(hex), `missing sampled color ${hex} in theme.css`);
  }
  assert.ok(css.includes("--km-drawer-bg:"));
  assert.ok(css.includes("--km-sidebar-active-bg:"));
  assert.ok(css.includes("--km-tag-work-bg:"));
  assert.ok(css.includes("--km-tag-personal-bg:"));
});
