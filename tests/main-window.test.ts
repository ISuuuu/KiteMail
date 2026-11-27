import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";

import {
  AUTO_RAIL_BREAKPOINT,
  BELOW_LIST_MIN_HEIGHT,
  LIST_MIN_WIDTH,
  LIST_WIDE_OFF,
  LIST_WIDE_ON,
  SIDEBAR_MAX_WIDTH,
  SIDEBAR_MIN_WIDTH,
  autoRailNext,
  clampBelowHeight,
  clampListWidth,
  clampSidebarWidth,
  listWideNext,
  validateReaderPosition,
} from "../src/main-window/usePaneLayout.ts";
import {
  dateGroupLabel,
  mergePagedMessages,
  shouldApplyListResponse,
  shouldApplyOpenResponse,
} from "../src/main-window/useMainWindow.ts";
import {
  extractErrorMessage,
  listMessages,
  mainBootstrap,
  openMessage,
  setReaderLayout,
  updateMainPreferences,
} from "../src/ipc/main-window.ts";
import type { MessageSummaryDto } from "../src/ipc/types.ts";

test("listWideNext 迟滞阈值完全对齐 (600 进入 / <560 退出)", () => {
  assert.equal(LIST_WIDE_ON, 600);
  assert.equal(LIST_WIDE_OFF, 560);
  assert.equal(listWideNext(false, 559), false);
  assert.equal(listWideNext(false, 599), false);
  assert.equal(listWideNext(false, 600), true);
  assert.equal(listWideNext(false, 900), true);

  assert.equal(listWideNext(true, 600), true);
  assert.equal(listWideNext(true, 560), true);
  assert.equal(listWideNext(true, 599), true);
  assert.equal(listWideNext(true, 559), false);
});

test("autoRailNext 窗口宽度 <= 1094 进入窄轨", () => {
  assert.equal(AUTO_RAIL_BREAKPOINT, 1094);
  assert.equal(autoRailNext(960), true);
  assert.equal(autoRailNext(1094), true);
  assert.equal(autoRailNext(1095), false);
  assert.equal(autoRailNext(1280), false);
});

test("布局尺寸限制与未知阅读位置回落", () => {
  assert.equal(clampSidebarWidth(100), SIDEBAR_MIN_WIDTH);
  assert.equal(clampSidebarWidth(240), 240);
  assert.equal(clampSidebarWidth(400), SIDEBAR_MAX_WIDTH);

  assert.equal(clampListWidth(120), LIST_MIN_WIDTH);
  assert.equal(clampListWidth(500, 420), 420);

  assert.equal(clampBelowHeight(100), BELOW_LIST_MIN_HEIGHT);
  assert.equal(clampBelowHeight(360), 360);

  assert.equal(validateReaderPosition("right"), "right");
  assert.equal(validateReaderPosition("below"), "below");
  assert.equal(validateReaderPosition("off"), "off");
  assert.equal(validateReaderPosition("unknown"), "right");
});

test("dateGroupLabel 日期分组边界正确", () => {
  const now = Math.floor(new Date(2026, 10, 27, 15, 0, 0).getTime() / 1000);
  const todayMsg = Math.floor(new Date(2026, 10, 27, 8, 0, 0).getTime() / 1000);
  const yesterdayMsg = Math.floor(new Date(2026, 10, 26, 20, 0, 0).getTime() / 1000);
  const sixDaysAgo = Math.floor(new Date(2026, 10, 21, 12, 0, 0).getTime() / 1000);
  const tenDaysAgo = Math.floor(new Date(2026, 10, 17, 12, 0, 0).getTime() / 1000);
  const twentyDaysAgo = Math.floor(new Date(2026, 10, 7, 12, 0, 0).getTime() / 1000);

  assert.equal(dateGroupLabel(todayMsg, now), "今天");
  assert.equal(dateGroupLabel(yesterdayMsg, now), "昨天");
  assert.equal(dateGroupLabel(sixDaysAgo, now), "最近 7 天");
  assert.equal(dateGroupLabel(tenDaysAgo, now), "最近 14 天");
  assert.equal(dateGroupLabel(twentyDaysAgo, now), "更早");
  assert.equal(dateGroupLabel(0, now), "更早");
});

test("mergePagedMessages 使用 accountId:id 稳定键去重与重置", () => {
  const makeMsg = (accountId: number, id: number): MessageSummaryDto => ({
    id,
    accountId,
    folderId: 1,
    folderPath: "INBOX",
    sender: `s${id}@example.com`,
    recipients: "me@example.com",
    subject: `subj-${accountId}-${id}`,
    preview: "",
    dateTs: 1000,
    dateDisplay: "今天",
    unread: false,
    flagged: false,
    hasAttachment: false,
    threadCount: 1,
    importance: 0,
    tags: [],
    answered: false,
    forwarded: false,
  });

  const page1 = [makeMsg(1, 1), makeMsg(1, 2)];
  const page2 = [makeMsg(1, 2), makeMsg(2, 2), makeMsg(1, 3)];
  const merged = mergePagedMessages(page1, page2, false);
  assert.deepEqual(
    merged.map((m) => `${m.accountId}:${m.id}`),
    ["1:1", "1:2", "2:2", "1:3"]
  );

  const reset = mergePagedMessages(merged, [makeMsg(1, 99)], true);
  assert.deepEqual(
    reset.map((m) => `${m.accountId}:${m.id}`),
    ["1:99"]
  );
});

