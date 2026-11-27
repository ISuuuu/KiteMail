import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import net from "node:net";
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
  resolveDarkMode,
  usePaneLayout,
  validateReaderPosition,
} from "../src/main-window/usePaneLayout.ts";
import {
  computeAnchorScrollTop,
  dateGroupLabel,
  findMessageByRef,
  mergePagedMessages,
  shouldApplyListResponse,
  shouldApplyOpenResponse,
  useMainWindow,
} from "../src/main-window/useMainWindow.ts";
import {
  allowRemoteImages,
  archiveMessage,
  deleteMessage,
  emitMockEvent,
  extractErrorMessage,
  getMockRecordedReaderLayouts,
  isLockedError,
  listMessages,
  loadOlderMessages,
  mainBootstrap,
  markMessageSpam,
  moveMessage,
  normalizeSyncStatusEvent,
  openMessage,
  resetMockSession,
  setMessageFlagged,
  setMessageSeen,
  setReaderLayout,
  undoMessageMove,
  updateMainPreferences,
} from "../src/ipc/main-window.ts";
import type { MessageSummaryDto } from "../src/ipc/types.ts";
import {
  buildTauriDevConfig,
  buildTauriDevInvocation,
  findFreeLoopbackPort,
  isPortConflictOutput,
  parseDevTauriArgs,
} from "../scripts/dev-tauri.mjs";

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

test("锁屏清空全部敏感状态与阅读器，解锁校验与退避恢复视图", async () => {
  resetMockSession({ locked: false, hasPasscode: true, passcode: "pass-9527" });
  const win = useMainWindow();
  await win.init();

  assert.equal(win.locked.value, false);
  assert.equal(win.hasPasscode.value, true);
  assert.ok(win.accounts.value.length > 0);
  assert.ok(win.folders.value.length > 0);
  assert.ok(win.messages.value.length > 0);

  win.setSearch("KiteMail");
  await new Promise((r) => setTimeout(r, 10));
  win.selectMessage({ accountId: 1, messageId: 1 });
  await new Promise((r) => setTimeout(r, 10));
  assert.ok(win.openedMessage.value !== null);

  await win.lock();
  assert.equal(win.locked.value, true);
  assert.equal(win.accounts.value.length, 0);
  assert.equal(win.folders.value.length, 0);
  assert.equal(win.messages.value.length, 0);
  assert.equal(win.selectedId.value, null);
  assert.equal(win.openedMessage.value, null);
  assert.equal(win.searchQuery.value, "");
  assert.ok(
    getMockRecordedReaderLayouts().some((layout) => layout.visible === false),
    "锁屏时应发送 visible: false 隐藏原生阅读器"
  );

  // 连续输错 5 次触发退避倒计时
  for (let i = 0; i < 4; i++) {
    const ok = await win.unlock("wrong");
    assert.equal(ok, false);
    assert.equal(win.unlockRetryAfterSecs.value, 0);
  }
  const fifthOk = await win.unlock("wrong");
  assert.equal(fifthOk, false);
  assert.equal(win.unlockRetryAfterSecs.value, 5);
  assert.ok(win.unlockError.value?.includes("5 秒"));

  // 重置冷却后输入正确密码可解锁并重新 bootstrap 恢复视图
  resetMockSession({ locked: true, hasPasscode: true, passcode: "pass-9527" });
  win.unlockRetryAfterSecs.value = 0;
  const unlocked = await win.unlock("pass-9527");
  assert.equal(unlocked, true);
  assert.equal(win.locked.value, false);
  assert.ok(win.accounts.value.length > 0);
  assert.ok(win.folders.value.length > 0);
  assert.ok(win.messages.value.length > 0);

  win.dispose();
  resetMockSession();
});

test("LOCKED 结构化错误触发锁屏并清空已加载邮件状态", async () => {
  assert.equal(isLockedError({ code: "LOCKED", message: "已锁定", retryable: false }), true);
  assert.equal(isLockedError({ code: "INTERNAL_ERROR", message: "err", retryable: true }), false);
  assert.equal(isLockedError(new Error("LOCKED")), false);

  resetMockSession({ locked: false, hasPasscode: true });
  const win = useMainWindow();
  await win.init();
  assert.ok(win.messages.value.length > 0);

  // 模拟后端单独进入锁定状态，使后续 IPC 返回 code === "LOCKED"
  resetMockSession({ locked: true, hasPasscode: true });
  await assert.rejects(
    () =>
      listMessages({
        requestId: "locked-req",
        view: { kind: "unifiedInbox" },
        query: "",
        scope: "thisFolder",
        sort: "dateNewest",
        filter: {},
        offset: 0,
        limit: 50,
      }),
    (err: unknown) => isLockedError(err)
  );

  await win.toggleMessageSeen({ accountId: 1, messageId: 1 }, true);
  assert.equal(win.locked.value, true);
  assert.equal(win.accounts.value.length, 0);
  assert.equal(win.folders.value.length, 0);
  assert.equal(win.messages.value.length, 0);

  win.dispose();
  resetMockSession();
});

