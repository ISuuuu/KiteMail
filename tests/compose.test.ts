import test from "node:test";
import assert from "node:assert/strict";
import { isLockedError } from "../src/ipc/main-window.ts";
import {
  deleteDraft,
  flushMockOutboxDelivery,
  getMockSendRequests,
  listDrafts,
  listOutbox,
  resetMockComposeState,
  saveDraft,
  sendMail,
  undoSendMail,
} from "../src/compose/ipc.ts";
import {
  buildReplyPrefill,
  detectMissingAttachmentReminder,
  normalizeSubjectPrefix,
  renderSimpleMarkdownToHtml,
  useCompose,
} from "../src/compose/useCompose.ts";
import type { ComposeAccountOption, ComposeOriginalMessage } from "../src/compose/types.ts";

const ACCOUNTS: ComposeAccountOption[] = [
  {
    id: "1",
    displayName: "我方账号",
    email: "me@kitemail.example",
    signature: "Best,\nKiteMail User",
  },
  {
    id: "2",
    displayName: "工作邮箱",
    email: "work@company.example",
  },
];

const SAMPLE_MSG: ComposeOriginalMessage = {
  accountId: "1",
  sender: "Alice <alice@example.com>",
  recipients: '"Me" <me@kitemail.example>, Bob <bob@example.com>, ALICE@example.com',
  cc: "Carol <carol@example.com>, Bob <bob@example.com>, me@kitemail.example",
  subject: "季度评审纪要",
  dateDisplay: "2026-11-27 10:00",
  plainText: "第一行正文\n第二行正文",
  rfcMessageId: "<msg-100@example.com>",
};

test("buildReplyPrefill 正确生成回复、全部回复（去重并过滤自身）与转发预填", () => {
  assert.equal(normalizeSubjectPrefix("Hello", "Re:"), "Re: Hello");
  assert.equal(normalizeSubjectPrefix("Re: Hello", "Re:"), "Re: Hello");
  assert.equal(normalizeSubjectPrefix("回复：Hello", "Re:"), "回复：Hello");
  assert.equal(normalizeSubjectPrefix("Hello", "Fwd:"), "Fwd: Hello");
  assert.equal(normalizeSubjectPrefix("Fwd: Hello", "Fwd:"), "Fwd: Hello");
  assert.equal(normalizeSubjectPrefix("转发: Hello", "Fwd:"), "转发: Hello");

  // 回复：优先 replyTo，不带 Cc，带 Re: 与 inReplyTo，引用原正文
  const replyForm = buildReplyPrefill(
    {
      mode: "reply",
      accountId: "1",
      originalMessage: {
        ...SAMPLE_MSG,
        replyTo: "reply-alice@example.com",
      },
    },
    ACCOUNTS
  );
  assert.equal(replyForm.toAddr, "reply-alice@example.com");
  assert.equal(replyForm.ccAddr, "");
  assert.equal(replyForm.subject, "Re: 季度评审纪要");
  assert.equal(replyForm.inReplyTo, "<msg-100@example.com>");
  assert.ok(replyForm.bodyText.includes("> 第一行正文"));
  assert.ok(replyForm.bodyText.includes("> 第二行正文"));

  // 全部回复：过滤 me@kitemail.example，去重 Bob 与 Alice（大小写无关），Cc 不重复 To 中的 Bob
  const replyAllForm = buildReplyPrefill(
    {
      mode: "replyAll",
      accountId: "1",
      originalMessage: SAMPLE_MSG,
    },
    ACCOUNTS
  );
  assert.equal(replyAllForm.toAddr, "Alice <alice@example.com>, Bob <bob@example.com>");
  assert.equal(replyAllForm.ccAddr, "Carol <carol@example.com>");
  assert.equal(replyAllForm.showCcBcc, true);
  assert.equal(replyAllForm.subject, "Re: 季度评审纪要");

  // 转发：清空收件人与 inReplyTo，生成 Fwd: 前缀与转发头
  const fwdForm = buildReplyPrefill(
    {
      mode: "forward",
      accountId: "1",
      originalMessage: SAMPLE_MSG,
    },
    ACCOUNTS
  );
  assert.equal(fwdForm.toAddr, "");
  assert.equal(fwdForm.inReplyTo, null);
  assert.equal(fwdForm.subject, "Fwd: 季度评审纪要");
  assert.ok(fwdForm.bodyText.includes("---------- 转发的邮件 ----------"));
  assert.ok(fwdForm.bodyText.includes("第一行正文"));
});

