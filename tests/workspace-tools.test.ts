import test from "node:test";
import assert from "node:assert/strict";

import {
  __resetContactsMock,
  deleteContact,
  exportContacts,
  importContacts,
  isLockedError,
  listContacts,
  saveContact,
} from "../src/contacts/ipc.ts";
import {
  buildSaveContactInput,
  createEmptyContactForm,
  useContacts,
  validateContactForm,
} from "../src/contacts/useContacts.ts";

import {
  __resetPreferencesMock,
  checkAppUpdate,
  clearBodyCaches,
  getFullPreferences,
  manageTrustedSenders,
  setAppLockPasscode,
  updateFullPreferences,
} from "../src/preferences/ipc.ts";
import {
  formatBytes,
  usePreferences,
  validatePasscodeChange,
} from "../src/preferences/usePreferences.ts";

import type { CommandPaletteItem } from "../src/command-palette/types.ts";
import {
  filterCommandItems,
  matchSingleKeyShortcut,
  useCommandPalette,
} from "../src/command-palette/useCommandPalette.ts";

// ==================== 1. 通讯录管理与导入导出 ====================

test("通讯录支持检索（含自动聚合联系人）、新建、编辑升级、删除与表单校验", async () => {
  __resetContactsMock();

  // 初始列表同时包含个人联系人与自动聚合联系人
  const initial = await listContacts({ limit: 20 });
  assert.ok(initial.total >= 4);
  assert.ok(initial.items.some((c) => c.isUserContact));
  assert.ok(initial.items.some((c) => !c.isUserContact && c.id === null));

  // 搜索自动聚合联系人
  const botSearch = await listContacts({ query: "Build Bot" });
  assert.equal(botSearch.total, 1);
  assert.equal(botSearch.items[0].email, "ci@kitemail.example");
  assert.equal(botSearch.items[0].isUserContact, false);

  // 表单校验拦截非法姓名与邮箱
  const invalidForm = createEmptyContactForm();
  invalidForm.name = "  ";
  invalidForm.email = "bad-email";
  const check = validateContactForm(invalidForm);
  assert.equal(check.valid, false);
  assert.ok(check.errors.name);
  assert.ok(check.errors.email);

  // 合法表单保存新联系人
  const validForm = createEmptyContactForm();
  validForm.name = "赵敏";
  validForm.email = "zhaomin@example.com";
  validForm.phone = "13800138000";
  validForm.organization = "研发中心";
  validForm.jobTitle = "架构师";
  validForm.groupsText = "工作, 核心";
  validForm.notes = "重点联系人";
  assert.equal(validateContactForm(validForm).valid, true);

  const createdRes = await saveContact(buildSaveContactInput(validForm));
  assert.ok(createdRes.contact.id !== null);
  assert.equal(createdRes.contact.isUserContact, true);
  assert.deepEqual(createdRes.contact.groups, ["工作", "核心"]);

  // 升级自动聚合联系人为个人联系人（保留历史往来计数）
  const promoted = await saveContact({
    id: null,
    name: "CI 机器人",
    email: "ci@kitemail.example",
    notes: "自动构建通知",
  });
  assert.equal(promoted.contact.isUserContact, true);
  assert.ok(promoted.contact.id !== null);
  assert.equal(promoted.contact.messageCount, 45);

  // 删除新建的个人联系人
  const delRes = await deleteContact({ contactId: createdRes.contact.id! });
  assert.equal(delRes.deleted, true);

  const afterDel = await listContacts({ query: "zhaomin@example.com" });
  assert.equal(afterDel.total, 0);

  __resetContactsMock();
});

