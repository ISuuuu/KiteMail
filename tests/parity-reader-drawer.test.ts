import test from "node:test";
import assert from "node:assert/strict";
import {
  findInReader,
  getReaderViewOptions,
  listTags,
  mainBootstrap,
  resetMockSession,
  setMessageTags,
  setReaderViewOptions,
  unmarkMessageSpam,
} from "../src/ipc/main-window.ts";
import {
  __resetReaderToolsMock,
  getMockOpenedAttachmentIds,
} from "../src/reader-tools/ipc.ts";
import {
  aggregateThreadAttachments,
  isPdfAttachment,
  sortDrawerAttachments,
  useAttachments,
} from "../src/reader-tools/useAttachments.ts";
import { useMainWindow } from "../src/main-window/useMainWindow.ts";
import {
  buildFallbackFindSegments,
  computeReaderOverflowActions,
  countFallbackFindMatches,
  countRecipientsDisplay,
  isSpamFolderPath,
  nextFindIndex,
  splitSenderDisplay,
} from "../src/main-window/readerHelpers.ts";

test("aggregateThreadAttachments 跨会话去重、识别 PDF 且 sortDrawerAttachments 支持正倒序", () => {
  const topAtts = [
    { id: 10, filename: "z-notes.txt", mimeType: "text/plain", sizeBytes: 120, downloaded: false },
    { id: 11, filename: "roadmap.pdf", mimeType: "application/pdf", sizeBytes: 4096, downloaded: true },
  ];
  const threadMsgs = [
    {
      id: 101,
      accountId: 1,
      sender: "a@example.com",
      recipients: "b@example.com",
      cc: "",
      dateDisplay: "昨天",
      preview: "",
      plainText: null,
      hasHtml: false,
      htmlBody: null,
      unread: false,
      flagged: false,
      attachments: [
        { id: 11, filename: "roadmap.pdf", mimeType: "application/pdf", sizeBytes: 4096 },
        { id: 12, filename: "diagram.png", mimeType: "image/png", sizeBytes: 2048 },
      ],
    },
  ];

  const items = aggregateThreadAttachments(topAtts, threadMsgs);
  assert.equal(items.length, 3);
  assert.deepEqual(
    items.map((x) => x.id),
    [10, 11, 12]
  );
  assert.equal(items[1].isPdf, true);
  assert.equal(items[1].isImage, false);
  assert.equal(items[2].isImage, true);
  assert.equal(isPdfAttachment({ mimeType: "application/octet-stream", filename: "Report.PDF" }), true);

  const asc = sortDrawerAttachments(items, "asc");
  assert.deepEqual(
    asc.map((x) => x.filename),
    ["diagram.png", "roadmap.pdf", "z-notes.txt"]
  );
  const desc = sortDrawerAttachments(items, "desc");
  assert.deepEqual(
    desc.map((x) => x.filename),
    ["z-notes.txt", "roadmap.pdf", "diagram.png"]
  );
});

test("saveManyWithPicker 支持取消目录选择与确认保存，openWithDefaultApp 正确记录打开附件", async () => {
  __resetReaderToolsMock({ pickedDir: null });
  const state = useAttachments();

  const cancelled = await state.saveManyWithPicker([1, 2]);
  assert.equal(cancelled, null);
  assert.equal(state.error.value, null);
  assert.equal(state.lastBatchSaveResult.value, null);

  __resetReaderToolsMock({ pickedDir: "D:\\Exported" });
  const confirmed = await state.saveManyWithPicker([1, 2]);
  assert.ok(confirmed);
  assert.equal(confirmed.destDir, "D:\\Exported");
  assert.equal(confirmed.savedCount, 2);

  const opened = await state.openWithDefaultApp(5);
  assert.ok(opened);
  assert.equal(opened.opened, true);
  assert.deepEqual([...getMockOpenedAttachmentIds()], [5]);

  state.reset();
  __resetReaderToolsMock();
});