test("detectMissingAttachmentReminder 准确识别忘带附件并忽略引用行与已附文件", () => {
  const zhHit = detectMissingAttachmentReminder("季度报告", "请查收附件中的 PDF。", []);
  assert.equal(zhHit.shouldWarn, true);
  assert.ok(zhHit.matchedKeywords.includes("附件"));

  const enHit = detectMissingAttachmentReminder(
    "Meeting notes",
    "Please find the slides attached below.",
    []
  );
  assert.equal(enHit.shouldWarn, true);
  assert.ok(enHit.matchedKeywords.includes("attached"));

  // 已添加附件时不提醒
  const withAttach = detectMissingAttachmentReminder(
    "季度报告",
    "请查收附件",
    ["C:\\report.pdf"]
  );
  assert.equal(withAttach.shouldWarn, false);

  // 仅在 > 引用行中出现关键词时不误报
  const quotedOnly = detectMissingAttachmentReminder(
    "Re: 季度报告",
    "收到，谢谢！\n\n> 原文：请查收附件。",
    []
  );
  assert.equal(quotedOnly.shouldWarn, false);

  // 英文词边界：detached 不触发 attached
  const wordBoundary = detectMissingAttachmentReminder(
    "Status",
    "This process is detached.",
    []
  );
  assert.equal(wordBoundary.shouldWarn, false);
});

test("1.5s 防抖静默自动保存草稿与关闭前自动落盘", async () => {
  resetMockComposeState();
  const compose = useCompose({
    accounts: ACCOUNTS,
    autoSaveDebounceMs: 30,
  });
  await compose.init();

  compose.openCompose({ mode: "new", accountId: "1" });
  compose.updateField("toAddr", "bob@example.com");
  compose.updateField("subject", "第一版主题");
  compose.updateField("bodyText", "草稿内容 v1");
  assert.equal(compose.dirty.value, true);

  // 快速连续输入应在防抖窗口合并为一次静默保存
  await new Promise((r) => setTimeout(r, 60));
  assert.equal(compose.dirty.value, false);
  assert.ok(compose.form.value.localDraftId);
  assert.equal(compose.drafts.value.length, 1);
  assert.equal(compose.drafts.value[0].subject, "第一版主题");

  // 修改后直接关闭（模拟 Esc）应在关闭前自动保存最新内容
  compose.updateField("bodyText", "草稿内容 v2");
  assert.equal(compose.dirty.value, true);
  await compose.closeWithAutoSave();
  assert.equal(compose.open.value, false);
  assert.equal(compose.dirty.value, false);
  assert.equal(compose.drafts.value[0].bodyText, "草稿内容 v2");

  compose.dispose();
  resetMockComposeState();
});

test("Mock send_mail + undo_send_mail 闭环、草稿清理与表单恢复", async () => {
  resetMockComposeState({ defaultUndoWindowSecs: 5, autoCompleteDelayMs: 5000 });
  const compose = useCompose({
    accounts: ACCOUNTS,
    autoSaveDebounceMs: 20,
  });
  await compose.init();

  compose.openCompose({ mode: "new", accountId: "1" });
  // 收件人为空校验
  const emptyRes = await compose.send();
  assert.equal(emptyRes.sent, false);
  assert.ok(compose.error.value?.includes("收件人"));

  // 触发忘带附件提醒拦截
  compose.updateField("toAddr", "alice@example.com");
  compose.updateField("subject", "项目资料");
  compose.updateField("bodyText", "详见附件，谢谢。");
  await compose.saveDraftNow(true);
  const draftId = compose.form.value.localDraftId;
  assert.ok(draftId);

  const warnRes = await compose.send();
  assert.equal(warnRes.sent, false);
  assert.equal(warnRes.needsAttachmentConfirm, true);

  // 忽略提醒后发送成功：清理关联草稿、启动撤销倒计时、关闭写信窗
  const sendRes = await compose.send({ ignoreAttachmentWarning: true });
  assert.equal(sendRes.sent, true);
  assert.equal(compose.open.value, false);
  assert.equal(compose.drafts.value.length, 0);
  assert.ok(compose.undoBanner.value);
  assert.equal(compose.undoBanner.value.remainingSecs, 5);
  assert.equal(compose.outboxItems.value.length, 1);

  // 撤销发送：取消发件队列、恢复草稿内容并重新打开写信窗
  const undone = await compose.undoSend();
  assert.equal(undone, true);
  assert.equal(compose.undoBanner.value, null);
  assert.equal(compose.open.value, true);
  assert.equal(compose.form.value.toAddr, "alice@example.com");
  assert.equal(compose.form.value.subject, "项目资料");
  assert.equal(compose.form.value.bodyText, "详见附件，谢谢。");
  assert.equal(compose.outboxItems.value.length, 0);

  // 再次发送并等待 Mock 投递完成事件清理队列
  const sendRes2 = await compose.send({ ignoreAttachmentWarning: true });
  assert.equal(sendRes2.sent, true);
  if (sendRes2.sent) {
    flushMockOutboxDelivery(sendRes2.output.outboxId);
  }
  await new Promise((r) => setTimeout(r, 15));
  assert.equal(compose.undoBanner.value, null);
  assert.equal(compose.outboxItems.value.length, 0);

  compose.dispose();
  resetMockComposeState();
});

