import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";

import { BUILTIN_THEMES } from "../src/ipc/types.ts";
import { mainBootstrap, resetMockSession } from "../src/ipc/main-window.ts";
import { usePaneLayout } from "../src/main-window/usePaneLayout.ts";

function readSource(relPath: string): string {
  return fs.readFileSync(path.join(process.cwd(), relPath), "utf8");
}

test("MainWindow 根据阅读窗格位置正确分配右上角关窗按钮并接线主题", () => {
  const mainSrc = readSource("src/main-window/MainWindow.vue");

  // FolderSidebar 接线 themeId 与 setTheme
  const sidebarBlock = mainSrc.match(/<FolderSidebar[\s\S]*?\/>/)?.[0] ?? "";
  assert.ok(sidebarBlock.includes(':themeId="state.layout.prefs.value.themeId"'));
  assert.ok(sidebarBlock.includes('@setTheme="state.layout.setTheme"'));

  // MessageListPane 在非 right（below/off）模式下接管右上角关窗按钮
  const listBlock = mainSrc.match(/<MessageListPane[\s\S]*?\/>/)?.[0] ?? "";
  assert.ok(
    listBlock.includes(':showWindowClose="state.layout.prefs.value.readerPosition !== \'right\'"')
  );
  assert.ok(listBlock.includes('@closeWindow="state.closeWindow()"'));

  // below 模式 ReaderPane 禁用关窗按钮，right 模式 ReaderPane 启用关窗按钮
  const belowBlock =
    mainSrc.match(/<ReaderPane[\s\S]*?ref="belowReaderPaneRef"[\s\S]*?\/>/)?.[0] ?? "";
  assert.ok(belowBlock.includes(':showWindowClose="false"'));

  const rightBlock =
    mainSrc.match(/<ReaderPane[\s\S]*?ref="rightReaderPaneRef"[\s\S]*?\/>/)?.[0] ?? "";
  assert.ok(rightBlock.includes(':showWindowClose="true"'));
});

test("MessageListPane 支持可选 showWindowClose 并在顶部右侧渲染一致 SVG 关窗按钮", () => {
  const listSrc = readSource("src/main-window/MessageListPane.vue");
  assert.match(listSrc, /showWindowClose\?:\s*boolean/);
  assert.match(listSrc, /showWindowClose:\s*false/);
  assert.match(listSrc, /closeWindow:\s*\[\]/);
  assert.ok(listSrc.includes('v-if="showWindowClose"'));
  assert.ok(listSrc.includes('class="km-icon-btn km-window-close-btn"'));
  assert.ok(listSrc.includes('title="关闭主窗口"'));
  assert.ok(listSrc.includes('aria-label="关闭窗口"'));
  assert.ok(listSrc.includes("@click=\"emit('closeWindow')\""));
});

test("FolderSidebar 导入 BUILTIN_THEMES、在主菜单展示 7 套应用配色并绑定 setTheme", async () => {
  const sidebarSrc = readSource("src/main-window/FolderSidebar.vue");
  assert.match(sidebarSrc, /import\s*\{[\s\S]*BUILTIN_THEMES[\s\S]*\}\s*from\s*"\.\.\/ipc\/types"/);
  assert.match(sidebarSrc, /themeId\?:\s*string/);
  assert.match(sidebarSrc, /themeId:\s*"system"/);
  assert.match(sidebarSrc, /setTheme:\s*\[themeId:\s*string\]/);
  assert.ok(sidebarSrc.includes("v-for=\"theme in BUILTIN_THEMES\""));
  assert.ok(sidebarSrc.includes("theme.swatch"));
  assert.ok(sidebarSrc.includes("theme.label"));
  assert.ok(sidebarSrc.includes("runMenuAction(() => emit('setTheme', theme.id))"));

  // 运行时校验 7 套内置主题均可经由 usePaneLayout.setTheme 切换并持久化
  assert.equal(BUILTIN_THEMES.length, 7);
  resetMockSession({ locked: false });
  const boot = await mainBootstrap();
  const layout = usePaneLayout();
  layout.applyPrefs(boot.preferences);
  for (const theme of BUILTIN_THEMES) {
    layout.setTheme(theme.id);
    assert.equal(layout.prefs.value.themeId, theme.id);
  }
  await new Promise((r) => setTimeout(r, 10));
  assert.equal((await mainBootstrap()).preferences.themeId, BUILTIN_THEMES.at(-1)?.id);
  resetMockSession();
});

test("ReaderPane 支持 showWindowClose 按需显示，清理残余字符图标并保持标题区高度基线", () => {
  const readerSrc = readSource("src/main-window/ReaderPane.vue");
  assert.match(readerSrc, /showWindowClose\?:\s*boolean/);
  assert.match(readerSrc, /showWindowClose:\s*true/);
  assert.ok(readerSrc.includes('v-if="showWindowClose"'));

  // 不得残留 ✉ emoji 以及标签勾选 ✓ / 查找关闭 ✕ 字符
  assert.equal(readerSrc.includes("✉"), false);
  assert.equal(readerSrc.includes("✓"), false);
  assert.equal(readerSrc.includes("✕"), false);

  // 空状态信封改用 ~34px stroke="currentColor" SVG
  const emptyBlock =
    readerSrc.match(/<div class="km-empty-icon"[\s\S]*?<\/div>/)?.[0] ?? "";
  assert.ok(emptyBlock.includes("<svg"));
  assert.ok(emptyBlock.includes('width="34"'));
  assert.ok(emptyBlock.includes('stroke="currentColor"'));
  assert.ok(emptyBlock.includes('aria-hidden="true"'));

  // 工具栏 48px 与 22px 主题标题基线样式存在且不发生裁切
  assert.match(readerSrc, /\.km-reader-toolbar\s*\{[^}]*height:\s*48px;/);
  assert.match(readerSrc, /\.km-subject-title\s*\{[^}]*font-size:\s*22px;/);
  assert.match(readerSrc, /\.km-reader-subject-header\s*\{[^}]*padding:\s*4px 16px 10px 21px;/);
});
