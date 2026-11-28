import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import {
  __resetReaderToolsMock,
  fetchAttachmentPreview,
  fetchAttachmentThumbnail,
} from "../src/reader-tools/ipc.ts";
import { resolveDrawerThumbUrl, useAttachments } from "../src/reader-tools/useAttachments.ts";
import type { DrawerAttachmentItem, FetchAttachmentThumbnailOutput } from "../src/reader-tools/types.ts";

function thumbOutput(attachmentId: number, url: string | null): FetchAttachmentThumbnailOutput {
  return {
    attachmentId,
    filename: `att-${attachmentId}.png`,
    mimeType: "image/png",
    sizeBytes: 0,
    thumbnailDataUrl: url,
  };
}

function flushAsync(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 0));
}

function drawerItems(): DrawerAttachmentItem[] {
  return [
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
}

test("缩略图 IPC 只返回小图，原文件仍走完整预览", async () => {
  __resetReaderToolsMock();

  const pdfThumb = await fetchAttachmentThumbnail({ attachmentId: 1 });
  assert.ok(pdfThumb.thumbnailDataUrl?.startsWith("data:image/svg+xml"));
  assert.equal(pdfThumb.sizeBytes, 0);

  const fullPdf = await fetchAttachmentPreview({ attachmentId: 1 });
  assert.ok(fullPdf.dataUrl.startsWith("data:application/pdf;base64,"));

  const imageThumb = await fetchAttachmentThumbnail({ attachmentId: 2 });
  const imageSvg = decodeURIComponent((imageThumb.thumbnailDataUrl ?? "").split(",")[1] ?? "");
  assert.ok(imageSvg.includes('width="240"'), "缩略图应为受限尺寸小图");
  assert.equal(imageThumb.sizeBytes, 0);

  const fullImage = await fetchAttachmentPreview({ attachmentId: 2 });
  assert.ok(fullImage.dataUrl.startsWith("data:image/svg+xml"));
  assert.ok(fullImage.dataUrl.length > (imageThumb.thumbnailDataUrl ?? "").length);

  // 非图片非 PDF 没有缩略图
  const textThumb = await fetchAttachmentThumbnail({ attachmentId: 9 });
  assert.equal(textThumb.thumbnailDataUrl, null);

  __resetReaderToolsMock();
});

test("抽屉缩略图只预取图片与 PDF，失败项保留占位并可重试", async () => {
  __resetReaderToolsMock();
  const asked: number[] = [];
  const state = useAttachments({
    fetchThumbnailFn: async (input) => {
      asked.push(input.attachmentId);
      return fetchAttachmentThumbnail(input);
    },
  });

  const items = drawerItems();
  assert.equal(resolveDrawerThumbUrl(items[0], state.thumbUrls.value), null);
  assert.equal(resolveDrawerThumbUrl(items[1], state.thumbUrls.value), null);

  await state.prefetchThumbnails(items);

  assert.deepEqual(asked, [1, 2, 9999], "xlsx 不应触发缩略图请求");
  assert.equal(state.previewError.value, null, "失败项不污染预览错误");
  assert.equal(state.previewCache.value[1], undefined, "缩略图不写入完整预览缓存");

  assert.ok(
    resolveDrawerThumbUrl(items[0], state.thumbUrls.value)?.startsWith("data:image/svg+xml")
  );
  assert.ok(
    resolveDrawerThumbUrl(items[1], state.thumbUrls.value)?.startsWith("data:image/svg+xml")
  );
  assert.equal(resolveDrawerThumbUrl(items[2], state.thumbUrls.value), null);
  // 损坏 PDF 无缩略图，界面回落 SVG 占位
  assert.equal(resolveDrawerThumbUrl(items[3], state.thumbUrls.value), null);

  // 失败结果不缓存，重试会重新发起请求
  const retried = await state.requestThumbnail(9999);
  assert.equal(retried, null);
  assert.deepEqual(asked, [1, 2, 9999, 9999]);

  // 已缓存项不再重复请求
  const cachedAgain = await state.requestThumbnail(1);
  assert.ok(cachedAgain?.startsWith("data:image/svg+xml"));
  assert.deepEqual(asked, [1, 2, 9999, 9999]);

  // 图库翻页：当前页已全部命中缓存时不再请求
  asked.length = 0;
  await state.prefetchThumbnails(items.slice(0, 2));
  assert.deepEqual(asked, [], "缓存命中的当前页不再请求");

  state.reset();
  __resetReaderToolsMock();
});

test("缩略图并发上限为 2，同 id 在途请求合并", async () => {
  const pending: Array<(value: FetchAttachmentThumbnailOutput) => void> = [];
  let active = 0;
  let peak = 0;
  const state = useAttachments({
    thumbConcurrency: 2,
    fetchThumbnailFn: () =>
      new Promise<FetchAttachmentThumbnailOutput>((resolve) => {
        active += 1;
        peak = Math.max(peak, active);
        pending.push((value) => {
          active -= 1;
          resolve(value);
        });
      }),
  });

  const first = state.requestThumbnail(11);
  const deduped = state.requestThumbnail(11);
  assert.equal(first, deduped, "同 id 应复用同一个在途 Promise");

  const rest = [12, 13, 14].map((id) => state.requestThumbnail(id));
  assert.equal(pending.length, 2, "首轮只启动 2 个请求");

  pending.shift()!(thumbOutput(11, "data:image/png;base64,AA"));
  await flushAsync();
  assert.equal(pending.length, 2, "完成一个后补上一个");

  while (pending.length > 0) {
    const resolve = pending.shift()!;
    resolve(thumbOutput(0, "data:image/png;base64,AA"));
    await flushAsync();
  }

  const results = await Promise.all([first, ...rest]);
  assert.equal(results.length, 4);
  assert.ok(peak <= 2, `并发峰值应不超过 2，实际 ${peak}`);
  assert.equal(Object.keys(state.thumbUrls.value).length, 4);
});

test("切邮件/关抽屉后递增代次，晚到的缩略图结果被丢弃", async () => {
  const pending: Array<{ id: number; resolve: (value: FetchAttachmentThumbnailOutput) => void }> =
    [];
  const state = useAttachments({
    thumbConcurrency: 2,
    fetchThumbnailFn: ({ attachmentId }) =>
      new Promise<FetchAttachmentThumbnailOutput>((resolve) => {
        pending.push({ id: attachmentId, resolve });
      }),
  });

  const started = state.requestThumbnail(21);
  const queued = [22, 23].map((id) => state.requestThumbnail(id));
  assert.equal(pending.length, 2);

  // 切邮件：丢弃在途与排队请求
  state.invalidateThumbnails();
  assert.equal(await queued[1], null, "排队请求被放弃");
  assert.deepEqual(state.thumbUrls.value, {});

  pending.forEach(({ id, resolve }) => resolve(thumbOutput(id, "data:image/png;base64,AA")));
  assert.equal(await started, null, "在途结果因代次过期被丢弃");
  assert.equal(await queued[0], null);
  assert.deepEqual(state.thumbUrls.value, {}, "过期结果不写入缓存");

  // 新代次可以重新发起同一附件
  const retry = state.requestThumbnail(21);
  assert.equal(pending.length, 3);
  pending[2].resolve(thumbOutput(21, "data:image/png;base64,BB"));
  assert.equal(await retry, "data:image/png;base64,BB");
  assert.equal(state.thumbUrls.value[21], "data:image/png;base64,BB");
});

test("缩略图缓存受条目上限约束，超出按最早写入淘汰", async () => {
  const state = useAttachments({
    thumbCacheLimit: 2,
    fetchThumbnailFn: async ({ attachmentId }) =>
      thumbOutput(attachmentId, `data:image/png;base64,${attachmentId}`),
  });

  for (const id of [1, 2, 3]) {
    await state.requestThumbnail(id);
  }

  assert.equal(Object.keys(state.thumbUrls.value).length, 2);
  assert.equal(state.thumbUrls.value[1], undefined);
  assert.ok(state.thumbUrls.value[2]);
  assert.ok(state.thumbUrls.value[3]);
});

test("锁屏时清空缩略图与完整预览缓存", async () => {
  __resetReaderToolsMock();
  const state = useAttachments();

  await state.loadGallery(true);
  await state.openPreview(2);
  assert.ok(state.previewCache.value[2]);
  await state.requestThumbnail(2);
  assert.ok(state.thumbUrls.value[2]);

  __resetReaderToolsMock({ locked: true });
  await state.requestThumbnail(3);

  assert.equal(state.locked.value, true);
  assert.deepEqual(state.thumbUrls.value, {});
  assert.deepEqual(state.previewCache.value, {});
  assert.equal(state.activePreview.value, null);

  state.reset();
  __resetReaderToolsMock();
});

test("AttachmentDrawer 使用缩略图缓存、并发调度与生命周期失效", () => {
  const drawerSource = fs.readFileSync(
    path.join(process.cwd(), "src/reader-tools/AttachmentDrawer.vue"),
    "utf8"
  );
  assert.ok(drawerSource.includes("cachedThumbUrl(item)"));
  assert.ok(drawerSource.includes("state.thumbUrls.value"));
  assert.ok(drawerSource.includes("km-pdf-corner-badge"));
  assert.ok(drawerSource.includes('v-else-if="item.isPdf" class="km-pdf-thumb"'));
  assert.ok(drawerSource.includes("prefetchPreviewThumbnails"));
  assert.ok(drawerSource.includes("state.prefetchThumbnails("));
  assert.ok(drawerSource.includes("state.invalidateThumbnails()"));
});
