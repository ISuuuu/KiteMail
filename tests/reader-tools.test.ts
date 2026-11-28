import test from "node:test";
import assert from "node:assert/strict";
import { isLockedError } from "../src/ipc/main-window.ts";
import {
  __resetReaderToolsMock,
  fetchAttachmentPreview,
  getMessagePrintHtml,
  listCachedAttachments,
  openMessageWindow,
  saveAllAttachments,
  saveAttachment,
} from "../src/reader-tools/ipc.ts";
import {
  decodeTextPreviewDataUrl,
  formatAttachmentSize,
  isImageAttachment,
  resolveNumericAttachmentId,
  useAttachments,
} from "../src/reader-tools/useAttachments.ts";
import type { OpenMessageOutput } from "../src/ipc/types.ts";

test("全局附件搜索、类型过滤与分页加载", async () => {
  __resetReaderToolsMock();
  const state = useAttachments({ pageSize: 5 });

  // 首屏分页加载（默认 14 项，每页 5 项）
  const ok = await state.loadGallery(true);
  assert.equal(ok, true);
  assert.equal(state.items.value.length, 5);
  assert.equal(state.total.value, 14);
  assert.equal(state.hasMore.value, true);

  // 加载下一页并校验去重追加
  const loadedMore = await state.loadMore();
  assert.equal(loadedMore, true);
  assert.equal(state.items.value.length, 10);
  assert.equal(state.hasMore.value, true);

  // 类型过滤：all / images / documents
  state.setFilter("images");
  assert.ok(state.filteredItems.value.length > 0);
  assert.ok(state.filteredItems.value.every((item) => item.isImage));

  state.setFilter("documents");
  assert.ok(state.filteredItems.value.length > 0);
  assert.ok(state.filteredItems.value.every((item) => !item.isImage));

  // 搜索查询应重置分页并过滤结果
  state.setFilter("all");
  await state.setQuery("三栏");
  assert.equal(state.hasMore.value, false);
  assert.ok(state.items.value.length >= 2);
  assert.ok(
    state.items.value.every(
      (item) =>
        item.filename.includes("三栏") ||
        item.subject.includes("三栏") ||
        item.sender.includes("三栏")
    )
  );

  // 辅助函数验证
  assert.equal(formatAttachmentSize(512), "512 B");
  assert.equal(formatAttachmentSize(2048), "2.0 KB");
  assert.equal(isImageAttachment({ mimeType: "image/png", filename: "a.png" }), true);
  assert.equal(isImageAttachment({ mimeType: "application/pdf", filename: "a.pdf" }), false);
  assert.equal(resolveNumericAttachmentId("12"), 12);
  assert.equal(resolveNumericAttachmentId(undefined), null);

  state.reset();
  __resetReaderToolsMock();
});

test("图片与文档预览缓存命中，避免重复拉取 IPC", async () => {
  __resetReaderToolsMock();
  const state = useAttachments();
  await state.loadGallery(true);

  const first = await state.openPreview(2);
  assert.ok(first);
  assert.equal(first.attachmentId, 2);
  assert.ok(first.dataUrl.startsWith("data:image/svg+xml"));
  assert.equal(state.activePreview.value?.attachmentId, 2);
  assert.ok(state.previewCache.value[2]);

  // 开启预览失败注入后，二次读取同一附件应直接命中缓存而不报错
  __resetReaderToolsMock({ failPreviewMessage: "模拟网络中断" });
  const cached = await state.openPreview(2);
  assert.ok(cached);
  assert.equal(cached.dataUrl, first.dataUrl);
  assert.equal(state.previewError.value, null);

  // 强制刷新缓存或请求未缓存文档应触发实际 IPC 失败分支
  const uncachedFail = await state.fetchPreview(1);
  assert.equal(uncachedFail, null);
  assert.ok(state.previewError.value?.includes("模拟网络中断"));

  // 重置并校验文档文本 preview 解码
  __resetReaderToolsMock();
  const docPreview = await state.fetchPreview(9, { force: true });
  assert.ok(docPreview);
  assert.ok(docPreview.dataUrl.startsWith("data:text/plain"));
  const decoded = decodeTextPreviewDataUrl(docPreview.dataUrl);
  assert.ok(decoded?.includes("CHANGELOG-v0.4.txt"));

  state.closePreview();
  assert.equal(state.activePreview.value, null);
  assert.deepEqual(state.previewCache.value, {});
  state.reset();
  __resetReaderToolsMock();
});

