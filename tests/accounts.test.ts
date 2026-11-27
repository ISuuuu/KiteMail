import test from "node:test";
import assert from "node:assert/strict";

import {
  BUILTIN_PROVIDER_PRESETS,
  emitMockAccountEvent,
  listAccounts,
  resetMockAccounts,
} from "../src/accounts/ipc.ts";
import {
  ACCOUNT_ACCENT_COLORS,
  accountAccentHex,
  brandForAccountFields,
  senderInitials,
} from "../src/accounts/brand.ts";
import { buildSignaturePreviewHtml } from "../src/accounts/signature.ts";
import { useAccountLayers } from "../src/accounts/useAccountLayers.ts";
import {
  buildSaveAccountInput,
  createEmptyAccountForm,
  extractEmailDomain,
  matchPresetByEmail,
  useAccounts,
  validateAccountForm,
} from "../src/accounts/useAccounts.ts";
import {
  addToBlocklist,
  buildFilterRuleSummary,
  filterBlocklist,
  filterFieldLabel,
  generateLocalFingerprint,
  normalizeBlockedSender,
  normalizeTagColor,
  normalizeTagShortcut,
  pgpKeySubtitle,
  pgpKeyTitle,
  registerPgpKey,
  removeFromBlocklist,
  saveTag,
  smartFolderSubtitle,
  tagShortcutLabel,
  type FilterRuleItem,
} from "../src/settings/extraTabsLogic.ts";

test("extractEmailDomain 与 matchPresetByEmail 正确识别主流邮箱服务商", () => {
  assert.equal(extractEmailDomain("Alice <Alice@QQ.COM>"), "qq.com");
  assert.equal(extractEmailDomain("user@foxmail.com"), "foxmail.com");
  assert.equal(extractEmailDomain("invalid-email"), "");

  const qq = matchPresetByEmail("10086@qq.com", BUILTIN_PROVIDER_PRESETS);
  assert.equal(qq.matched, true);
  assert.equal(qq.preset?.id, "qq");
  assert.equal(qq.preset?.imapHost, "imap.qq.com");
  assert.equal(qq.preset?.smtpHost, "smtp.qq.com");

  const netease = matchPresetByEmail("dev@163.com", BUILTIN_PROVIDER_PRESETS);
  assert.equal(netease.matched, true);
  assert.equal(netease.preset?.id, "netease163");

  const gmail = matchPresetByEmail("dev@gmail.com", BUILTIN_PROVIDER_PRESETS);
  assert.equal(gmail.matched, true);
  assert.equal(gmail.preset?.id, "gmail");

  const outlook = matchPresetByEmail("user@outlook.com", BUILTIN_PROVIDER_PRESETS);
  assert.equal(outlook.matched, true);
  assert.equal(outlook.preset?.id, "outlook");
  assert.equal(outlook.preset?.authBackend, "microsoftGraphOAuth2");

  const icloud = matchPresetByEmail("me@icloud.com", BUILTIN_PROVIDER_PRESETS);
  assert.equal(icloud.matched, true);
  assert.equal(icloud.preset?.id, "icloud");
  assert.equal(icloud.preset?.smtpPort, 587);
  assert.equal(icloud.preset?.smtpSecurity, "startTls");

  const unknown = matchPresetByEmail("staff@custom-corp.io", BUILTIN_PROVIDER_PRESETS);
  assert.equal(unknown.matched, false);
  assert.equal(unknown.domain, "custom-corp.io");
  assert.equal(unknown.preset, null);
});