test("通讯录 vCard 与 CSV 自动识别导入、去重跳过以及导出格式正确", async () => {
  __resetContactsMock({ contacts: [] });

  // vCard 含折行与转义字符
  const vcfPayload = [
    "BEGIN:VCARD",
    "VERSION:3.0",
    "FN:王五",
    "EMAIL;TYPE=INTERNET:wangwu@example.com",
    "TEL;TYPE=CELL:+86 135-1111-2222",
    "ORG:KiteMail\\, Inc.",
    "NOTE:第一行",
    " 续行内容",
    "CATEGORIES:项目,VIP",
    "END:VCARD",
    "BEGIN:VCARD",
    "VERSION:3.0",
    "FN:重复邮箱",
    "EMAIL:WANGWU@EXAMPLE.COM",
    "END:VCARD",
    "BEGIN:VCARD",
    "VERSION:3.0",
    "FN:无效邮箱",
    "EMAIL:not-an-email",
    "END:VCARD",
  ].join("\r\n");

  const vcfImport = await importContacts({ format: "auto", content: vcfPayload });
  assert.equal(vcfImport.totalParsed, 3);
  assert.equal(vcfImport.added, 1);
  assert.equal(vcfImport.skipped, 2);

  // CSV 含引号内逗号与表头识别
  const csvPayload = [
    "Name,Email,Phone,Organization,JobTitle,Groups,Notes",
    '"李四, Jr.",lisi@example.org,13600001111,"星云, 科技",PM,"产品;VIP","备注, 含逗号"',
    "王五重复,wangwu@example.com,,,,,",
  ].join("\n");

  const csvImport = await importContacts({ format: "auto", content: csvPayload });
  assert.equal(csvImport.totalParsed, 2);
  assert.equal(csvImport.added, 1);
  assert.equal(csvImport.skipped, 1);

  // 导出 vCard 与 CSV
  const exportedVcf = await exportContacts({ format: "vcard" });
  assert.equal(exportedVcf.format, "vcard");
  assert.equal(exportedVcf.filename, "kitemail-contacts.vcf");
  assert.equal(exportedVcf.count, 2);
  assert.ok(exportedVcf.content.includes("BEGIN:VCARD"));
  assert.ok(exportedVcf.content.includes("wangwu@example.com"));

  const exportedCsv = await exportContacts({ format: "csv" });
  assert.equal(exportedCsv.format, "csv");
  assert.equal(exportedCsv.filename, "kitemail-contacts.csv");
  assert.equal(exportedCsv.count, 2);
  assert.ok(exportedCsv.content.includes('"李四, Jr."'));
  assert.ok(exportedCsv.content.includes("lisi@example.org"));

  __resetContactsMock();
});

test("通讯录在 LOCKED 状态下抛出结构化 LOCKED 错误并触发 useContacts onLocked", async () => {
  __resetContactsMock({ locked: true });
  await assert.rejects(() => listContacts(), (err: unknown) => isLockedError(err));

  let lockedCount = 0;
  const mgr = useContacts({
    onLocked: () => {
      lockedCount += 1;
    },
  });
  await mgr.init();
  assert.equal(lockedCount, 1);
  assert.ok(mgr.loadError.value);

  mgr.dispose();
  __resetContactsMock();
});

// ==================== 2. 偏好设置、安全锁、白名单、缓存与更新 ====================

test("偏好设置加载与局部更新持久化，且触发 preferencesUpdated 回调", async () => {
  __resetPreferencesMock();
  let updatedCount = 0;
  const mgr = usePreferences({
    onPreferencesUpdated: () => {
      updatedCount += 1;
    },
  });
  await mgr.init();

  assert.equal(mgr.preferences.value.themeId, "system");
  assert.equal(mgr.hasPasscode.value, false);
  assert.ok(mgr.dataDir.value.length > 0);
  assert.ok(mgr.mailDataDir.value.length > 0);

  const updated = await mgr.updatePreference({
    themeId: "midnight",
    darkMode: true,
    singleKeyShortcuts: true,
    undoSendSecs: 10,
    readMarkMode: "delay",
  });
  assert.ok(updated);
  assert.equal(updated.themeId, "midnight");
  assert.equal(updated.darkMode, true);
  assert.equal(updated.singleKeyShortcuts, true);
  assert.equal(updated.undoSendSecs, 10);
  assert.equal(updated.readMarkMode, "delay");
  assert.equal(updatedCount, 1);

  const fetched = await getFullPreferences();
  assert.equal(fetched.preferences.themeId, "midnight");
  assert.equal(fetched.preferences.singleKeyShortcuts, true);

  mgr.dispose();
  __resetPreferencesMock();
});

test("安全锁口令设置、校验失败与清除，且绝不在报错或状态中暴露口令明文", async () => {
  __resetPreferencesMock({ hasPasscode: false, passcode: null });

  let passcodeEventCount = 0;
  const mgr = usePreferences({
    onPasscodeChanged: () => {
      passcodeEventCount += 1;
    },
  });
  await mgr.init();

  // 本地校验：口令不一致或过短
  const mismatchMsg = validatePasscodeChange({
    hasPasscode: false,
    currentPasscode: "",
    newPasscode: "secret-pass-1",
    confirmPasscode: "secret-pass-2",
    clearing: false,
  });
  assert.ok(mismatchMsg);
  assert.equal(mismatchMsg.includes("secret-pass"), false);

  // 设置初始口令
  const setOk = await mgr.submitPasscode({
    newPasscode: "kitemail-9527",
    confirmPasscode: "kitemail-9527",
    enableOnStartup: true,
    idleMinutes: 10,
  });
  assert.equal(setOk, true);
  assert.equal(mgr.hasPasscode.value, true);
  assert.equal(mgr.preferences.value.appLockEnabled, true);
  assert.equal(mgr.preferences.value.appLockOnStartup, true);
  assert.equal(mgr.newPasscodeInput.value, "");
  assert.equal(mgr.confirmPasscodeInput.value, "");
  assert.equal(passcodeEventCount, 1);

  // 错误旧口令被拒绝且绝不回显错误输入或原口令明文
  const badAttempt = await mgr.submitPasscode({
    currentPasscode: "wrong-plain-9999",
    newPasscode: "new-plain-8888",
    confirmPasscode: "new-plain-8888",
  });
  assert.equal(badAttempt, false);
  assert.ok(mgr.passcodeError.value);
  assert.equal(mgr.passcodeError.value.includes("wrong-plain-9999"), false);
  assert.equal(mgr.passcodeError.value.includes("kitemail-9527"), false);
  assert.equal(mgr.passcodeError.value.includes("new-plain-8888"), false);
  assert.equal(mgr.currentPasscodeInput.value, "");

  // 正确旧口令清除安全锁
  const cleared = await mgr.clearPasscode("kitemail-9527");
  assert.equal(cleared, true);
  assert.equal(mgr.hasPasscode.value, false);
  assert.equal(mgr.preferences.value.appLockEnabled, false);
  assert.equal(mgr.preferences.value.appLockOnStartup, false);
  assert.equal(passcodeEventCount, 2);

  mgr.dispose();
  __resetPreferencesMock();
});

