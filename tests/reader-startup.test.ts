import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import {
  LAYOUT_PENDING_TIMEOUT_MS,
  isLayoutPendingTimedOut,
  resolveReaderViewportMode,
  triggerEarlyWindowReveal,
} from "../src/main-window/startupHelpers";

test("resolveReaderViewportMode 区分导航就绪与原生布局生效，并优先上报错误", () => {
  // 1. 加载中保持 loading
  assert.equal(
    resolveReaderViewportMode({
      tauriMode: true,
      bodyStatus: "loading",
      hasHtml: true,
      threadCount: 1,
      overlayOccluded: false,
      layoutError: null,
      localLayoutApplied: false,
    }),
    "loading"
  );

  // 2. 导航就绪但尚未收到布局应用回执 -> layout-pending（不得误当作渲染完成）
  assert.equal(
    resolveReaderViewportMode({
      tauriMode: true,
      bodyStatus: "ready",
      hasHtml: true,
      threadCount: 1,
      overlayOccluded: false,
      layoutError: null,
      localLayoutApplied: false,
    }),
    "layout-pending"
  );

  // 3. 前端已发 setReaderLayout 但主线程尚未回执 layoutApplied=true -> 仍为 layout-pending
  assert.equal(
    resolveReaderViewportMode({
      tauriMode: true,
      bodyStatus: "ready",
      hasHtml: true,
      threadCount: 1,
      overlayOccluded: false,
      layoutError: null,
      localLayoutApplied: true,
      backendLayoutApplied: false,
    }),
    "layout-pending"
  );

  // 4. 导航就绪且布局与显示已确认 -> native-ready
  assert.equal(
    resolveReaderViewportMode({
      tauriMode: true,
      bodyStatus: "ready",
      hasHtml: true,
      threadCount: 1,
      overlayOccluded: false,
      layoutError: null,
      localLayoutApplied: true,
      backendLayoutApplied: true,
    }),
    "native-ready"
  );

  // 5. 多封会话即使无顶层 hasHtml 也启用原生阅读器
  assert.equal(
    resolveReaderViewportMode({
      tauriMode: true,
      bodyStatus: "ready",
      hasHtml: false,
      threadCount: 3,
      overlayOccluded: false,
      layoutError: null,
      localLayoutApplied: true,
      backendLayoutApplied: true,
    }),
    "native-ready"
  );

  // 6. 菜单/弹窗遮挡时进入 occluded 隐藏态
  assert.equal(
    resolveReaderViewportMode({
      tauriMode: true,
      bodyStatus: "ready",
      hasHtml: true,
      threadCount: 1,
      overlayOccluded: true,
      layoutError: null,
      localLayoutApplied: true,
      backendLayoutApplied: true,
    }),
    "occluded"
  );

  // 7. 布局调用失败时进入 unavailable（不得停留在占位或吞错）
  assert.equal(
    resolveReaderViewportMode({
      tauriMode: true,
      bodyStatus: "ready",
      hasHtml: true,
      threadCount: 1,
      overlayOccluded: false,
      layoutError: "SetWindowPos 失败",
      localLayoutApplied: false,
    }),
    "unavailable"
  );

  // 8. 单封纯文本或浏览器预览走 fallback
  assert.equal(
    resolveReaderViewportMode({
      tauriMode: true,
      bodyStatus: "ready",
      hasHtml: false,
      threadCount: 1,
      overlayOccluded: false,
      layoutError: null,
      localLayoutApplied: false,
    }),
    "fallback"
  );
  assert.equal(
    resolveReaderViewportMode({
      tauriMode: false,
      bodyStatus: "ready",
      hasHtml: true,
      threadCount: 1,
      overlayOccluded: false,
      layoutError: null,
      localLayoutApplied: false,
    }),
    "fallback"
  );
});

test("triggerEarlyWindowReveal 立即携带来源与耗时显窗，且失败时显式上报不吞错", async () => {
  const calls: Array<{ source: string; stageMs: number }> = [];
  let succeeded = false;
  const ok = await triggerEarlyWindowReveal({
    source: "shell-mounted",
    now: () => 42.5,
    reveal: async (source, stageMs) => {
      calls.push({ source, stageMs });
    },
    onSuccess: () => {
      succeeded = true;
    },
  });
  assert.equal(ok, true);
  assert.equal(succeeded, true);
  assert.deepEqual(calls, [{ source: "shell-mounted", stageMs: 42.5 }]);

  let reportedError = "";
  const failOk = await triggerEarlyWindowReveal({
    source: "shell-mounted",
    now: () => 50,
    reveal: async () => {
      throw new Error("show window denied");
    },
    onError: (msg) => {
      reportedError = msg;
    },
  });
  assert.equal(failOk, false);
  assert.equal(reportedError, "show window denied");
});

test("layout-pending 超时兜底：有限等待后转不可用而不是永久等待回执", () => {
  assert.ok(
    LAYOUT_PENDING_TIMEOUT_MS >= 3000 && LAYOUT_PENDING_TIMEOUT_MS <= 5000,
    "等待回执的窗口应落在 3-5s"
  );
  assert.equal(isLayoutPendingTimedOut(LAYOUT_PENDING_TIMEOUT_MS - 1), false);
  assert.equal(isLayoutPendingTimedOut(LAYOUT_PENDING_TIMEOUT_MS), true);
  assert.equal(isLayoutPendingTimedOut(Number.NaN), false);

  const viewportVue = fs.readFileSync(
    path.resolve("src/main-window/ReaderViewport.vue"),
    "utf8"
  );
  // 超时写入 localLayoutError 后由 resolveReaderViewportMode 转 unavailable，并保留重试入口
  assert.match(viewportVue, /armLayoutPendingWatchdog\(\);/);
  assert.match(viewportVue, /localLayoutError\.value = LAYOUT_PENDING_TIMEOUT_MESSAGE;/);
});

test("启动加载壳与阅读器回执源码守卫", () => {
  const indexHtml = fs.readFileSync(path.resolve("index.html"), "utf8");
  assert.match(indexHtml, /km-boot-shell/);
  assert.match(indexHtml, /prefers-color-scheme:\s*dark/);
  // 确保不引入违反 CSP script-src 'self' 的内联脚本
  const scriptTags = indexHtml.match(/<script\b[^>]*>/g) ?? [];
  assert.equal(scriptTags.length, 1);
  assert.match(scriptTags[0], /src="\/src\/main\.ts"/);

  const appVue = fs.readFileSync(path.resolve("src/App.vue"), "utf8");
  assert.match(appVue, /shell-mounted/);
  assert.doesNotMatch(appVue, /requestAnimationFrame\s*\(/);

  const viewportVue = fs.readFileSync(
    path.resolve("src/main-window/ReaderViewport.vue"),
    "utf8"
  );
  assert.doesNotMatch(viewportVue, /km-plain-backdrop/);
  assert.doesNotMatch(viewportVue, /setReaderLayout\([\s\S]*?\)\.catch\(\(\)\s*=>\s*\{\s*\}\)/);
});