test("邮箱自动匹配预设且不覆写用户已手动修改的自定义字段，支持显式重置", async () => {
  resetMockAccounts({ accounts: [] });
  const mgr = useAccounts();
  await mgr.init();

  mgr.startCreate();
  mgr.updateEmail("user@qq.com");
  assert.equal(mgr.form.value.presetId, "qq");
  assert.equal(mgr.form.value.imapHost, "imap.qq.com");
  assert.equal(mgr.form.value.smtpHost, "smtp.qq.com");
  assert.equal(mgr.form.value.imapUsername, "user@qq.com");

  // 手动修改 imapHost 后锁定该字段
  mgr.form.value.imapHost = "custom-imap.internal";
  mgr.markFieldEdited("imapHost");

  // 更换邮箱域名为 163，未锁定的 smtpHost 更新为 smtp.163.com，而锁定的 imapHost 保持不变
  mgr.updateEmail("user@163.com");
  assert.equal(mgr.form.value.presetId, "netease163");
  assert.equal(mgr.form.value.imapHost, "custom-imap.internal");
  assert.equal(mgr.form.value.smtpHost, "smtp.163.com");

  // 显式重置后恢复为 163 默认主机
  mgr.resetToPreset();
  assert.equal(mgr.form.value.imapHost, "imap.163.com");
  assert.equal(mgr.hasLockedPresetFields.value, false);

  mgr.dispose();
  resetMockAccounts();
});

test("validateAccountForm 校验邮箱、端口范围、编辑态保留密码与独立 SMTP 凭据", () => {
  const form = createEmptyAccountForm(
    BUILTIN_PROVIDER_PRESETS.find((p) => p.id === "qq")
  );
  form.displayName = "测试用户";
  form.email = "bad-email";
  form.imapUsername = "bad-email";
  form.imapPassword = "";
  form.imapPort = 70000;
  form.smtpPort = 0;

  const invalidRes = validateAccountForm(form);
  assert.equal(invalidRes.valid, false);
  assert.ok(invalidRes.errors.email);
  assert.ok(invalidRes.errors.imapPort);
  assert.ok(invalidRes.errors.smtpPort);
  assert.ok(invalidRes.errors.imapPassword);

  // 补齐合法参数：新增态仍要求密码非空
  form.email = "ok@qq.com";
  form.imapUsername = "ok@qq.com";
  form.imapPort = 993;
  form.smtpPort = 465;
  form.imapPassword = "secret-auth-code";
  assert.equal(validateAccountForm(form).valid, true);

  // 开启独立 SMTP 凭据时需填写 SMTP 用户名与密码
  form.smtpSeparateCredentials = true;
  const separateMissing = validateAccountForm(form);
  assert.equal(separateMissing.valid, false);
  assert.ok(separateMissing.errors.smtpUsername);
  assert.ok(separateMissing.errors.smtpPassword);

  form.smtpUsername = "sender@qq.com";
  form.smtpPassword = "smtp-secret";
  assert.equal(validateAccountForm(form).valid, true);

  // 编辑态且 hasPassword=true 时允许密码留空（表示保留原密码）
  form.id = "1";
  form.hasPassword = true;
  form.imapPassword = "";
  form.smtpPassword = "";
  assert.equal(validateAccountForm(form).valid, true);

  const payload = buildSaveAccountInput(form);
  assert.equal(payload.imapPassword, null);
  assert.equal(payload.smtpPassword, null);
  assert.equal(payload.smtpUsername, "sender@qq.com");

  // OAuth2 模式不需要输入密码
  const oauthForm = createEmptyAccountForm(
    BUILTIN_PROVIDER_PRESETS.find((p) => p.id === "outlook")
  );
  oauthForm.displayName = "Outlook 用户";
  oauthForm.email = "alice@outlook.com";
  assert.equal(validateAccountForm(oauthForm).valid, true);
});