test("远图白名单增删清空、正文缓存清理、字节格式化与检查更新", async () => {
  __resetPreferencesMock({
    allowedRemoteSenders: ["existing@example.com"],
    appVersion: "0.2.0",
    isPortable: true,
    updateResponder: () => ({
      checked: true,
      updateAvailable: true,
      currentVersion: "0.2.0",
      target: "windows-x86_64",
      latestVersion: "0.3.0",
      notes: "新增命令面板与通讯录管理",
      pubDate: "2026-11-27",
      downloadUrl: "https://example.com/kitemail-0.3.0.exe",
      message: "发现新版本 v0.3.0",
    }),
  });

  const mgr = usePreferences();
  await mgr.init();
  assert.equal(mgr.isPortable.value, true);
  assert.equal(mgr.appVersion.value, "0.2.0");

  // 远图白名单添加、去重、移除与清空
  await mgr.addTrustedSender("TrustMe@Example.ORG");
  assert.deepEqual(mgr.allowedRemoteSenders.value, [
    "existing@example.com",
    "trustme@example.org",
  ]);
  await mgr.removeTrustedSender("existing@example.com");
  assert.deepEqual(mgr.allowedRemoteSenders.value, ["trustme@example.org"]);
  await mgr.clearTrustedSenders();
  assert.deepEqual(mgr.allowedRemoteSenders.value, []);

  // 清理缓存统计变化
  const beforeBytes = mgr.storageStats.value.mailBytes;
  assert.ok(beforeBytes > 0);
  assert.equal(formatBytes(0), "0 B");
  assert.equal(formatBytes(1536), "1.5 KB");

  const cleared = await mgr.clearCaches(null);
  assert.equal(cleared, true);
  assert.equal(mgr.storageStats.value.mailBytes, 0);

  // 检查更新
  const update = await mgr.checkForUpdates();
  assert.ok(update);
  assert.equal(update.updateAvailable, true);
  assert.equal(update.latestVersion, "0.3.0");

  // 直接验证 IPC 函数在 LOCKED 下拒绝
  __resetPreferencesMock({ locked: true });
  await assert.rejects(() => updateFullPreferences({ compactList: true }), isLockedError);
  await assert.rejects(
    () => setAppLockPasscode({ newPasscode: "1234" }),
    isLockedError
  );
  await assert.rejects(
    () => manageTrustedSenders({ action: "list" }),
    isLockedError
  );
  await assert.rejects(() => clearBodyCaches(), isLockedError);
  await assert.rejects(() => checkAppUpdate(), isLockedError);

  mgr.dispose();
  __resetPreferencesMock();
});

// ==================== 3. 命令面板与单键快捷键系统 ====================

test("filterCommandItems 支持模糊检索、多关键词与分类排序", () => {
  const items: CommandPaletteItem[] = [
    {
      id: "compose.new",
      title: "写新邮件",
      category: "写信",
      shortcut: "c",
      keywords: ["compose", "new", "mail"],
    },
    {
      id: "message.archive",
      title: "归档选中邮件",
      category: "邮件操作",
      shortcut: "e",
      keywords: ["archive"],
    },
    {
      id: "contacts.open",
      title: "打开通讯录管理",
      category: "工具",
      keywords: ["contacts", "vcard", "csv"],
    },
    {
      id: "preferences.open",
      title: "打开偏好设置",
      category: "设置",
      keywords: ["settings", "preferences", "theme"],
    },
  ];

  assert.equal(filterCommandItems(items, "").length, 4);
  assert.equal(filterCommandItems(items, "归档")[0].id, "message.archive");
  assert.equal(filterCommandItems(items, "vcard")[0].id, "contacts.open");
  // 子序列模糊检索：cmps -> compose
  assert.equal(filterCommandItems(items, "cmps")[0].id, "compose.new");
  // 多词联合匹配
  assert.equal(filterCommandItems(items, "设置 pref")[0].id, "preferences.open");
  assert.equal(filterCommandItems(items, "不存在的命令xyz").length, 0);

  const palette = useCommandPalette(() => items);
  palette.setQuery("");
  assert.equal(palette.activeIndex.value, 0);
  palette.moveSelection(1);
  assert.equal(palette.getActiveItem()?.id, "message.archive");
  palette.moveSelection(-2);
  assert.equal(palette.getActiveItem()?.id, "preferences.open");
});

