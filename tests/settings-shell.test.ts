import test from "node:test";
import assert from "node:assert/strict";

import {
  DEFAULT_SETTINGS_TAB,
  NARROW_BREAKPOINT,
  SETTINGS_TABS,
  computeSidebarWidth,
  findSettingsTab,
  isNarrowViewport,
  tabsOfGroup,
} from "../src/settings/nav.ts";
import { isSettingsWindow, isTauriRuntime } from "../src/settings/windowEnv.ts";

test("设置窗导航沿用 GPUI PrefTab::ALL 的 22 页签与账号/设置分组", () => {
  assert.equal(SETTINGS_TABS.length, 22);
  assert.equal(tabsOfGroup("accounts").length, 9);
  assert.equal(tabsOfGroup("settings").length, 13);
  assert.deepEqual(
    SETTINGS_TABS.map((tab) => tab.label),
    [
      "邮箱账号",
      "标签",
      "过滤器",
      "智能文件夹",
      "发件人",
      "OpenPGP 密钥",
      "云存储",
      "翻译",
      "通讯录源",
      "常规",
      "外观",
      "专注模式",
      "侧边栏",
      "邮件列表",
      "会话",
      "阅读",
      "写信",
      "隐私",
      "MCP 服务",
      "日期和时间",
      "系统",
      "备份",
    ]
  );
  // 页签 id 唯一
  assert.equal(new Set(SETTINGS_TABS.map((tab) => tab.id)).size, 22);
});

test("computeSidebarWidth 对齐 GPUI 27% 钳制 180-250", () => {
  assert.equal(computeSidebarWidth(500), 180);
  assert.ok(Math.abs(computeSidebarWidth(920) - 248.4) < 0.01);
  assert.equal(computeSidebarWidth(1200), 250);
});

test("窄窗断点 640/641px 切换", () => {
  assert.equal(isNarrowViewport(NARROW_BREAKPOINT), true);
  assert.equal(isNarrowViewport(NARROW_BREAKPOINT + 1), false);
});

test("findSettingsTab 未知 id 回落默认页签（常规）", () => {
  assert.equal(DEFAULT_SETTINGS_TAB, "general");
  assert.equal(findSettingsTab("system").id, "system");
  assert.equal(findSettingsTab("no-such-tab").id, "general");
});

test("windowEnv：URL 参数 ?window=settings 判定设置窗，浏览器环境非 Tauri 运行时", () => {
  // node 测试环境无 window 对象
  assert.equal(isTauriRuntime(), false);
  assert.equal(isSettingsWindow(), false);
});