test("Mock IPC 测试连接、保存、停用/启用、编辑保留密码与删除闭环", async () => {
  resetMockAccounts({ accounts: [] });
  let changedCalls = 0;
  const mgr = useAccounts({
    onChanged: () => {
      changedCalls += 1;
    },
  });
  await mgr.init();
  assert.equal(mgr.accounts.value.length, 0);

  // 填写 QQ 邮箱并测试连接
  mgr.startCreate("qq");
  mgr.updateEmail("test@qq.com");
  mgr.form.value.displayName = "小风筝";
  mgr.form.value.imapPassword = "top-secret-password-999";
  mgr.form.value.signature = "-- \nKiteMail";

  const verifyOk = await mgr.verifyCurrentAccount();
  assert.equal(verifyOk?.ok, true);

  // 模拟连接失败时也不暴露密码文本
  resetMockAccounts({
    accounts: [],
    verifyResponder: () => ({
      ok: false,
      message: "认证失败：授权码无效",
    }),
  });
  const verifyFail = await mgr.verifyCurrentAccount();
  assert.equal(verifyFail?.ok, false);
  assert.equal(mgr.statusBanner.value?.text.includes("top-secret-password-999"), false);

  // 保存新增账号
  const saved = await mgr.saveCurrentAccount();
  assert.ok(saved);
  assert.equal(saved.email, "test@qq.com");
  assert.equal(saved.hasPassword, true);
  assert.equal(mgr.accounts.value.length, 1);
  // 保存后内存表单密码清空
  assert.equal(mgr.form.value.imapPassword, "");

  // 停用再启用账号
  const toggledOff = await mgr.toggleAccountEnabled(saved.id, false);
  assert.equal(toggledOff, true);
  assert.equal(mgr.accounts.value[0].enabled, false);
  const toggledOn = await mgr.toggleAccountEnabled(saved.id, true);
  assert.equal(toggledOn, true);
  assert.equal(mgr.accounts.value[0].enabled, true);

  // 二次确认删除
  mgr.requestDelete(saved.id);
  assert.equal(mgr.confirmDeleteId.value, saved.id);
  const deleted = await mgr.confirmDelete();
  assert.equal(deleted, true);
  assert.equal(mgr.accounts.value.length, 0);
  assert.ok(changedCalls >= 3);

  const listAfter = await listAccounts();
  assert.equal(listAfter.accounts.length, 0);

  mgr.dispose();
  resetMockAccounts();
});

test("Microsoft OAuth 启动与 main://account-event 授权成功/改绑拒绝事件响应", async () => {
  resetMockAccounts({
    accounts: [],
    oauthMode: "autoSuccess",
    oauthDelayMs: 10,
  });

  let oauthAuthorizedEmail: string | null = null;
  const mgr = useAccounts({
    onOAuthAuthorized: (acc) => {
      oauthAuthorizedEmail = acc?.email ?? null;
    },
  });
  await mgr.init();

  mgr.startCreate("outlook");
  mgr.updateEmail("alice@outlook.com");
  const started = await mgr.startMicrosoftOAuthFlow();
  assert.equal(started, true);
  assert.equal(mgr.oauthStatus.value, "pending");

  // 等待 Mock OAuth 回调事件触发
  await new Promise((r) => setTimeout(r, 40));
  assert.equal(mgr.oauthStatus.value, "authorized");
  assert.equal(oauthAuthorizedEmail, "alice@outlook.com");
  assert.equal(mgr.accounts.value.length, 1);
  assert.equal(mgr.accounts.value[0].authBackend, "microsoftGraphOAuth2");

  // 模拟 OAuth 改绑被拒绝事件
  emitMockAccountEvent({
    stateVersion: 99,
    kind: "oauthRebindRejected",
    ok: false,
    accountId: mgr.accounts.value[0].id,
    email: "other@outlook.com",
    message: "授权邮箱与目标账号不一致，已拒绝改绑",
  });
  assert.equal(mgr.oauthStatus.value, "rejected");
  assert.equal(mgr.statusBanner.value?.kind, "error");
  assert.ok(mgr.oauthMessage.value?.includes("拒绝改绑"));

  mgr.dispose();
  resetMockAccounts();
});

test("编辑已有账号留空密码保留原凭据，以及 LOCKED 状态触发 onLocked 回调", async () => {
  resetMockAccounts();
  let lockedCalls = 0;
  const mgr = useAccounts({
    onLocked: () => {
      lockedCalls += 1;
    },
  });
  await mgr.init();
  assert.equal(mgr.accounts.value.length, 1);
  const existing = mgr.accounts.value[0];
  assert.equal(existing.hasPassword, true);

  // 修改备注后保存（密码留空）
  mgr.form.value.label = "主工作邮箱";
  const updated = await mgr.saveCurrentAccount();
  assert.ok(updated);
  assert.equal(updated.label, "主工作邮箱");
  assert.equal(updated.hasPassword, true);

  // 锁屏状态下操作触发 onLocked
  resetMockAccounts({ locked: true });
  const toggled = await mgr.toggleAccountEnabled(existing.id, false);
  assert.equal(toggled, false);
  assert.equal(lockedCalls, 1);

  mgr.dispose();
  resetMockAccounts();
});