test("定时发送、发件失败重试/删除与 LOCKED 错误拦截", async () => {
  resetMockComposeState({
    failDeliveryMessage: "SMTP 550 投递被拒绝",
    autoCompleteDelayMs: 20,
  });
  const compose = useCompose({ accounts: ACCOUNTS });
  await compose.init();

  // 失败投递进入 failed 状态，随后可重试或删除
  compose.openCompose({
    mode: "new",
    accountId: "2",
    to: "fail@example.com",
    subject: "测试失败重试",
    bodyText: "hello",
  });
  const failSend = await compose.send({ undoDelaySecs: 0 });
  assert.equal(failSend.sent, true);
  await new Promise((r) => setTimeout(r, 40));

  assert.equal(compose.outboxItems.value.length, 1);
  assert.equal(compose.outboxItems.value[0].status, "failed");
  assert.equal(compose.outboxItems.value[0].lastError, "SMTP 550 投递被拒绝");

  const failedId = compose.outboxItems.value[0].id;
  const retried = await compose.retryOutbox("2", failedId);
  assert.equal(retried, true);
  await new Promise((r) => setTimeout(r, 60));
  assert.equal(compose.outboxItems.value.length, 0);

  // 定时发送保留在 queued 队列并支持删除
  const futureTs = Math.floor(Date.now() / 1000) + 3600;
  compose.openCompose({
    mode: "new",
    accountId: "1",
    to: "later@example.com",
    subject: "定时邮件",
    bodyText: "一小时后发出",
    scheduledAt: futureTs,
  });
  const schedRes = await compose.send();
  assert.equal(schedRes.sent, true);
  if (schedRes.sent) {
    assert.equal(schedRes.output.scheduledAt, futureTs);
  }
  assert.equal(compose.outboxItems.value.length, 1);
  assert.equal(compose.outboxItems.value[0].sendAt, futureTs);

  const deleted = await compose.deleteOutbox("1", compose.outboxItems.value[0].id);
  assert.equal(deleted, true);
  assert.equal(compose.outboxItems.value.length, 0);

  // 锁屏状态下所有命令均返回 LOCKED
  resetMockComposeState({ locked: true });
  await assert.rejects(() => listDrafts(), isLockedError);
  await assert.rejects(
    () =>
      saveDraft({
        accountId: "1",
        toAddr: "a@b.com",
        subject: "s",
        bodyText: "b",
      }),
    isLockedError
  );
  await assert.rejects(() => deleteDraft({ draftId: "draft-1" }), isLockedError);
  await assert.rejects(
    () =>
      sendMail({
        accountId: "1",
        to: "a@b.com",
        subject: "s",
        body: "b",
      }),
    isLockedError
  );
  await assert.rejects(
    () =>
      undoSendMail({
        accountId: "1",
        outboxId: "outbox-1",
        undoToken: "tok",
      }),
    isLockedError
  );
  await assert.rejects(() => listOutbox(), isLockedError);

  compose.dispose();
  resetMockComposeState();
});

test("发件幂等键在回执超时重试时复用、编辑内容后重新生成", async () => {
  resetMockComposeState({
    sendError: { code: "SEND_TIMEOUT", message: "等待发件回执超时", retryable: true },
  });
  const compose = useCompose({ accounts: ACCOUNTS });
  await compose.init();

  compose.openCompose({ mode: "new", accountId: "1" });
  compose.updateField("toAddr", "idem@example.com");
  compose.updateField("subject", "幂等键");
  compose.updateField("bodyText", "正文");

  // 首次提交因回执超时失败，但请求已带幂等键
  const firstRes = await compose.send({ undoDelaySecs: 0 });
  assert.equal(firstRes.sent, false);
  const firstReq = getMockSendRequests().at(-1);
  assert.ok(firstReq?.clientRequestId);

  // 排除故障后重试：内容未变，复用同一个幂等键
  resetMockComposeState();
  const retryRes = await compose.send({ undoDelaySecs: 0 });
  assert.equal(retryRes.sent, true);
  const retryReq = getMockSendRequests().at(-1);
  assert.equal(retryReq?.clientRequestId, firstReq?.clientRequestId);

  // 编辑内容后再发：重新生成幂等键
  compose.openCompose({ mode: "new", accountId: "1" });
  compose.updateField("toAddr", "idem@example.com");
  compose.updateField("subject", "幂等键");
  compose.updateField("bodyText", "正文已修改");
  const editedRes = await compose.send({ undoDelaySecs: 0 });
  assert.equal(editedRes.sent, true);
  const editedReq = getMockSendRequests().at(-1);
  assert.ok(editedReq?.clientRequestId);
  assert.notEqual(editedReq?.clientRequestId, firstReq?.clientRequestId);

  compose.dispose();
  resetMockComposeState();
});

test("Markdown 转 HTML 预览转义安全脚本并渲染基础格式", () => {
  const html = renderSimpleMarkdownToHtml(
    "# 标题\n\n**加粗** 与 <script>alert(1)</script>\n\n- 列表项 1\n- 列表项 2"
  );
  assert.ok(html.includes("<h1>标题</h1>"));
  assert.ok(html.includes("<strong>加粗</strong>"));
  assert.ok(html.includes("&lt;script&gt;alert(1)&lt;/script&gt;"));
  assert.ok(!html.includes("<script>"));
  assert.ok(html.includes("<ul><li>列表项 1</li><li>列表项 2</li></ul>"));
});
