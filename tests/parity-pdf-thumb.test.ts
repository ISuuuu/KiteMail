import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import {
  __resetReaderToolsMock,
  fetchAttachmentPreview,
} from "../src/reader-tools/ipc.ts";
import {
  prefetchDrawerThumbnails,
  resolveDrawerThumbUrl,
  useAttachments,
} from "../src/reader-tools/useAttachments.ts";
import type { DrawerAttachmentItem, FetchAttachmentPreviewOutput } from "../src/reader-tools/types.ts";

function decodeBase64Ascii(b64: string): string {
  return Buffer.from(b64, "base64").toString("latin1");
}

test("Mock fetchAttachmentPreview 对 PDF 附件返回合法最小 PDF dataUrl 与图像类 thumbnailDataUrl", async () => {
  __resetReaderToolsMock();

  // id=1: KiteMail-三栏布局规范.pdf, id=10: 安全沙箱策略白皮书.pdf
  const pdfPreview = await fetchAttachmentPreview({ attachmentId: 1 });
  assert.equal(pdfPreview.attachmentId, 1);
  assert.equal(pdfPreview.mimeType, "application/pdf");
  assert.ok(pdfPreview.dataUrl.startsWith("data:application/pdf;base64,"));
  assert.ok(pdfPreview.thumbnailDataUrl?.startsWith("data:image/svg+xml"));

  const b64Payload = pdfPreview.dataUrl.slice("data:application/pdf;base64,".length);
  const rawPdf = decodeBase64Ascii(b64Payload);
  assert.ok(rawPdf.startsWith("%PDF-1.4\n"));
  assert.ok(rawPdf.includes("xref\n"));
  assert.ok(rawPdf.includes("startxref\n"));
  assert.ok(rawPdf.trimEnd().endsWith("%%EOF"));

  // 非 PDF 非图片附件不生成 thumbnailDataUrl
  const textPreview = await fetchAttachmentPreview({ attachmentId: 9 });
  assert.equal(textPreview.thumbnailDataUrl ?? null, null);

  __resetReaderToolsMock();
});

test("AttachmentDrawer 缩略图预取同时覆盖图片与 PDF 并支持 cachedThumbUrl 渲染与回落", async () => {
  __resetReaderToolsMock();
  const state = useAttachments();

  const items: DrawerAttachmentItem[] = [
    {
      id: 1,
      filename: "KiteMail-三栏布局规范.pdf",
      mimeType: "application/pdf",
      sizeBytes: 184_320,
      downloaded: true,
      isImage: false,
      isPdf: true,
    },
    {
      id: 2,
      filename: "三栏间距与色板对比.png",
      mimeType: "image/png",
      sizeBytes: 426_800,
      downloaded: true,
      isImage: true,
      isPdf: false,
    },
    {
      id: 5,
      filename: "压测指标明细.xlsx",
      mimeType: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
      sizeBytes: 95_232,
      downloaded: false,
      isImage: false,
      isPdf: false,
    },
    {
      id: 9999,
      filename: "broken-remote.pdf",
      mimeType: "application/pdf",
      sizeBytes: 10_240,
      downloaded: false,
      isImage: false,
      isPdf: true,
    },
  ];

  // 预取前均为 null（触发 PDF SVG 占位回落）
  assert.equal(resolveDrawerThumbUrl(items[0], state.previewCache.value), null);
  assert.equal(resolveDrawerThumbUrl(items[1], state.previewCache.value), null);

  const fetchedIds: number[] = [];
  await prefetchDrawerThumbnails(items, state.previewCache.value, async (id, opts) => {
    fetchedIds.push(id);
    return state.fetchPreview(id, opts);
  });

  // 仅对 isImage || isPdf 发起静默预取，不预取 xlsx，且失败项不污染 previewError
  assert.deepEqual(fetchedIds, [1, 2, 9999]);
  assert.equal(state.previewError.value, null);

  const pdfThumb = resolveDrawerThumbUrl(items[0], state.previewCache.value);
  const imgThumb = resolveDrawerThumbUrl(items[1], state.previewCache.value);
  const xlsxThumb = resolveDrawerThumbUrl(items[2], state.previewCache.value);
  const brokenPdfThumb = resolveDrawerThumbUrl(items[3], state.previewCache.value);

  assert.ok(pdfThumb?.startsWith("data:image/svg+xml"));
  assert.ok(imgThumb?.startsWith("data:image/svg+xml"));
  assert.equal(xlsxThumb, null);
  assert.equal(brokenPdfThumb, null);

  // 若后端返回 null/undefined thumbnailDataUrl，PDF 也安全回落为 null
  const noThumbCache: Record<number, FetchAttachmentPreviewOutput> = {
    1: {
      attachmentId: 1,
      filename: "KiteMail-三栏布局规范.pdf",
      mimeType: "application/pdf",
      dataUrl: "data:application/pdf;base64,JVBERi0xLjQK",
      sizeBytes: 12,
      thumbnailDataUrl: null,
    },
  };
  assert.equal(resolveDrawerThumbUrl(items[0], noThumbCache), null);

  // 二次预取命中缓存不再重复调用
  const secondFetchedIds: number[] = [];
  await prefetchDrawerThumbnails(items.slice(0, 2), state.previewCache.value, async (id) => {
    secondFetchedIds.push(id);
    return null;
  });
  assert.deepEqual(secondFetchedIds, []);

  // 验证 AttachmentDrawer.vue 使用了 cachedThumbUrl、PDF 角标与 SVG 回落
  const drawerSource = fs.readFileSync(
    path.join(process.cwd(), "src/reader-tools/AttachmentDrawer.vue"),
    "utf8"
  );
  assert.ok(drawerSource.includes("cachedThumbUrl(item)"));
  assert.ok(drawerSource.includes("item.isImage || item.isPdf"));
  assert.ok(drawerSource.includes("km-pdf-corner-badge"));
  assert.ok(drawerSource.includes("v-else-if=\"item.isPdf\" class=\"km-pdf-thumb\""));
  assert.ok(drawerSource.includes("prefetchPreviewThumbnails"));

  state.reset();
  __resetReaderToolsMock();
});