test("brandForAccountFields 按主机/域名/认证方式匹配品牌图标", () => {
  const oauth = brandForAccountFields("microsoftGraphOAuth2", "", 993, "user@anywhere.org");
  assert.equal(oauth, "outlook");

  assert.equal(brandForAccountFields("password", "imap.qq.com", 993, "a@corp.cn"), "qq");
  assert.equal(brandForAccountFields("password", "mail.corp.internal", 993, "a@foxmail.com"), "qq");
  assert.equal(brandForAccountFields("password", "imap.163.com", 993, ""), "netease163");
  assert.equal(brandForAccountFields("password", "imap.qiye.aliyun.com", 993, ""), "aliyun_qiye");
  assert.equal(brandForAccountFields("password", "outlook.office365.com", 993, ""), "outlook");
  assert.equal(brandForAccountFields("password", "mail.corp.internal", 993, "u@hotmail.com"), "outlook");
  assert.equal(brandForAccountFields("password", "imap.gmail.com", 993, ""), "gmail");
  assert.equal(brandForAccountFields("password", "imap.mail.me.com", 993, ""), "icloud");
  assert.equal(brandForAccountFields("password", "127.0.0.1", 1143, "u@bridge.local"), "proton");
  assert.equal(brandForAccountFields("password", "mail.corp.internal", 993, "u@corp.internal"), null);
});

test("senderInitials 中文取 1 字、英文取首末词首字母，与 GPUI 规则一致", () => {
  assert.equal(senderInitials("张三"), "张");
  assert.equal(senderInitials("Marcus Chen"), "MC");
  assert.equal(senderInitials("alice <alice@example.com>"), "A");
  assert.equal(senderInitials("单字"), "单");
  assert.equal(senderInitials("   "), "?");
});

test("accountAccentHex 自定义色优先，否则按邮箱 FNV-1a 稳定取色", () => {
  assert.equal(accountAccentHex("a@qq.com", "#337FDC"), "#337fdc");
  const stable = accountAccentHex("stable@example.com");
  assert.equal(accountAccentHex("stable@example.com"), stable);
  assert.ok(ACCOUNT_ACCENT_COLORS.some(([hex]) => hex === stable));
  assert.equal(
    accountAccentHex("stable@example.com", "not-a-color"),
    stable
  );
});

test("签名预览：纯文本转义换行，HTML 清洗脚本与事件属性，纯文本模式不预览", () => {
  assert.equal(buildSignaturePreviewHtml("Ada & Co\nCEO", true), "Ada &amp; Co<br>CEO");

  const sanitized = buildSignaturePreviewHtml(
    `<p onclick="x()">Ada</p><script>alert(1)</script>`,
    true
  );
  assert.ok(sanitized.includes("Ada"), sanitized);
  assert.ok(!sanitized.includes("script"), sanitized);
  assert.ok(!sanitized.includes("onclick"), sanitized);

  assert.equal(buildSignaturePreviewHtml("<p>x</p>", false), "");
  assert.equal(buildSignaturePreviewHtml("  ", true), "");
});