test("set_message_seen 与 set_message_flagged Mock 状态联动及未读统计更新", async () => {
  resetMockSession({ locked: false });
  const win = useMainWindow();
  await win.init();

  const target = findMessageByRef(win.messages.value, { accountId: 1, messageId: 1 });
  assert.ok(target);
  assert.equal(target.unread, true);
  assert.equal(target.flagged, true);

  const inboxBefore = win.folders.value.find((f) => f.id === 1)?.unreadCount ?? 0;

  win.selectMessage({ accountId: 1, messageId: 1 });
  await new Promise((r) => setTimeout(r, 10));
  assert.equal(win.openedMessage.value?.flagged, true);

  await win.toggleMessageSeen({ accountId: 1, messageId: 1 }, true);
  const afterSeen = findMessageByRef(win.messages.value, { accountId: 1, messageId: 1 });
  assert.equal(afterSeen?.unread, false);
  const inboxAfter = win.folders.value.find((f) => f.id === 1)?.unreadCount ?? 0;
  assert.equal(inboxAfter, Math.max(0, inboxBefore - 1));

  await win.toggleMessageFlagged({ accountId: 1, messageId: 1 }, false);
  const afterFlag = findMessageByRef(win.messages.value, { accountId: 1, messageId: 1 });
  assert.equal(afterFlag?.flagged, false);
  assert.equal(win.openedMessage.value?.flagged, false);

  // 直接调用底层 Mock IPC 验证状态版本号递增与返回值契约
  const seenOut = await setMessageSeen({ accountId: 1, messageId: 1, seen: false });
  assert.equal(seenOut.unread, true);
  const flagOut = await setMessageFlagged({ accountId: 1, messageId: 1, flagged: true });
  assert.equal(flagOut.flagged, true);

  win.dispose();
  resetMockSession();
});

test("main://mail-changed 保持选中邮件、不重置 scrollResetToken 并正确计算锚点偏移", async () => {
  const prevLayout = [
    { type: "header", key: "hdr:今天", top: 0, height: 28 },
    { type: "row", key: "1:1", top: 28, height: 90 },
    { type: "row", key: "1:2", top: 118, height: 90 },
    { type: "row", key: "1:3", top: 208, height: 90 },
  ];
  const nextLayout = [
    { type: "header", key: "hdr:今天", top: 0, height: 28 },
    { type: "row", key: "1:999", top: 28, height: 90 },
    { type: "row", key: "1:1", top: 118, height: 90 },
    { type: "row", key: "1:2", top: 208, height: 90 },
    { type: "row", key: "1:3", top: 298, height: 90 },
  ];
  // 原 scrollTop=150 停在 1:2 (top=118) 内部偏移 +32 处，上方新增 1 封邮件后应锚定到 208+32=240
  assert.equal(computeAnchorScrollTop(prevLayout, nextLayout, 150, "1:2"), 240);

  resetMockSession({ locked: false });
  const win = useMainWindow();
  await win.init();
  win.selectMessage({ accountId: 1, messageId: 2 });
  await new Promise((r) => setTimeout(r, 10));
  const tokenBefore = win.scrollResetToken.value;

  emitMockEvent("main://mail-changed", {
    stateVersion: win.stateVersion.value + 5,
    reason: "folderSynced",
  });
  await new Promise((r) => setTimeout(r, 20));

  assert.equal(win.scrollResetToken.value, tokenBefore);
  assert.deepEqual(win.selectedId.value, { accountId: 1, messageId: 2 });
  assert.equal(win.openedMessage.value?.messageId, 2);

  // SyncStatusEvent 兼容嵌套与扁平两种载荷
  const nested = normalizeSyncStatusEvent({
    stateVersion: 10,
    syncStatus: { syncing: true, message: "同步中", lastSyncAt: null },
  });
  assert.equal(nested.stateVersion, 10);
  assert.equal(nested.syncStatus.syncing, true);
  assert.equal(nested.syncStatus.message, "同步中");

  const flat = normalizeSyncStatusEvent({
    stateVersion: 11,
    syncing: false,
    message: null,
    lastSyncAt: 1700000000,
  });
  assert.equal(flat.stateVersion, 11);
  assert.equal(flat.syncStatus.lastSyncAt, 1700000000);

  win.dispose();
  resetMockSession();
});

