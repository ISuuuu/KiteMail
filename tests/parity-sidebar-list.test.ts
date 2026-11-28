import test from "node:test";
import assert from "node:assert/strict";

import {
  accountAccentHex,
  buildAccountFolderTree,
  findUnifiedSubrowFolder,
  resolveMessageRowTint,
  resolveTagPillStyle,
  senderDisplayName,
  senderInitials,
} from "../src/main-window/parityHelpers.ts";
import {
  buildMessageListVirtualLayout,
  dateInputToUnixRange,
  resolveSelectedGroupToUnfold,
  useMainWindow,
} from "../src/main-window/useMainWindow.ts";
import {
  listMessages,
  listThreadSummaries,
  mainBootstrap,
  resetMockSession,
} from "../src/ipc/main-window.ts";
import type { FolderSummaryDto, MessageSummaryDto } from "../src/ipc/types.ts";

function makeMsg(overrides: Partial<MessageSummaryDto> = {}): MessageSummaryDto {
  return {
    id: 1,
    accountId: 1,
    folderId: 1,
    folderPath: "INBOX",
    sender: "Marcus Chen <marcus@studio.dev>",
    recipients: "demo@kitemail.example",
    subject: "Q3 roadmap review",
    preview: "Updated doc with parallel tracks.",
    dateTs: 1_760_000_000,
    dateDisplay: "14:32",
    unread: true,
    flagged: false,
    hasAttachment: false,
    threadCount: 1,
    importance: 0,
    tags: [],
    answered: false,
    forwarded: false,
    ...overrides,
  };
}

