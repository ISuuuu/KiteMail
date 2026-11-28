import test from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";

import { computeReaderOverflowActions } from "../src/main-window/readerHelpers.ts";
import {
  AUTO_RAIL_BREAKPOINT,
  LIST_DEFAULT_WIDTH,
  LIST_MIN_WIDTH,
  LIST_WIDE_OFF,
  LIST_WIDE_ON,
  autoRailNext,
  clampListWidth,
  listWideNext,
} from "../src/main-window/usePaneLayout.ts";

function readSource(relPath: string): string {
  return fs.readFileSync(path.join(process.cwd(), relPath), "utf8");
}

function loadProductionCss(): string {
  const assetsDir = path.join(process.cwd(), "dist-web", "assets");
  const findCss = () =>
    fs.existsSync(assetsDir)
      ? fs.readdirSync(assetsDir).filter((f) => /^index-.*\.css$/.test(f))
      : [];

  let cssFiles = findCss();
  if (cssFiles.length === 0) {
    const npxBin = process.platform === "win32" ? "npx.cmd" : "npx";
    execFileSync(npxBin, ["vite", "build"], { stdio: "pipe" });
    cssFiles = findCss();
  }
  assert.ok(cssFiles.length > 0, "dist-web/assets/index-*.css should exist after vite build");
  return fs.readFileSync(path.join(assetsDir, cssFiles[0]), "utf8");
}

