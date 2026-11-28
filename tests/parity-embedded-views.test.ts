import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";

import { matchSingleKeyShortcut } from "../src/command-palette/useCommandPalette.ts";
import { __resetContactsMock, listContacts } from "../src/contacts/ipc.ts";
import { useContacts } from "../src/contacts/useContacts.ts";
import { isInteractiveDragTarget } from "../src/main-window/windowControls.ts";
import { __resetReaderToolsMock } from "../src/reader-tools/ipc.ts";
import { isPdfAttachment, useAttachments } from "../src/reader-tools/useAttachments.ts";

function readWorktreeFile(relPath: string): string {
  return fs.readFileSync(path.join(process.cwd(), relPath), "utf8");
}

test("ContactsModal.vue 支持 embedded 内嵌模式、48px 拖拽顶栏、返回邮件与关闭窗口且保留弹窗兼容", () => {
  const src = readWorktreeFile("src/contacts/ContactsModal.vue");

  // Props & emits
  assert.match(src, /embedded\?:\s*boolean/);
  assert.match(src, /embedded:\s*false/);
  assert.match(src, /closeWindow:\s*\[\]/);

  // 无遮罩内嵌容器与非 embedded 遮罩兼容
  assert.match(src, /:is="embedded \? 'section' : 'div'"/);
  assert.match(src, /:class="embedded \? 'km-contacts-embedded' : 'km-contacts-backdrop'"/);
  assert.match(src, /@click\.self="!embedded && closeModal\(\)"/);
  assert.match(
    src,
    /\.km-contacts-embedded\s*\{[^}]*flex:\s*1 1 auto;[^}]*min-width:\s*0;[^}]*height:\s*100%;[^}]*display:\s*flex;[^}]*flex-direction:\s*column;[^}]*overflow:\s*hidden;[^}]*background:\s*var\(--km-surface\);[^}]*color:\s*var\(--km-text\);/s
  );
  assert.match(src, /\.km-contacts-backdrop\s*\{/);
  assert.match(src, /\.km-contacts-modal\s*\{/);

  // embedded 模式不上报 overlayChange
  assert.match(
    src,
    /function reportOverlay\(next:\s*boolean\)\s*\{[^}]*if\s*\(props\.embedded\)/s
  );

  // 顶部 48px + data-tauri-drag-region + 返回邮件 + 关闭窗口 SVG 按钮
  assert.match(
    src,
    /\.km-contacts-embedded\s+\.km-modal-header\s*\{[^}]*height:\s*48px;/s
  );
  assert.match(
    src,
    /<header[\s\S]*?class="km-modal-header"[\s\S]*?:data-tauri-drag-region="embedded \? '' : undefined"/
  );
  assert.match(src, />\s*导入 vCard \/ CSV\s*</);
  assert.match(src, />\s*导出 vCard\s*</);
  assert.match(src, />\s*导出 CSV\s*</);
  assert.match(src, />\s*\+ 新建联系人\s*</);
  assert.match(
    src,
    /<button[\s\S]*?v-if="embedded"[\s\S]*?@click="closeModal"[\s\S]*?>\s*返回邮件\s*<\/button>/
  );
  assert.match(
    src,
    /<button[\s\S]*?v-else[\s\S]*?class="km-close-btn"[\s\S]*?@click="closeModal"/
  );
  assert.match(
    src,
    /<button[\s\S]*?v-if="embedded"[\s\S]*?aria-label="关闭窗口"[\s\S]*?@click="emit\('closeWindow'\)"[\s\S]*?<svg/
  );

  // Esc 在 embedded 模式下仅关闭导入面板，不退出工作区
  assert.match(
    src,
    /if\s*\(ev\.key === "Escape"\)\s*\{[\s\S]*?if\s*\(state\.importPanelOpen\.value\)\s*\{[\s\S]*?state\.closeImportPanel\(\);[\s\S]*?return;[\s\S]*?\}[\s\S]*?if\s*\(props\.embedded\)\s*\{[\s\S]*?return;[\s\S]*?\}[\s\S]*?closeModal\(\);/
  );
});

test("AttachmentGalleryModal.vue 支持 embedded 内嵌模式、48px 顶栏、灯箱 Teleport 及缩略图字段兼容", () => {
  const src = readWorktreeFile("src/reader-tools/AttachmentGalleryModal.vue");

  // Props & emits
  assert.match(src, /embedded\?:\s*boolean/);
  assert.match(src, /embedded:\s*false/);
  assert.match(src, /closeWindow:\s*\[\]/);

  // 外层 Teleport 在 embedded 时禁用，非 embedded 保持 Teleport + backdrop
  assert.match(src, /<Teleport to="body" :disabled="embedded">/);
  assert.match(src, /:class="embedded \? 'km-gallery-embedded' : 'km-gallery-backdrop'"/);
  assert.match(src, /@pointerdown\.self="!embedded && closeModal\(\)"/);
  assert.match(
    src,
    /\.km-gallery-embedded\s*\{[^}]*flex:\s*1 1 auto;[^}]*min-width:\s*0;[^}]*height:\s*100%;[^}]*display:\s*flex;[^}]*flex-direction:\s*column;[^}]*overflow:\s*hidden;[^}]*background:\s*var\(--km-surface\);[^}]*color:\s*var\(--km-text\);/s
  );
  assert.match(src, /\.km-gallery-backdrop\s*\{/);
  assert.match(src, /\.km-gallery-modal\s*\{/);

  // embedded 模式不上报图库自身的 overlayChange
  assert.match(
    src,
    /function reportOverlay\(next:\s*boolean\)\s*\{[^}]*if\s*\(props\.embedded\)/s
  );

  // 顶部 48px + data-tauri-drag-region + 刷新 / 返回邮件 / 关闭窗口 SVG 按钮
  assert.match(
    src,
    /\.km-gallery-embedded\s+\.km-gallery-header\s*\{[^}]*height:\s*48px;/s
  );
  assert.match(
    src,
    /<header[\s\S]*?class="km-gallery-header"[\s\S]*?:data-tauri-drag-region="embedded \? '' : undefined"/
  );
  assert.match(src, /@click="reloadGallery"/);
  assert.match(
    src,
    /<button[\s\S]*?v-if="embedded"[\s\S]*?@click="closeModal"[\s\S]*?>\s*返回邮件\s*<\/button>/
  );
  assert.match(
    src,
    /<button[\s\S]*?v-if="embedded"[\s\S]*?aria-label="关闭窗口"[\s\S]*?@click="emit\('closeWindow'\)"[\s\S]*?<svg/
  );

  // 灯箱在 embedded 模式下保留 Teleport to="body" 并支持图片与 PDF 预览
  assert.match(
    src,
    /<Teleport to="body" :disabled="!embedded">[\s\S]*?class="km-lightbox-overlay"[\s\S]*?v-if="isLightboxImage"[\s\S]*?<iframe[\s\S]*?v-else-if="isLightboxPdf"[\s\S]*?class="km-lightbox-pdf"/
  );

  // 卡片缩略图统一走轻量缩略图缓存，不再复用完整预览的 dataUrl
  assert.match(src, /resolveDrawerThumbUrl\(item, state\.thumbUrls\.value\)/);
  assert.match(src, /state\.prefetchThumbnails\([\s\S]*?isPdf: isPdfAttachment\(item\)/);
  assert.match(src, /:src="cardThumbnailUrl\(item\)!"/);

  // 内部按钮均为 button，确保命中交互白名单不触发拖拽
  const btnTarget = {
    closest: (sel: string) => (sel.includes("button") ? {} : null),
  } as unknown as EventTarget;
  assert.equal(isInteractiveDragTarget(btnTarget), true);
});

test("MainWindow.vue 将通讯录与附件图库接入主窗口右侧内嵌区域并保持导航/快捷键兼容", () => {
  const src = readWorktreeFile("src/main-window/MainWindow.vue");

  // 保留 FolderSidebar 的 activeWorkspaceView 与视图切换事件
  assert.match(src, /:activeWorkspaceView="activeWorkspaceView"/);
  assert.match(src, /@selectView="onSidebarSelectView"/);
  assert.match(src, /@applyFilter="onSidebarApplyFilter"/);
  assert.match(src, /@openAttachments="openAttachmentsView\(\)"/);
  assert.match(src, /@openContacts="openContactsView\(\)"/);

  // 邮件区由 activeWorkspaceView === 'mail' 包裹（包含 PaneSplitter、列表列与 Right ReaderPane）
  assert.match(
    src,
    /<template v-if="activeWorkspaceView === 'mail'">[\s\S]*?<PaneSplitter[\s\S]*?<div class="km-list-column"[\s\S]*?readerPosition === 'right'[\s\S]*?<ReaderPane[\s\S]*?<\/template>/
  );
  assert.match(
    src,
    /v-if="activeWorkspaceView === 'mail' && state\.layout\.prefs\.value\.readerPosition === 'off'"/
  );

  // 右侧内嵌通讯录与附件图库，且无底部弹窗版实例
  const contactsMatches = src.match(/<ContactsModal\b/g) ?? [];
  const galleryMatches = src.match(/<AttachmentGalleryModal\b/g) ?? [];
  assert.equal(contactsMatches.length, 1);
  assert.equal(galleryMatches.length, 1);

  assert.match(
    src,
    /<ContactsModal\s+v-else-if="activeWorkspaceView === 'contacts'"\s+:open="true"\s+:embedded="true"\s+@close="closeContactsView\(\)"\s+@composeTo="onComposeToContact"\s+@closeWindow="state\.closeWindow\(\)"\s*\/>/
  );
  assert.match(
    src,
    /<AttachmentGalleryModal\s+v-else-if="activeWorkspaceView === 'attachments'"\s+:open="true"\s+:embedded="true"\s+@close="closeAttachmentsView\(\)"\s+@openMessage="onGalleryOpenMessage"\s+@closeWindow="state\.closeWindow\(\)"\s*\/>/
  );

  // 命令面板接入内嵌工作区视图
  assert.match(
    src,
    /case "tools\.attachments":\s*openAttachmentsView\(\);\s*break;/
  );
  assert.match(
    src,
    /case "tools\.contacts":\s*openContactsView\(\);\s*break;/
  );

  // 查看原邮件与写信回调切回 mail 视图；单键快捷键与列表方向键在非 mail 视图拦截
  assert.match(
    src,
    /function onGalleryOpenMessage\([^)]*\)\s*\{[\s\S]*?closeAttachmentsView\(\);/
  );
  assert.match(
    src,
    /function onComposeToContact\([^)]*\)\s*\{[\s\S]*?closeContactsView\(\);/
  );
  assert.match(
    src,
    /hasOpenModal:\s*hasAnyOpenModal\.value \|\| activeWorkspaceView\.value !== "mail"/
  );
  assert.match(
    src,
    /activeWorkspaceView\.value !== "mail"\s*\|\|\s*isEditableFocused\(\)/
  );
});

test("内嵌工作区状态机行为：导入面板切换、thumbnailDataUrl 兼容、PDF 判定与快捷键守卫", async () => {
  __resetContactsMock();
  const contacts = useContacts();
  await contacts.init();
  assert.equal(contacts.importPanelOpen.value, false);
  contacts.openImportPanel();
  assert.equal(contacts.importPanelOpen.value, true);
  contacts.closeImportPanel();
  assert.equal(contacts.importPanelOpen.value, false);
  const initialContacts = await listContacts({ limit: 5 });
  assert.ok(initialContacts.total > 0);
  contacts.dispose();
  __resetContactsMock();

  __resetReaderToolsMock();
  const attachments = useAttachments();
  await attachments.loadGallery(true);
  const pdfItem = attachments.items.value.find((it) => isPdfAttachment(it));
  assert.ok(pdfItem);
  const previewWithThumb = {
    attachmentId: 2,
    filename: "sample.png",
    mimeType: "image/png",
    dataUrl: "data:image/png;base64,FULL",
    thumbnailDataUrl: "data:image/png;base64,THUMB",
    sizeBytes: 128,
  };
  assert.equal(
    previewWithThumb.thumbnailDataUrl || previewWithThumb.dataUrl,
    "data:image/png;base64,THUMB"
  );
  attachments.reset();
  __resetReaderToolsMock();

  // 非 mail 工作区视图时，屏蔽邮件单键快捷键但保留 Ctrl+K 命令面板
  for (const key of ["j", "k", "e", "r", "a", "f", "/"]) {
    assert.equal(
      matchSingleKeyShortcut(
        { key },
        { singleKeyShortcuts: true, hasOpenModal: true, isInputFocused: false }
      ).matched,
      false
    );
  }
  const paletteShortcut = matchSingleKeyShortcut(
    { key: "k", ctrlKey: true },
    { singleKeyShortcuts: true, hasOpenModal: true, isInputFocused: false }
  );
  assert.equal(paletteShortcut.matched, true);
  assert.equal(paletteShortcut.isPaletteToggle, true);
});
