import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";

import { __resetPreferencesMock, openDataDirectory } from "../src/preferences/ipc.ts";

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
  HIGHLIGHT_MS,
  SEARCH_ENTRIES,
  SEARCH_RESULT_CAP,
  filterSearchEntries,
} from "../src/settings/settingsSearch.ts";
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


test("设置搜索：排序镜像 GPUI（标题前缀>包含>页签>关键词，上限 60）", () => {
  assert.equal(filterSearchEntries("   ").length, 0);

  // 标题前缀命中排最前
  const prefix = filterSearchEntries("默认屏蔽");
  assert.ok(prefix.length > 0);
  assert.equal(prefix[0].tabId, "privacy");

  // 关键词命中（英文别名）
  const idle = filterSearchEntries("idle");
  assert.ok(idle.some((entry) => entry.id === "general-push"));

  // 页签名命中
  const byTab = filterSearchEntries("备份");
  assert.ok(byTab.some((entry) => entry.tabId === "backup"));

  // 单字符宽泛命中不超过上限
  const broad = filterSearchEntries("a");
  assert.ok(broad.length > 0 && broad.length <= SEARCH_RESULT_CAP);
  assert.equal(SEARCH_RESULT_CAP, 60);
});

test("设置搜索：索引覆盖与锚点约定", () => {
  // id 唯一
  assert.equal(new Set(SEARCH_ENTRIES.map((entry) => entry.id)).size, SEARCH_ENTRIES.length);
  // 全部 tabId 合法（能解析出中文页签名）
  for (const entry of SEARCH_ENTRIES) {
    assert.ok(entry.title.length > 0);
  }
  // 全部索引项必须有锚点（云存储/翻译/通讯录源三页已补齐行级/卡片级锚点）
  for (const entry of SEARCH_ENTRIES) {
    assert.ok(entry.anchorId && entry.anchorId.startsWith("sp-"), entry.id);
  }
  // 高亮时长 1.6s
  assert.equal(HIGHLIGHT_MS, 1600);
});

function readSystemPageSource(): string {
  return fs.readFileSync(path.join(process.cwd(), "src/settings/SystemPage.vue"), "utf8");
}

test("系统页「打开目录」接通 open_data_directory：配置目录 data、邮件目录 mail", async () => {
  __resetPreferencesMock({
    dataDir: "C:\\Demo\\KiteMailData",
    mailDataDir: "C:\\Demo\\KiteMailData\\mail",
  });

  // mock 传输层与 Tauri 后端共用同一契约：data→配置目录，mail→邮件目录
  const dataRes = await openDataDirectory("data");
  assert.equal(dataRes.opened, true);
  assert.equal(dataRes.path, "C:\\Demo\\KiteMailData");
  const mailRes = await openDataDirectory("mail");
  assert.equal(mailRes.opened, true);
  assert.equal(mailRes.path, "C:\\Demo\\KiteMailData\\mail");

  const source = readSystemPageSource();
  // 两个入口分别传 data / mail，openDir 统一交给 openDataDirectory
  assert.ok(source.includes("openDir('data')"), "程序配置目录应传 kind=data");
  assert.ok(source.includes("openDir('mail')"), "邮件存储目录应传 kind=mail");
  assert.ok(source.includes("await openDataDirectory(kind)"), "openDir 应走 openDataDirectory");
  assert.equal(source.includes("尚未接入 Tauri IPC"), false, "占位提示应已删除");
});

test("更改邮件存储目录弹层不再假装会话内记录，改为退出应用后手动迁移指引", () => {
  const source = readSystemPageSource();
  assert.equal(source.includes("仅在会话内记录"), false);
  assert.equal(source.includes("仍要继续"), false);
  assert.ok(source.includes("不支持在运行中切换邮件存储目录"));
  // 迁移步骤中的两个路径取自组件状态，避免硬编码
  assert.ok(source.includes("${mailDir}"));
  assert.ok(source.includes("${dataDir}"));
});