test("整改 1：侧栏选中态统一使用 --km-sidebar-active-bg 且生产 CSS 无 :global 丢失选择器回归", () => {
  const sfcFiles = [
    "src/main-window/FolderSidebar.vue",
    "src/main-window/MessageListPane.vue",
    "src/main-window/ReaderPane.vue",
    "src/main-window/ReaderViewport.vue",
    "src/reader-tools/AttachmentDrawer.vue",
  ];
  for (const file of sfcFiles) {
    const src = readSource(file);
    assert.equal(
      src.includes(":global("),
      false,
      `${file} should not contain scoped :global(...) rules that truncate descendant selectors`
    );
  }

  const sidebarSrc = readSource("src/main-window/FolderSidebar.vue");
  const sidebarActiveClasses = [
    ".km-nav-row.active",
    ".km-subrow-btn.active",
    ".km-folder-btn.active",
    ".km-rail-icon-btn.active",
    ".km-rail-account-btn.active",
    ".km-footer-row.active",
  ];
  for (const sel of sidebarActiveClasses) {
    const escaped = sel.replace(/\./g, "\\.");
    assert.match(
      sidebarSrc,
      new RegExp(`${escaped}\\s*\\{[^}]*background:\\s*var\\(--km-sidebar-active-bg\\);`),
      `${sel} in FolderSidebar.vue should use var(--km-sidebar-active-bg)`
    );
  }

  const prodCss = loadProductionCss();

  // 确保不会再次出现 :global(:root[...]) 编译后丢失组件选择器而污染 :root 的规则
  assert.equal(
    /:root\[data-color-mode=dark\]\{(?:background|border-color):/.test(prodCss),
    false,
    "production CSS must not leak :root[data-color-mode=dark]{background:...}"
  );
  assert.equal(
    /:root\[data-theme=system\]\[data-color-mode=(?:light|dark)\]\{(?:background|border-color):/.test(prodCss),
    false,
    "production CSS must not leak :root[data-theme=system]...{background:...}"
  );

  // 断言生产 CSS 中所有侧栏选中态规则均保留并绑定 --km-sidebar-active-bg
  for (const cls of [
    "km-nav-row",
    "km-subrow-btn",
    "km-folder-btn",
    "km-rail-icon-btn",
    "km-rail-account-btn",
    "km-footer-row",
  ]) {
    assert.match(
      prodCss,
      new RegExp(`\\.${cls}\\.active\\[data-v-[^\\]]+\\]\\{[^}]*background:var\\(--km-sidebar-active-bg\\)`),
      `production CSS missing active rule for .${cls}`
    );
  }

  // 断言浅色 #d8d8db 与深色 #434347 变量定义完整存在于生产 CSS
  assert.ok(prodCss.includes("--km-sidebar-active-bg: #d8d8db"));
  assert.ok(prodCss.includes("--km-sidebar-active-bg: #434347"));
});

test("整改 2：阅读区蓝色账号胶囊、双轨道滑块开关、圆形关闭按钮与窄窗溢出", () => {
  const themeCss = readSource("src/styles/theme.css");
  assert.ok(themeCss.includes("--km-account-pill-bg:"));
  assert.ok(themeCss.includes("--km-account-pill-fg:"));

  const readerSrc = readSource("src/main-window/ReaderPane.vue");
  assert.match(
    readerSrc,
    /\.km-account-pill\s*\{[^}]*border:\s*none;[^}]*background:\s*var\(--km-account-pill-bg/s
  );
  assert.match(readerSrc, /\.km-account-pill\s*\{[^}]*color:\s*var\(--km-account-pill-fg/s);

  // 阅读视图开关用 role="switch"+aria-checked；亮暗开关是三态覆盖，改用 aria-pressed（不声称深色态）
  const switchMatches = readerSrc.match(/role="switch"/g) ?? [];
  assert.equal(switchMatches.length, 1);
  assert.ok(readerSrc.includes(':aria-pressed="isPerMessageDarkActive"'));
  assert.ok(readerSrc.includes(':aria-checked="Boolean(readingView)"'));
  assert.match(
    readerSrc,
    /isPerMessageDarkActive\s*=\s*computed\(\s*\(\)\s*=>\s*props\.perMessageDark !== null && props\.perMessageDark !== undefined/
  );
  assert.match(readerSrc, /class="km-switch-track"[\s\S]*?class="km-switch-thumb"/);
  assert.match(readerSrc, /\.km-switch-track\s*\{[^}]*border-radius:\s*999px;/s);
  assert.match(readerSrc, /\.km-switch-thumb\s*\{[^}]*width:\s*14px;[^}]*height:\s*14px;/s);
  assert.match(readerSrc, /\.km-reader-switch:focus-visible\s*\{[^}]*outline:/s);

  // 圆形关闭按钮与紧凑工具栏
  assert.match(
    readerSrc,
    /\.km-tool-close\s*\{[^}]*width:\s*24px;[^}]*height:\s*24px;[^}]*border-radius:\s*999px;[^}]*background:\s*var\(--km-pill-bg\);/s
  );
  assert.match(readerSrc, /:class="\{\s*compact:\s*isCompactToolbar\s*\}"/);
  assert.match(readerSrc, /toolbarWidth\.value < 430/);

  // 工具栏溢出阈值校验
  assert.deepEqual(computeReaderOverflowActions(673), []);
  assert.deepEqual(computeReaderOverflowActions(560), []);
  assert.deepEqual(computeReaderOverflowActions(559), ["print", "move"]);
  assert.deepEqual(computeReaderOverflowActions(490), ["print", "move"]);
  assert.deepEqual(computeReaderOverflowActions(468), ["print", "move", "spam", "tags"]);
  assert.deepEqual(computeReaderOverflowActions(430), ["print", "move", "spam", "tags"]);
  assert.deepEqual(computeReaderOverflowActions(429), [
    "tags",
    "toggleSeen",
    "spam",
    "move",
    "find",
    "print",
  ]);
});