test("dev-tauri 多 worktree 参数解析、配置构造与空闲回环端口探测", async () => {
  const parsed = parseDevTauriArgs(["--demo", "--port", "1455", "--verbose", "--", "--extra"], {
    KITEMAIL_DEMO: "0",
  });
  assert.equal(parsed.demo, true);
  assert.equal(parsed.preferredPort, 1455);
  assert.deepEqual(parsed.extraTauriArgs, ["--verbose"]);
  assert.deepEqual(parsed.extraAppArgs, ["--extra"]);

  const parsedFromEnv = parseDevTauriArgs([], { KITEMAIL_DEMO: "1", KITEMAIL_DEV_PORT: "1520" });
  assert.equal(parsedFromEnv.demo, true);
  assert.equal(parsedFromEnv.preferredPort, 1520);

  assert.equal(
    buildTauriDevConfig(1455),
    JSON.stringify({ build: { devUrl: "http://127.0.0.1:1455" } })
  );

  const invocation = buildTauriDevInvocation({
    port: 1455,
    demo: true,
    extraTauriArgs: ["--no-watch"],
  });
  assert.equal(invocation.envPatch.KITEMAIL_DEV_PORT, "1455");
  assert.equal(invocation.envPatch.KITEMAIL_DEMO, "1");
  // 双 `--`：第一个后是 runner（cargo）参数（留空），第二个后才是应用参数
  assert.deepEqual(invocation.args, [
    "tauri",
    "dev",
    "--config",
    '{"build":{"devUrl":"http://127.0.0.1:1455"}}',
    "--no-watch",
    "--",
    "--",
    "--demo",
  ]);

  assert.equal(isPortConflictOutput("Error: Port 1420 is already in use"), true);
  assert.equal(isPortConflictOutput("listen EADDRINUSE: address already in use 127.0.0.1:1420"), true);
  assert.equal(isPortConflictOutput("VITE v6.4.6 ready in 120 ms"), false);

  // 占用一个回环端口，验证 findFreeLoopbackPort 自动跳过并选择新空闲端口
  const busyServer = net.createServer();
  await new Promise<void>((resolve) => busyServer.listen(0, "127.0.0.1", () => resolve()));
  const busyAddr = busyServer.address();
  assert.ok(busyAddr && typeof busyAddr === "object");
  try {
    const freePort = await findFreeLoopbackPort({ preferredPort: busyAddr.port });
    assert.ok(Number.isInteger(freePort) && freePort > 0 && freePort < 65536);
    assert.notEqual(freePort, busyAddr.port);
  } finally {
    await new Promise<void>((resolve) => busyServer.close(() => resolve()));
  }
});