test("完整预览只保留当前项，图库翻页按页调度缩略图并在关闭时释放", async () => {
  __resetReaderToolsMock();
  const state = useAttachments({ pageSize: 5 });
  await state.loadGallery(true);

  const pageIds = state.filteredItems.value.map((item) => item.id);
  assert.equal(pageIds.length, 5);

  await state.prefetchThumbnails(state.filteredItems.value);
  for (const item of state.filteredItems.value) {
    if (item.isImage) {
      assert.ok(state.thumbUrls.value[item.id], `图片 ${item.filename} 应有缩略图`);
    } else {
      assert.equal(state.thumbUrls.value[item.id], undefined, `文档 ${item.filename} 不应有缩略图`);
    }
  }
  assert.deepEqual(state.previewCache.value, {}, "缩略图不写入完整预览缓存");

  // 图库翻页：追加下一页后只为新页排缩略图，已缓存的保持不动
  const firstPageThumbs = { ...state.thumbUrls.value };
  await state.loadMore();
  assert.equal(state.items.value.length, 10);
  await state.prefetchThumbnails(state.filteredItems.value);
  for (const [id, url] of Object.entries(firstPageThumbs)) {
    assert.equal(state.thumbUrls.value[Number(id)], url, "已缓存缩略图不被重写");
  }
  assert.ok(Object.keys(state.thumbUrls.value).length > Object.keys(firstPageThumbs).length);

  // 完整预览按当前项替换，关闭即释放
  await state.openPreview(pageIds[0]);
  assert.ok(state.previewCache.value[pageIds[0]]);
  await state.openPreview(pageIds[1]);
  assert.equal(state.previewCache.value[pageIds[0]], undefined);
  assert.ok(state.previewCache.value[pageIds[1]]);

  state.closePreview();
  assert.deepEqual(state.previewCache.value, {});
  assert.equal(state.activePreview.value, null);

  // 重置清空缩略图缓存
  assert.ok(Object.keys(state.thumbUrls.value).length > 0);
  state.reset();
  assert.deepEqual(state.thumbUrls.value, {});
  __resetReaderToolsMock();
});

test("单件保存与批量全部保存更新状态并同步 downloaded 标记", async () => {
  __resetReaderToolsMock({ destDir: "D:\\Downloads\\KiteMail" });
  const state = useAttachments({ pageSize: 20 });
  await state.loadGallery(true);

  const targetBefore = state.items.value.find((i) => i.id === 3);
  assert.equal(targetBefore?.downloaded, false);

  const singleRes = await state.saveSingle(3);
  assert.ok(singleRes);
  assert.equal(singleRes.saved, true);
  assert.equal(singleRes.savedPath, "D:\\Downloads\\KiteMail\\暗色主题预览图.webp");
  assert.equal(state.items.value.find((i) => i.id === 3)?.downloaded, true);
  assert.ok(state.statusMessage.value?.includes("暗色主题预览图.webp"));

  const batchRes = await state.saveMany([5, 8], "E:\\Archive");
  assert.ok(batchRes);
  assert.equal(batchRes.savedCount, 2);
  assert.equal(batchRes.destDir, "E:\\Archive");
  assert.deepEqual(batchRes.savedPaths, [
    "E:\\Archive\\压测指标明细.xlsx",
    "E:\\Archive\\WebView2-DPI-150-截图.jpg",
  ]);
  assert.equal(state.items.value.find((i) => i.id === 5)?.downloaded, true);
  assert.equal(state.items.value.find((i) => i.id === 8)?.downloaded, true);

  state.reset();
  __resetReaderToolsMock();
});