test("matchSingleKeyShortcut 正确映射 Ctrl+K 与单键，并严格在 IME / 输入框 / 弹窗 / 关闭开关时拦截放行", () => {
  const activeCtx = {
    singleKeyShortcuts: true,
    hasOpenModal: false,
    isInputFocused: false,
  };

  // Ctrl+K 打开命令面板
  const ctrlK = matchSingleKeyShortcut(
    { key: "k", ctrlKey: true },
    { singleKeyShortcuts: false, hasOpenModal: false, isInputFocused: true }
  );
  assert.equal(ctrlK.matched, true);
  assert.equal(ctrlK.commandId, "command-palette.toggle");
  assert.equal(ctrlK.isPaletteToggle, true);

  // 全量单键快捷键映射验证
  const expectedKeys: Array<[string, string, boolean?]> = [
    ["c", "compose.new"],
    ["r", "message.reply"],
    ["a", "message.replyAll"],
    ["f", "message.forward"],
    ["e", "message.archive"],
    ["s", "message.toggleStar"],
    ["u", "message.toggleRead"],
    ["j", "message.next"],
    ["k", "message.prev"],
    ["?", "help.shortcuts", true],
    ["#", "message.delete", true],
    ["/", "search.focus"],
  ];

  for (const [key, expectedCmd, shiftKey] of expectedKeys) {
    const res = matchSingleKeyShortcut(
      { key, shiftKey: Boolean(shiftKey) },
      activeCtx
    );
    assert.equal(res.matched, true, `key=${key} should match`);
    assert.equal(res.commandId, expectedCmd);
    assert.equal(res.preventDefault, true);
  }

  // Shift+/ 与 Shift+3 兼容映射
  assert.equal(
    matchSingleKeyShortcut({ key: "/", shiftKey: true }, activeCtx).commandId,
    "help.shortcuts"
  );
  assert.equal(
    matchSingleKeyShortcut({ key: "3", shiftKey: true }, activeCtx).commandId,
    "message.delete"
  );

  // 1. IME 组合输入期间严格放行（含 Ctrl+K 与单键）
  assert.equal(
    matchSingleKeyShortcut({ key: "c", isComposing: true }, activeCtx).matched,
    false
  );
  assert.equal(
    matchSingleKeyShortcut({ key: "Process" }, activeCtx).matched,
    false
  );
  assert.equal(
    matchSingleKeyShortcut({ key: "k", ctrlKey: true, keyCode: 229 }, activeCtx)
      .matched,
    false
  );

  // 2. INPUT / TEXTAREA / SELECT / contentEditable 聚焦时拦截单键快捷键
  for (const target of [
    { tagName: "INPUT" },
    { tagName: "TEXTAREA" },
    { tagName: "SELECT" },
    { tagName: "DIV", isContentEditable: true },
    { tagName: "DIV", getAttribute: (n: string) => (n === "contenteditable" ? "true" : null) },
  ]) {
    assert.equal(
      matchSingleKeyShortcut({ key: "c", target }, activeCtx).matched,
      false
    );
  }
  assert.equal(
    matchSingleKeyShortcut(
      { key: "j" },
      { ...activeCtx, isInputFocused: true }
    ).matched,
    false
  );

  // 3. 模态弹窗打开时拦截单键快捷键
  assert.equal(
    matchSingleKeyShortcut(
      { key: "e" },
      { ...activeCtx, hasOpenModal: true }
    ).matched,
    false
  );

  // 4. 关闭 singleKeyShortcuts 开关时拦截单键快捷键
  assert.equal(
    matchSingleKeyShortcut(
      { key: "r" },
      { ...activeCtx, singleKeyShortcuts: false }
    ).matched,
    false
  );

  // 5. 带有 Ctrl/Alt/Meta 修饰键的按键不触发单键命令
  assert.equal(
    matchSingleKeyShortcut({ key: "c", ctrlKey: true }, activeCtx).matched,
    false
  );
  assert.equal(
    matchSingleKeyShortcut({ key: "e", altKey: true }, activeCtx).matched,
    false
  );
});
