import test from "node:test";
import assert from "node:assert/strict";

import {
  BUILTIN_PROVIDER_PRESETS,
  emitMockAccountEvent,
  listAccounts,
  resetMockAccounts,
} from "../src/accounts/ipc.ts";
import {
  buildSaveAccountInput,
  createEmptyAccountForm,
  extractEmailDomain,
  matchPresetByEmail,
  useAccounts,
  validateAccountForm,
} from "../src/accounts/useAccounts.ts";

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