test("useAccountLayers 两级层级：脏检查、未保存守卫与离开动作", async () => {
  resetMockAccounts();
  const state = useAccounts();
  await state.init();
  let closed = 0;
  const layers = useAccountLayers({ state, onClose: () => { closed += 1; } });

  assert.equal(layers.layer.value, "list");

  // 未修改：行点击直接进编辑，返回直接回列表
  layers.onRowSelect(state.accounts.value[0].id);
  assert.equal(layers.layer.value, "editor");
  assert.equal(layers.dirty.value, false);
  layers.requestBack();
  assert.equal(layers.layer.value, "list");

  // 修改昵称后：返回挂起「回列表」动作，取消后停留在编辑层
  layers.onRowSelect(state.accounts.value[0].id);
  state.form.value.label = "脏昵称";
  assert.equal(layers.dirty.value, true);
  layers.requestBack();
  assert.equal(layers.pendingLeave.value?.type, "list");
  layers.pendingLeave.value = null;

  // 行切换守卫：编辑中脏 → 挂起 select 动作
  layers.onRowSelect(state.accounts.value[0].id);
  assert.equal(layers.pendingLeave.value?.type, "select");
  assert.equal(layers.layer.value, "editor");

  // 关闭守卫：挂起 close，放弃更改后执行宿主 onClose
  assert.equal(layers.requestClose(), true);
  assert.equal(layers.pendingLeave.value?.type, "close");
  assert.equal(closed, 0);
  layers.discardPendingLeave();
  assert.equal(closed, 1);

  // 横幅「保存」：保存成功后执行挂起的离开动作（回列表）
  layers.openEditor(state.accounts.value[0].id);
  state.form.value.label = "保存后昵称";
  layers.requestBack();
  await layers.saveAndLeave();
  assert.equal(layers.layer.value, "list");
  assert.equal(layers.dirty.value, false);
  assert.equal(state.accounts.value[0].label, "保存后昵称");

  // 顶栏保存：编辑组件先调 IPC 保存，宿主收到 saved 事件后按 GPUI 行为回列表
  layers.onRowSelect(state.accounts.value[0].id);
  state.form.value.label = "顶栏保存昵称";
  const saved = await state.saveCurrentAccount();
  assert.ok(saved);
  layers.onEditorSaved();
  assert.equal(layers.layer.value, "list");
  assert.equal(state.accounts.value[0].label, "顶栏保存昵称");

  state.dispose();
  resetMockAccounts();
});

test("saveTag 名称必填、颜色规整、快捷键 1..9 且跨标签唯一", () => {
  const base = [{ id: 1, name: "工作", color: "#337FDC", shortcut: 3 }];

  const empty = saveTag(base, { id: null, name: "  ", color: "#29ae74", shortcut: null });
  assert.equal(empty.error, "标签名称不能为空");

  const added = saveTag(base, { id: null, name: " 紧急 ", color: "nope", shortcut: 2 });
  assert.equal(added.error, null);
  assert.equal(added.tags.length, 2);
  assert.equal(added.tags[1].name, "紧急");
  assert.equal(added.tags[1].color, "#3584e4");
  assert.equal(added.tags[1].shortcut, 2);
  assert.equal(added.tags[0].shortcut, 3);

  // 同一快捷键移到新标签时，旧标签快捷键被清空
  const moved = saveTag(added.tags, { id: null, name: "稍后", color: "#29AE74", shortcut: 3 });
  assert.equal(moved.tags[0].shortcut, null);
  assert.equal(moved.tags[2].shortcut, 3);
  assert.equal(moved.tags[2].color, "#29ae74");

  const clamped = saveTag([], { id: null, name: "x", color: "#29ae74", shortcut: 12 });
  assert.equal(clamped.tags[0].shortcut, null);

  assert.equal(normalizeTagShortcut(0), null);
  assert.equal(normalizeTagShortcut(9), 9);
  assert.equal(tagShortcutLabel(3), "快捷键 3");
  assert.equal(tagShortcutLabel(null), "无快捷键");
  assert.equal(normalizeTagColor("#337FDC"), "#337fdc");
});