test("三态 darkMode (null/false/true) 保留与系统 prefers-color-scheme 联动", async () => {
  resetMockSession({ locked: false, darkMode: null });

  const listeners = new Set<() => void>();
  let systemDark = true;
  const originalWindow = (globalThis as Record<string, unknown>).window;
  (globalThis as Record<string, unknown>).window = {
    innerWidth: 1280,
    innerHeight: 800,
    addEventListener: () => {},
    removeEventListener: () => {},
    matchMedia: (query: string) => ({
      media: query,
      get matches() {
        return systemDark;
      },
      addEventListener: (_type: string, cb: () => void) => listeners.add(cb),
      removeEventListener: (_type: string, cb: () => void) => listeners.delete(cb),
    }),
  };

  try {
    assert.equal(resolveDarkMode(false), false);
    assert.equal(resolveDarkMode(true), true);
    assert.equal(resolveDarkMode(null), true);

    const layout = usePaneLayout();
    const boot = await mainBootstrap();
    assert.equal(boot.preferences.darkMode, null);

    // 非 system 主题下 darkMode === null 仍应遵循系统暗色，且不覆写 prefs.darkMode 为布尔值
    layout.applyPrefs({ ...boot.preferences, themeId: "midnight", darkMode: null });
    assert.equal(layout.prefs.value.darkMode, null);
    assert.equal(layout.followSystemScheme.value, true);
    assert.equal(layout.effectiveDark.value, true);

    // 系统切换为浅色时实时联动，且 prefs.darkMode 依然保持 null
    systemDark = false;
    for (const cb of listeners) cb();
    assert.equal(layout.effectiveDark.value, false);
    assert.equal(layout.prefs.value.darkMode, null);

    // 循环切换三态：null -> false -> true -> null，并持久化到 Mock 偏好
    layout.toggleDarkMode();
    assert.equal(layout.prefs.value.darkMode, false);
    assert.equal(layout.effectiveDark.value, false);
    assert.equal(layout.followSystemScheme.value, false);
    await new Promise((r) => setTimeout(r, 10));
    assert.equal((await mainBootstrap()).preferences.darkMode, false);

    layout.toggleDarkMode();
    assert.equal(layout.prefs.value.darkMode, true);
    assert.equal(layout.effectiveDark.value, true);
    await new Promise((r) => setTimeout(r, 10));
    assert.equal((await mainBootstrap()).preferences.darkMode, true);

    layout.toggleDarkMode();
    assert.equal(layout.prefs.value.darkMode, null);
    assert.equal(layout.followSystemScheme.value, true);
    await new Promise((r) => setTimeout(r, 10));
    assert.equal((await mainBootstrap()).preferences.darkMode, null);
  } finally {
    if (originalWindow === undefined) {
      delete (globalThis as Record<string, unknown>).window;
    } else {
      (globalThis as Record<string, unknown>).window = originalWindow;
    }
    resetMockSession();
  }
});

test("allow_remote_images 放行单封与信任发件人并清除拦截提示", async () => {
  resetMockSession({ locked: false });
  const win = useMainWindow();
  await win.init();

  // 消息 #2 默认拦截远程图片
  win.selectMessage({ accountId: 1, messageId: 2 });
  await new Promise((r) => setTimeout(r, 15));
  assert.equal(win.openedMessage.value?.messageId, 2);
  assert.equal(win.openedMessage.value?.remoteImagesBlocked, true);
  assert.equal(win.openedMessage.value?.folderPath, "INBOX");
  assert.ok(win.openedMessage.value?.rfcMessageId);

  // 仅本次显示：当前消息解除拦截
  await win.allowOpenedRemoteImages(false);
  assert.equal(win.openedMessage.value?.remoteImagesBlocked, false);

  // 消息 #7（7 % 5 === 2）验证 trustSender=true 会放行同一发件人的其他邮件
  const resTrust = await allowRemoteImages({ accountId: 1, messageId: 7, trustSender: true });
  assert.equal(resTrust.allowed, true);
  assert.equal(resTrust.senderTrusted, true);
  const detail7 = await openMessage({ requestId: "open-7", accountId: 1, messageId: 7 });
  assert.equal(detail7.remoteImagesBlocked, false);

  // 第 2 封（同组）与第 127 封同为 sender3@example.com（(126 % 120)+1 = 7，(6 % 120)+1 = 7）
  const detail127 = await openMessage({ requestId: "open-127", accountId: 1, messageId: 127 });
  assert.equal(detail127.remoteImagesBlocked, false);

  win.dispose();
  resetMockSession();
});

test("load_older_messages 拉取更早邮件并更新 hasMoreRemote 状态", async () => {
  resetMockSession({ locked: false, inboxHasMoreRemote: true });
  const win = useMainWindow();
  await win.init();

  assert.equal(win.currentFolderHasMoreRemote.value, true);
  const beforeTotal = win.folders.value.find((f) => f.id === 1)?.totalCount ?? 0;
  assert.equal(beforeTotal, 10000);

  await win.loadOlderMessages(15);
  await new Promise((r) => setTimeout(r, 15));

  const afterTotal = win.folders.value.find((f) => f.id === 1)?.totalCount ?? 0;
  assert.equal(afterTotal, 10015);
  assert.equal(win.currentFolderHasMoreRemote.value, false);
  assert.equal(win.listTotal.value, 10015);

  // 再次调用返回 accepted: false
  const second = await loadOlderMessages({ view: { kind: "unifiedInbox" }, limit: 15 });
  assert.equal(second.accepted, false);
  assert.equal(second.hasMoreRemote, false);

  win.dispose();
  resetMockSession();
});

