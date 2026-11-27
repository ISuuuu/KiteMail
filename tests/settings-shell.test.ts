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
import {
  TOOLBAR_ITEMS,
  TOOLBAR_LEFT_MAX,
  defaultToolbarLayout,
  indexInZone,
  moveWithin,
  place,
  toolbarSide,
} from "../src/settings/readerToolbar.ts";

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

test("阅读器工具栏默认布局对齐 GPUI ReaderToolbarLayout::default", () => {
  assert.equal(TOOLBAR_ITEMS.length, 13);
  const layout = defaultToolbarLayout();
  assert.deepEqual(layout.left, ["reply", "replyAll", "forward", "star", "archive", "spam", "delete"]);
  assert.deepEqual(layout.right, ["snooze", "tags", "readUnread", "moveTo", "find", "print"]);
  assert.equal(toolbarSide(layout, "reply"), "left");
  assert.equal(toolbarSide(layout, "tags"), "right");
  assert.equal(toolbarSide(layout, "nope"), null);
});

test("工具栏 move_within：组内上移下移、越界与隐藏项不动", () => {
  const layout = defaultToolbarLayout();
  // 上移首项不动
  assert.deepEqual(moveWithin(layout, "reply", -1), layout);
  // 下移末项不动
  assert.deepEqual(moveWithin(layout, "print", 1), layout);
  // 左组内上移：archive 从 index 4 移到 index 3
  const moved = moveWithin(layout, "archive", -1);
  assert.deepEqual(moved.left.slice(2, 6), ["forward", "archive", "star", "spam"]);
  // 隐藏项不动
  const hidden = place(layout, "reply", null);
  assert.deepEqual(moveWithin(hidden, "reply", -1), hidden);
});

test("工具栏 place：隐藏/放置与左组 7 上限钳制", () => {
  const layout = defaultToolbarLayout();
  // 隐藏
  const hidden = place(layout, "reply", null);
  assert.equal(toolbarSide(hidden, "reply"), null);
  assert.equal(hidden.left.length, 6);
  // 从隐藏恢复到右组
  const restored = place(hidden, "reply", "right");
  assert.equal(toolbarSide(restored, "reply"), "right");
  // 左组已满（7）：其他项放入左组不动
  const full = place(layout, "tags", "left");
  assert.equal(toolbarSide(full, "tags"), "right");
  assert.equal(full.left.length, TOOLBAR_LEFT_MAX);
  // 左组未满时可放入
  const afterHide = place(hidden, "reply", "left");
  assert.equal(afterHide.left.length, 7);
  assert.equal(indexInZone(afterHide, "reply"), 7 - 1);
});