test("buildFilterRuleSummary 按字段/匹配/动作生成条件摘要", () => {
  const rule = (overrides: Partial<FilterRuleItem>): FilterRuleItem => ({
    id: 1,
    name: "r",
    accountEmail: "",
    enabled: true,
    field: "from",
    matcher: "contains",
    value: "boss",
    moveToFolder: "",
    archive: false,
    star: false,
    markRead: false,
    applyTag: "",
    forwardTo: "",
    ...overrides,
  });

  assert.equal(
    buildFilterRuleSummary(rule({})),
    "发件人 包含 “boss” → 无动作"
  );
  assert.equal(
    buildFilterRuleSummary(
      rule({
        field: "subject",
        matcher: "startsWith",
        moveToFolder: " 工作 ",
        archive: true,
        markRead: true,
        applyTag: " 紧急 ",
      })
    ),
    "主题 开头为 “boss” → 移动到 工作、归档、标为已读、打标签 紧急"
  );
  assert.equal(filterFieldLabel("body"), "正文");
});

test("smartFolderSubtitle 显示账号范围与查询", () => {
  assert.equal(smartFolderSubtitle("", "from:alice"), "全部账号 · 查询: from:alice");
  assert.equal(smartFolderSubtitle("a@b.com", "is:unread"), "a@b.com · 查询: is:unread");
});

test("normalizeBlockedSender 校验邮箱/域名并规整小写", () => {
  assert.equal(normalizeBlockedSender("  User@Example.COM  "), "user@example.com");
  assert.equal(normalizeBlockedSender("张三 <Boss@Corp.com>"), "boss@corp.com");
  assert.equal(normalizeBlockedSender("example.com"), "example.com");
  assert.equal(normalizeBlockedSender("user@"), null);
  assert.equal(normalizeBlockedSender("bad/path"), null);
  assert.equal(normalizeBlockedSender("   "), null);
});

test("黑名单增删筛选：去重、排序与实时过滤", () => {
  const added = addToBlocklist(["zz@x.com"], "A@B.com");
  assert.deepEqual(added.blocked, ["a@b.com", "zz@x.com"]);
  assert.equal(added.added, "a@b.com");

  // 重复添加不产生新条目，但仍返回规整结果（对照 block_sender 返回 Some）
  const dup = addToBlocklist(added.blocked, "A@B.COM");
  assert.equal(dup.blocked.length, 2);
  assert.equal(dup.added, "a@b.com");

  const illegal = addToBlocklist(added.blocked, "bad/path");
  assert.equal(illegal.added, null);
  assert.equal(illegal.blocked.length, 2);

  assert.deepEqual(filterBlocklist(added.blocked, "ZZ"), ["zz@x.com"]);
  assert.deepEqual(filterBlocklist(added.blocked, "nomatch"), []);
  assert.deepEqual(removeFromBlocklist(added.blocked, "A@b.com"), ["zz@x.com"]);
});

test("registerPgpKey 邮箱必填、指纹自动生成与标题说明", () => {
  const bad = registerPgpKey([], { name: "a", email: "no-at", fingerprint: "" });
  assert.equal(bad.error, "请输入有效的绑定邮箱地址");

  const ok = registerPgpKey([], {
    name: " 张三 ",
    email: " zhang@corp.com ",
    fingerprint: " ABCDEF ",
    nowMs: Date.UTC(2026, 0, 15),
  });
  assert.equal(ok.error, null);
  assert.equal(ok.keys[0].name, "张三");
  assert.equal(ok.keys[0].email, "zhang@corp.com");
  assert.equal(ok.keys[0].fingerprint, "ABCDEF");
  assert.equal(ok.keys[0].hasSecret, true);
  assert.equal(ok.keys[0].trusted, true);
  assert.equal(ok.keys[0].createdAt, "2026-01-15");

  const auto = registerPgpKey([], { name: "", email: "a@b.com", fingerprint: "", nowMs: 0 });
  assert.equal(auto.keys[0].fingerprint, generateLocalFingerprint(0));
  assert.match(auto.keys[0].fingerprint, /^LOCAL-[0-9A-F]{8}$/);

  assert.equal(pgpKeyTitle(ok.keys[0]), "张三 <zhang@corp.com>");
  assert.equal(pgpKeyTitle({ name: "", email: "a@b.com" }), "a@b.com");
  assert.equal(pgpKeySubtitle({ fingerprint: "ABC", hasSecret: false }), "指纹: ABC · 仅公钥");
});