test("归档/删除/标垃圾/移动自动选中下一封并支持撤销状态回滚", async () => {
  resetMockSession({ locked: false });
  const win = useMainWindow();
  await win.init();

  const initialInboxCount = win.folders.value.find((f) => f.id === 1)?.totalCount ?? 0;
  const initialArchiveCount = win.folders.value.find((f) => f.id === 4)?.totalCount ?? 0;

  // 选中第 1 封并归档：自动平滑选中第 2 封并展示撤销 Toast
  win.selectMessage({ accountId: 1, messageId: 1 });
  await new Promise((r) => setTimeout(r, 15));
  await win.archiveSelectedMessage();
  await new Promise((r) => setTimeout(r, 15));

  assert.deepEqual(win.selectedId.value, { accountId: 1, messageId: 2 });
  assert.equal(win.undoToast.value?.originalFolderPath, "INBOX");
  assert.equal(win.undoToast.value?.currentFolderPath, "Archive");
  assert.equal(win.folders.value.find((f) => f.id === 1)?.totalCount, initialInboxCount - 1);
  assert.equal(win.folders.value.find((f) => f.id === 4)?.totalCount, initialArchiveCount + 1);
  assert.equal(findMessageByRef(win.messages.value, { accountId: 1, messageId: 1 }), null);

  // 点击撤销：触发 undo_message_move 并恢复文件夹计数与列表
  const undone = await win.undoLastMove();
  await new Promise((r) => setTimeout(r, 15));
  assert.equal(undone, true);
  assert.equal(win.undoToast.value, null);
  assert.equal(win.folders.value.find((f) => f.id === 1)?.totalCount, initialInboxCount);
  assert.equal(win.folders.value.find((f) => f.id === 4)?.totalCount, initialArchiveCount);
  assert.ok(findMessageByRef(win.messages.value, { accountId: 1, messageId: 1 }));

  // 删除、标垃圾与移动到指定文件夹 + 撤销验证
  win.selectMessage({ accountId: 1, messageId: 2 });
  await new Promise((r) => setTimeout(r, 15));
  await win.deleteSelectedMessage();
  await new Promise((r) => setTimeout(r, 15));
  assert.deepEqual(win.selectedId.value, { accountId: 1, messageId: 3 });
  assert.equal(win.undoToast.value?.currentFolderPath, "Trash");
  await win.undoLastMove();
  await new Promise((r) => setTimeout(r, 15));
  assert.ok(findMessageByRef(win.messages.value, { accountId: 1, messageId: 2 }));

  const spamOut = await markMessageSpam({ accountId: 1, messageId: 3 });
  assert.equal(spamOut.targetFolderPath, "Junk");
  await undoMessageMove({
    accountId: spamOut.accountId,
    messageId: spamOut.messageId,
    rfcMessageId: spamOut.rfcMessageId,
    currentFolderPath: spamOut.targetFolderPath,
    originalFolderPath: spamOut.originalFolderPath,
  });

  const moveOut = await moveMessage({ accountId: 1, messageId: 4, targetFolderPath: "Sent" });
  assert.equal(moveOut.targetFolderPath, "Sent");
  await undoMessageMove({
    accountId: moveOut.accountId,
    messageId: moveOut.messageId,
    rfcMessageId: moveOut.rfcMessageId,
    currentFolderPath: moveOut.targetFolderPath,
    originalFolderPath: moveOut.originalFolderPath,
  });

  // 锁屏状态下 Wave 1 新命令均返回 LOCKED
  resetMockSession({ locked: true });
  await assert.rejects(() => archiveMessage({ accountId: 1, messageId: 1 }), isLockedError);
  await assert.rejects(() => deleteMessage({ accountId: 1, messageId: 1 }), isLockedError);
  await assert.rejects(() => markMessageSpam({ accountId: 1, messageId: 1 }), isLockedError);
  await assert.rejects(
    () => moveMessage({ accountId: 1, messageId: 1, targetFolderPath: "Archive" }),
    isLockedError
  );
  await assert.rejects(
    () =>
      undoMessageMove({
        accountId: 1,
        messageId: 1,
        rfcMessageId: "<mock-1-1@kitemail.example>",
        currentFolderPath: "Archive",
        originalFolderPath: "INBOX",
      }),
    isLockedError
  );
  await assert.rejects(
    () => allowRemoteImages({ accountId: 1, messageId: 2, trustSender: false }),
    isLockedError
  );
  await assert.rejects(
    () => loadOlderMessages({ view: { kind: "unifiedInbox" }, limit: 20 }),
    isLockedError
  );

  win.dispose();
  resetMockSession();
});