test("senderInitials 与 accountAccentHex 规则对齐原版头像", () => {
  assert.equal(senderInitials("Marcus Chen"), "MC");
  assert.equal(senderInitials("GNOME Foundation"), "GF");
  assert.equal(senderInitials("Apple"), "A");
  assert.equal(senderInitials("林晓明"), "林");
  assert.equal(senderInitials("Marco (3v1n0)"), "MT".slice(0, 1)); // single word outside parens -> M
  assert.equal(senderInitials("   "), "?");
  assert.equal(senderDisplayName(' "Sophie Turner" <sophie@studio.dev> '), "Sophie Turner");

  assert.equal(accountAccentHex("a@example.com", "#29AE74"), "#29ae74");
  const c1 = accountAccentHex("jason@hylki.hyprlab.co", null);
  const c2 = accountAccentHex("jason@hylki.hyprlab.co", "invalid");
  assert.equal(c1, c2);
  assert.match(c1, /^#[0-9a-f]{6}$/);
});

test("Work/Personal 行背景与标签底色及选中态反白对齐规范", () => {
  assert.equal(resolveMessageRowTint(["Work"], false, false), "#D2E1F5");
  assert.equal(resolveMessageRowTint(["Work"], false, true), "#213046");
  assert.equal(resolveMessageRowTint(["Personal"], false, false), "#D5F0E5");
  assert.equal(resolveMessageRowTint(["Personal"], false, true), "#243F36");
  assert.equal(resolveMessageRowTint(["Work"], true, false), null);

  assert.deepEqual(resolveTagPillStyle("Work", false), {
    backgroundColor: "#3584e4",
    color: "#ffffff",
  });
  assert.deepEqual(resolveTagPillStyle("Personal", false), {
    backgroundColor: "#26a269",
    color: "#ffffff",
  });
  assert.deepEqual(resolveTagPillStyle("Work", true), {
    backgroundColor: "rgba(255, 255, 255, 0.22)",
    color: "#ffffff",
  });
  assert.deepEqual(
    resolveTagPillStyle("自定义", false, [{ id: 9, name: "自定义", color: "#9141ac" }]),
    { backgroundColor: "#9141ac", color: "#ffffff" }
  );
});

test("buildAccountFolderTree 角色排序与多级子文件夹折叠", () => {
  const folders: FolderSummaryDto[] = [
    { id: 10, accountId: 1, name: "Archive", path: "Archive", unreadCount: 0, totalCount: 2, hasMoreRemote: false },
    { id: 11, accountId: 1, name: "Projects", path: "Projects", unreadCount: 1, totalCount: 5, hasMoreRemote: false },
    { id: 12, accountId: 1, name: "Q3", path: "Projects/Q3", unreadCount: 1, totalCount: 3, hasMoreRemote: false },
    { id: 1, accountId: 1, name: "收件箱", path: "INBOX", unreadCount: 4, totalCount: 20, hasMoreRemote: false },
  ];

  const expandedTree = buildAccountFolderTree(folders, new Set());
  assert.deepEqual(
    expandedTree.map((n) => [n.folder.path, n.depth, n.hasChildren]),
    [
      ["INBOX", 0, false],
      ["Archive", 0, false],
      ["Projects", 0, true],
      ["Projects/Q3", 1, false],
    ]
  );

  const collapsedTree = buildAccountFolderTree(folders, new Set(["Projects"]));
  assert.deepEqual(
    collapsedTree.map((n) => n.folder.path),
    ["INBOX", "Archive", "Projects"]
  );
  assert.equal(findUnifiedSubrowFolder("unifiedInbox", 1, folders)?.id, 1);
  assert.equal(findUnifiedSubrowFolder("archive", 1, folders)?.id, 10);
});

test("buildMessageListVirtualLayout 支持日期折叠、会话子行与步长计算", () => {
  const m1 = makeMsg({ id: 1, threadCount: 3 });
  const m2 = makeMsg({ id: 2, threadCount: 1 });
  const m3 = makeMsg({ id: 3, threadCount: 1 });
  const childA = makeMsg({ id: 101, threadCount: 1 });
  const childB = makeMsg({ id: 102, threadCount: 1 });

  const groups = [
    { label: "今天", items: [m1, m2] },
    { label: "昨天", items: [m3] },
  ];

  const expanded = buildMessageListVirtualLayout(groups, {
    rowHeight: 86,
    collapsedDayGroups: new Set(),
    expandedThreadKeys: new Set(["1:1"]),
    threadChildren: { "1:1": [childA, childB] },
  });

  // 2 headers (2*28=56) + 5 rows (m1, childA, childB, m2, m3 -> 5*86=430) = 486
  assert.equal(expanded.totalHeight, 486);
  assert.equal(expanded.items[2].type, "row");
  if (expanded.items[2].type === "row") {
    assert.equal(expanded.items[2].isThreadChild, true);
    assert.equal(expanded.items[2].isLastThreadChild, false);
  }
  if (expanded.items[3].type === "row") {
    assert.equal(expanded.items[3].isThreadChild, true);
    assert.equal(expanded.items[3].isLastThreadChild, true);
  }

  const foldedToday = buildMessageListVirtualLayout(groups, {
    rowHeight: 86,
    collapsedDayGroups: new Set(["今天"]),
    expandedThreadKeys: new Set(["1:1"]),
    threadChildren: { "1:1": [childA, childB] },
  });
  // 2 headers (56) + 1 row m3 (86) = 142
  assert.equal(foldedToday.totalHeight, 142);
  assert.equal(resolveSelectedGroupToUnfold({ accountId: 1, messageId: 2 }, groups, new Set(["今天"])), "今天");
});

test("dateInputToUnixRange 与 Mock Filters/Tags/日期范围/模板/会话子行", async () => {
  const range = dateInputToUnixRange("2026-11-01", "2026-11-02");
  assert.ok(range.dateFromTs !== null && range.dateToTs !== null);
  assert.equal(range.dateToTs! - range.dateFromTs!, 2 * 86400);

  resetMockSession({
    templates: [
      {
        id: 1,
        name: "周报模板",
        toAddr: "team@example.com",
        ccAddr: "",
        bccAddr: "",
        subject: "本周进度汇报",
        bodyText: "大家好，本周完成如下：",
        bodyFormat: "markdown",
        attachmentPaths: [],
        createdAt: 1_760_000_000,
      },
    ],
  });

  const boot = await mainBootstrap();
  assert.ok((boot.tags?.length ?? 0) >= 4);
  assert.equal(boot.templates?.length, 1);
  assert.equal(boot.templates?.[0].name, "周报模板");

  const tagsView = await listMessages({
    requestId: "tags-view",
    view: { kind: "tags" },
    query: "",
    scope: "allFolders",
    sort: "dateNewest",
    filter: {},
    offset: 0,
    limit: 50,
  });
  assert.ok(tagsView.total > 0);
  assert.ok(tagsView.items.every((m) => m.tags.length > 0));

  const singleTag = await listMessages({
    requestId: "tag-work",
    view: { kind: "tags" },
    query: "tag:工作",
    scope: "allFolders",
    sort: "dateNewest",
    filter: {},
    offset: 0,
    limit: 50,
  });
  assert.ok(singleTag.total >= 1);
  assert.ok(singleTag.items.every((m) => m.tags.includes("工作")));

  const nowSec = Math.floor(Date.now() / 1000);
  const recentFiltered = await listMessages({
    requestId: "date-range",
    view: { kind: "unifiedInbox" },
    query: "",
    scope: "thisFolder",
    sort: "dateNewest",
    filter: { dateFromTs: nowSec - 7200, dateToTs: nowSec + 60 },
    offset: 0,
    limit: 50,
  });
  assert.ok(recentFiltered.items.every((m) => m.dateTs >= nowSec - 7200 && m.dateTs < nowSec + 60));

  // 第 4 封 pinned sample threadCount = 3
  const threadRes = await listThreadSummaries({ accountId: 1, messageId: 4 });
  assert.equal(threadRes.items.length, 2);

  const state = useMainWindow();
  await state.init();
  const threadHead = state.messages.value.find((m) => m.threadCount > 1);
  assert.ok(threadHead);
  await state.toggleThreadExpand(threadHead!);
  const key = `${threadHead!.accountId}:${threadHead!.id}`;
  assert.equal(state.expandedThreadKeys.value.has(key), true);
  assert.ok((state.threadChildren.value[key]?.length ?? 0) >= 1);

  await state.toggleThreadExpand(threadHead!);
  assert.equal(state.expandedThreadKeys.value.has(key), false);

  state.dispose();
  resetMockSession();
});