test("get_message_print_html 安全转义危险字符与脚本并支持头信息开关", async () => {
  const xssMsg: OpenMessageOutput = {
    requestId: "req-xss",
    readerSessionId: "sess-xss",
    accountId: 1,
    messageId: 999,
    subject: '季度报告 <script>alert("xss")</script> & "报价"',
    sender: 'Evil <evil@example.com>" onmouseover="alert(1)',
    recipients: "demo@kitemail.example",
    cc: "<img src=x onerror=alert(2)>",
    dateDisplay: "2026-11-27 12:00",
    bodyStatus: "ready",
    plainText: "第一行 <script>evil()</script>\n第二行 & 特殊字符",
    hasHtml: true,
    htmlBody:
      '<p>正文内容</p><script>alert(1)</script><a href="javascript:alert(2)" onclick="alert(3)">链接</a><iframe src="https://evil.test"></iframe>',
    flagged: false,
    attachments: [
      {
        id: 1,
        filename: 'report<script>.pdf',
        mimeType: "application/pdf",
        sizeBytes: 1024,
      },
    ],
    threadMessages: [],
  };

  __resetReaderToolsMock({ customMessages: [xssMsg] });

  const withHeader = await getMessagePrintHtml({
    accountId: 1,
    messageId: 999,
    includeHeader: true,
  });
  assert.ok(!withHeader.htmlDocument.includes("<script>"));
  assert.ok(!withHeader.htmlDocument.includes("<iframe"));
  assert.ok(!withHeader.htmlDocument.includes("onclick="));
  assert.ok(!withHeader.htmlDocument.includes("javascript:alert"));
  assert.ok(withHeader.htmlDocument.includes("&lt;script&gt;alert(&quot;xss&quot;)&lt;/script&gt;"));
  assert.ok(withHeader.htmlDocument.includes("report&lt;script&gt;.pdf"));
  assert.ok(withHeader.htmlDocument.includes("km-print-meta"));

  const withoutHeader = await getMessagePrintHtml({
    accountId: 1,
    messageId: 999,
    includeHeader: false,
  });
  assert.ok(!withoutHeader.htmlDocument.includes('<dl class="km-print-meta">'));
  assert.ok(withoutHeader.htmlDocument.includes("正文内容"));

  __resetReaderToolsMock();
});

test("open_message_window 返回独立窗口标签与完整邮件内容，并校验 LOCKED 锁屏拦截", async () => {
  __resetReaderToolsMock();

  const win = await openMessageWindow({ accountId: 1, messageId: 3 });
  assert.equal(win.opened, true);
  assert.equal(win.windowLabel, "reader-1-3");
  assert.equal(win.message.accountId, 1);
  assert.equal(win.message.messageId, 3);
  assert.equal(win.message.attachments.length, 2);

  // 锁屏状态下全部 6 个 IPC 命令与 composable 状态均正确捕获 LOCKED 错误
  __resetReaderToolsMock({ locked: true });
  await assert.rejects(() => listCachedAttachments({}), isLockedError);
  await assert.rejects(() => fetchAttachmentPreview({ attachmentId: 1 }), isLockedError);
  await assert.rejects(() => saveAttachment({ attachmentId: 1 }), isLockedError);
  await assert.rejects(() => saveAllAttachments({ attachmentIds: [1, 2] }), isLockedError);
  await assert.rejects(() => getMessagePrintHtml({ accountId: 1, messageId: 3 }), isLockedError);
  await assert.rejects(() => openMessageWindow({ accountId: 1, messageId: 3 }), isLockedError);

  const state = useAttachments();
  const ok = await state.loadGallery(true);
  assert.equal(ok, false);
  assert.equal(state.locked.value, true);
  assert.ok(state.error.value?.includes("LOCKED"));

  state.reset();
  __resetReaderToolsMock();
});