test("Mock IPC 契约支持 10,000 封演示邮件分页、筛选与详情", async () => {
  const boot = await mainBootstrap();
  assert.equal(boot.mode, "demo");
  assert.equal(boot.locked, false);
  assert.equal(boot.accounts.length, 1);
  assert.equal(boot.folders[0].totalCount, 10000);

  const page1 = await listMessages({
    requestId: "req-1",
    view: { kind: "unifiedInbox" },
    query: "",
    scope: "thisFolder",
    sort: "dateNewest",
    filter: {},
    offset: 0,
    limit: 50,
  });
  assert.equal(page1.requestId, "req-1");
  assert.equal(page1.total, 10000);
  assert.equal(page1.hasMore, true);
  assert.equal(page1.items.length, 50);

  const pageLast = await listMessages({
    requestId: "req-last",
    view: { kind: "unifiedInbox" },
    query: "",
    scope: "thisFolder",
    sort: "dateNewest",
    filter: {},
    offset: 9980,
    limit: 50,
  });
  assert.equal(pageLast.items.length, 20);
  assert.equal(pageLast.hasMore, false);

  const searchRes = await listMessages({
    requestId: "req-search",
    view: { kind: "unifiedInbox" },
    query: "KiteMail",
    scope: "thisFolder",
    sort: "dateNewest",
    filter: { unread: true },
    offset: 0,
    limit: 50,
  });
  assert.equal(searchRes.items.length >= 1, true);

  const detail = await openMessage({
    requestId: "open-1",
    accountId: 1,
    messageId: 1,
  });
  assert.equal(detail.requestId, "open-1");
  assert.equal(detail.bodyStatus, "ready");
  assert.ok(detail.readerSessionId.length > 0);

  const layoutRes = await setReaderLayout({
    readerSessionId: detail.readerSessionId,
    layoutRevision: 1,
    contentRect: { x: 600, y: 80, width: 640, height: 700 },
    clipRect: { x: 600, y: 80, width: 640, height: 700 },
    visible: true,
    devicePixelRatio: 1.5,
  });
  assert.equal(layoutRes.applied, true);
  assert.equal(layoutRes.layoutRevision, 1);

  const updatedPrefs = await updateMainPreferences({ readerPosition: "below", themeId: "midnight" });
  assert.equal(updatedPrefs.readerPosition, "below");
  assert.equal(updatedPrefs.themeId, "midnight");
});

test("查询代次与正文中途切换防乱序守卫", () => {
  // 匹配当前代次、requestId 与 offset 时允许写入
  assert.equal(
    shouldApplyListResponse(2, 2, "list-2-50", 50, { requestId: "list-2-50", offset: 50 }),
    true
  );
  // 过期代次丢弃
  assert.equal(
    shouldApplyListResponse(1, 2, "list-1-0", 0, { requestId: "list-1-0", offset: 0 }),
    false
  );
  // 同代次错误偏移丢弃
  assert.equal(
    shouldApplyListResponse(2, 2, "list-2-50", 50, { requestId: "list-2-50", offset: 100 }),
    false
  );

  // A -> B -> A 快切：旧 A 请求不得覆盖新 A 请求；off 模式也不落地
  assert.equal(
    shouldApplyOpenResponse(
      3,
      3,
      "open-3-1-10",
      { accountId: 1, messageId: 10 },
      { requestId: "open-3-1-10", accountId: 1, messageId: 10 },
      "right"
    ),
    true
  );
  assert.equal(
    shouldApplyOpenResponse(
      1,
      3,
      "open-3-1-10",
      { accountId: 1, messageId: 10 },
      { requestId: "open-1-1-10", accountId: 1, messageId: 10 },
      "right"
    ),
    false
  );
  assert.equal(
    shouldApplyOpenResponse(
      3,
      3,
      "open-3-1-10",
      { accountId: 1, messageId: 10 },
      { requestId: "open-3-1-10", accountId: 1, messageId: 10 },
      "off"
    ),
    false
  );
});

test("extractErrorMessage 正确格式化 IpcError", () => {
  assert.equal(
    extractErrorMessage({ code: "LOCKED", message: "应用已锁定", retryable: false }),
    "[LOCKED] 应用已锁定"
  );
  assert.equal(extractErrorMessage(new Error("boom")), "boom");
  assert.equal(extractErrorMessage("plain error"), "plain error");
});

test("theme.css 包含全部 7 套内置主题的亮暗语义色与星标金", () => {
  const cssPath = path.resolve(process.cwd(), "src/styles/theme.css");
  const css = fs.readFileSync(cssPath, "utf8");
  assert.ok(css.includes("--km-star-gold: #e5a50a;"));
  assert.ok(css.includes("--km-accent-soft:"));
  for (const theme of ["system", "default", "midnight", "tidal", "rose", "earth", "forest"]) {
    assert.ok(css.includes(`:root[data-theme="${theme}"][data-color-mode="light"]`));
    assert.ok(css.includes(`:root[data-theme="${theme}"][data-color-mode="dark"]`));
  }
});