test("标签列表、标签写入、垃圾箱恢复校验与单封亮暗/阅读视图/正文查找 IPC", async () => {
  resetMockSession({ locked: false });
  const boot = await mainBootstrap();
  assert.ok(boot.accounts.length >= 1);

  const tags = await listTags();
  assert.ok(tags.length >= 4);

  const tagOut = await setMessageTags({
    accountId: 1,
    messageId: 1,
    tags: ["工作", " 重要 ", "工作"],
  });
  assert.deepEqual(tagOut.tags, ["工作", "重要"]);

  // 非垃圾箱邮件恢复应报错
  await assert.rejects(
    () => unmarkMessageSpam({ accountId: 1, messageId: 1 }),
    (err: unknown) =>
      typeof err === "object" &&
      err !== null &&
      String((err as { message?: string }).message).includes("垃圾邮件")
  );

  // 垃圾箱邮件可恢复至 INBOX
  const spamMsgId = 10000 + 4 + 2 + 2 + 1;
  const unspamOut = await unmarkMessageSpam({ accountId: 1, messageId: spamMsgId });
  assert.equal(unspamOut.targetFolderPath, "INBOX");

  const initOpts = await getReaderViewOptions({ accountId: 1, messageId: 1 });
  assert.equal(initOpts.perMessageDark, null);
  assert.equal(initOpts.readingView, false);

  const updatedOpts = await setReaderViewOptions({
    accountId: 1,
    messageId: 1,
    perMessageDark: true,
    readingView: true,
  });
  assert.equal(updatedOpts.perMessageDark, true);
  assert.equal(updatedOpts.readingView, true);

  const findRes = await findInReader({
    readerSessionId: "mock-sess",
    query: "KiteMail",
    step: 1,
  });
  assert.equal(findRes.dispatched, false);

  // 验证 useMainWindow 中的标签、阅读模式和查找状态联动
  resetMockSession({ locked: false });
  const win = useMainWindow();
  await win.init();
  assert.ok(win.availableTags.value.length >= 4);
  win.selectMessage({ accountId: 1, messageId: 1 });
  await new Promise((r) => setTimeout(r, 15));
  await win.setOpenedMessageTags(["重要"]);
  assert.deepEqual(win.selectedTags.value, ["重要"]);

  assert.equal(win.perMessageDark.value, null);
  await win.togglePerMessageDark();
  assert.equal(win.perMessageDark.value, true);
  await win.togglePerMessageDark();
  assert.equal(win.perMessageDark.value, null);

  await win.toggleReadingView();
  assert.equal(win.readingView.value, true);

  win.openReaderFind();
  assert.equal(win.readerFindOpen.value, true);
  await win.runReaderFind("KiteMail");
  assert.equal(win.readerFindQuery.value, "KiteMail");
  win.closeReaderFind();
  assert.equal(win.readerFindOpen.value, false);
  assert.equal(win.readerFindQuery.value, "");

  win.dispose();
  resetMockSession();
});

test("纯文本查找分段计数、循环跳转及工具栏窄窗溢出折叠计算", () => {
  const { segments, matchCount } = buildFallbackFindSegments("Hello KiteMail, hello world", "hello", 0);
  assert.equal(matchCount, 2);
  assert.deepEqual(
    segments.filter((s) => s.hit).map((s) => ({ text: s.text, hitIndex: s.hitIndex })),
    [
      { text: "Hello", hitIndex: 0 },
      { text: "hello", hitIndex: 1 },
    ]
  );

  assert.equal(countFallbackFindMatches(["Alpha beta", "BETA gamma beta"], "beta"), 3);
  assert.equal(nextFindIndex(0, 3, 1), 1);
  assert.equal(nextFindIndex(0, 3, -1), 2);
  assert.equal(nextFindIndex(2, 3, 1), 0);
  assert.equal(nextFindIndex(0, 0, 1), -1);

  assert.equal(isSpamFolderPath("Junk"), true);
  assert.equal(isSpamFolderPath("Spam/Sub"), true);
  assert.equal(isSpamFolderPath("INBOX"), false);

  assert.deepEqual(splitSenderDisplay("Alice <alice@example.com>"), {
    name: "Alice",
    email: "alice@example.com",
  });
  assert.equal(countRecipientsDisplay("a@x.com, b@x.com", "c@x.com"), 3);

  assert.deepEqual(computeReaderOverflowActions(673), []);
  assert.deepEqual(computeReaderOverflowActions(510), ["print", "move"]);
  assert.deepEqual(computeReaderOverflowActions(450), ["print", "move", "spam", "tags"]);
  assert.ok(computeReaderOverflowActions(360).includes("find"));
});