test("整改 3：列表最小宽度 290px、零宽度占位滚动条与行样式/截断顺序", () => {
  assert.equal(LIST_MIN_WIDTH, 290);
  assert.equal(LIST_DEFAULT_WIDTH, 350);
  assert.equal(clampListWidth(180), 290);
  assert.equal(clampListWidth(280), 290); // 旧下限 280 迁移到 290 的边界用例

  // 保留 600/560 与 1094/1095 断点行为
  assert.equal(LIST_WIDE_ON, 600);
  assert.equal(LIST_WIDE_OFF, 560);
  assert.equal(listWideNext(false, 600), true);
  assert.equal(listWideNext(true, 559), false);
  assert.equal(AUTO_RAIL_BREAKPOINT, 1094);
  assert.equal(autoRailNext(1094), true);
  assert.equal(autoRailNext(1095), false);

  const listSrc = readSource("src/main-window/MessageListPane.vue");
  assert.ok(listSrc.includes('class="km-list-viewport-wrap"'));
  assert.ok(listSrc.includes('class="km-overlay-scrollbar"'));
  assert.match(listSrc, /\.km-list-viewport\s*\{[^}]*scrollbar-width:\s*none;/s);
  assert.match(listSrc, /\.km-list-viewport::-webkit-scrollbar\s*\{[^}]*width:\s*0;[^}]*height:\s*0;/s);
  assert.match(listSrc, /\.km-date-group-header\s*\{[^}]*padding:\s*0 16px;/s);

  const rowSrc = readSource("src/main-window/MessageRow.vue");
  assert.match(rowSrc, /\.km-message-row\s*\{[^}]*margin:\s*2px 6px;[^}]*padding:\s*9px 8px 9px 10px;/s);
  assert.match(rowSrc, /\.km-card-sender\s*\{[^}]*flex:\s*1 1 80px;[^}]*min-width:\s*0;[^}]*font-weight:\s*600;/s);
  assert.match(rowSrc, /\.km-card-badges\s*\{[^}]*flex:\s*0 1 auto;[^}]*min-width:\s*0;/s);
  assert.match(rowSrc, /\.km-date-text\s*\{[^}]*flex:\s*0 1 auto;[^}]*min-width:\s*0;[^}]*text-overflow:\s*ellipsis;/s);
  assert.match(rowSrc, /\.km-tag-chip\s*\{[^}]*font-weight:\s*700;[^}]*flex-shrink:\s*0;/s);
  assert.match(rowSrc, /\.km-card-star\s*\{[^}]*width:\s*14px;/s);
  assert.match(rowSrc, /\.km-card-star:not\(\.flagged\)\s*\{[^}]*opacity:\s*0;[^}]*pointer-events:\s*none;/s);
});

test("整改 4：附件抽屉 100x5 手柄、110px 滑杆、34px 保存按钮与窄窗对齐", () => {
  const drawerSrc = readSource("src/reader-tools/AttachmentDrawer.vue");
  assert.match(drawerSrc, /\.km-drawer-handle-pill\s*\{[^}]*width:\s*100px;[^}]*height:\s*5px;/s);
  assert.match(drawerSrc, /\.km-thumb-slider\s*\{[^}]*width:\s*110px;/s);
  assert.match(drawerSrc, /\.km-btn-save-all\s*\{[^}]*height:\s*34px;/s);
  assert.match(drawerSrc, /\.km-drawer-summary-btn\s*\{[^}]*height:\s*34px;/s);
  assert.match(drawerSrc, /\.km-drawer-header\s*\{[^}]*height:\s*40px;/s);
  assert.match(drawerSrc, /showSizeSlider\s*=\s*computed\(\(\)\s*=>\s*drawerWidth\.value >= 460\)/);
  assert.ok(drawerSrc.includes("km-slider-icon-sm"));
  assert.ok(drawerSrc.includes("km-slider-icon-lg"));
});

test("整改 5：会话卡片 26px 头像、内边距与对齐线且不触碰 native placeholder 逻辑", () => {
  const rustDoc = readSource("crates/windows-mail-reader/src/thread_document.rs");
  assert.ok(rustDoc.includes("padding:14px 16px 16px;"));
  assert.ok(rustDoc.includes(".km-card.km-folded{{background:var(--km-card);padding:12px 16px;}}"));
  assert.ok(rustDoc.includes(".km-ava{{width:26px;height:26px;"));
  assert.ok(rustDoc.includes(".km-rcpts{{padding-left:34px;"));

  const viewportSrc = readSource("src/main-window/ReaderViewport.vue");
  assert.match(viewportSrc, /\.km-thread-card\s*\{[^}]*padding:\s*14px 16px 16px;/s);
  assert.match(viewportSrc, /\.km-thread-card\.folded\s*\{[^}]*padding:\s*12px 16px;/s);
  assert.match(viewportSrc, /\.km-card-avatar\s*\{[^}]*width:\s*26px;[^}]*height:\s*26px;/s);
  assert.match(viewportSrc, /\.km-rcpts-detail\s*\{[^}]*padding-left:\s*34px;/s);

  // 确保未修改原生阅读器占位与就绪控制逻辑
  assert.ok(viewportSrc.includes('class="km-native-placeholder"'));
  assert.match(
    viewportSrc,
    /nativeHostActive\s*=\s*computed\(\s*\(\)\s*=>\s*tauriMode &&\s*props\.bodyStatus === "ready"/s
  );
});
