/**
 * KiteMail 写邮件 / 草稿 / 发件箱 IPC 客户端与可控 Mock 传输层。
 *
 * 参数包装约定（与 Rust 后端完全一致）：
 *   list_drafts()                        无参
 *   save_draft({ input })                包在 input 字段
 *   delete_draft({ input })              包在 input 字段
 *   send_mail({ input })                 包在 input 字段
 *   undo_send_mail({ input })            包在 input 字段
 *   list_outbox()                        无参
 *   retry_outbox_item({ input })         包在 input 字段
 *   delete_outbox_item({ input })        包在 input 字段
 */

import { isTauri } from "../ipc/main-window";
import type { IpcError } from "../ipc/types";
import type {
  DeleteDraftInput,
  DeleteDraftOutput,
  DraftSavedEvent,
  ListDraftsOutput,
  ListOutboxOutput,
  LocalDraftDto,
  OutboxItemActionInput,
  OutboxItemActionOutput,
  OutboxItemDto,
  OutboxUpdatedEvent,
  SaveDraftInput,
  SaveDraftOutput,
  SendMailInput,
  SendMailOutput,
  UndoSendMailInput,
  UndoSendMailOutput,
} from "./types";

export type UnlistenFn = () => void;

async function tauriInvoke<T>(cmd: string, args?: unknown): Promise<T> {
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<T>(cmd, args as Record<string, unknown>);
}

async function tauriListen<T>(
  event: string,
  handler: (payload: T) => void
): Promise<UnlistenFn> {
  const { listen } = await import("@tauri-apps/api/event");
  return listen<T>(event, (ev) => handler(ev.payload));
}

// ---------- Mock 事件总线与状态 ----------

interface MockComposeBus {
  "main://outbox-updated": Array<(e: OutboxUpdatedEvent) => void>;
  "main://draft-saved": Array<(e: DraftSavedEvent) => void>;
}

const mockBus: MockComposeBus = {
  "main://outbox-updated": [],
  "main://draft-saved": [],
};

function emitMock<K extends keyof MockComposeBus>(
  event: K,
  payload: Parameters<MockComposeBus[K][number]>[0]
): void {
  for (const fn of [...mockBus[event]]) {
    fn(payload as never);
  }
}

function mockListen<K extends keyof MockComposeBus>(
  event: K,
  handler: MockComposeBus[K][number]
): UnlistenFn {
  mockBus[event].push(handler as never);
  return () => {
    const idx = mockBus[event].indexOf(handler as never);
    if (idx >= 0) mockBus[event].splice(idx, 1);
  };
}

function makeLockedError(): IpcError {
  return {
    code: "LOCKED",
    message: "应用已锁定，请解锁后再试",
    retryable: false,
  };
}

function makeInvalidInputError(message: string): IpcError {
  return {
    code: "INVALID_INPUT",
    message,
    retryable: false,
  };
}

interface MockUndoEntry {
  outboxId: string;
  accountId: string;
  undoToken: string;
  timer: ReturnType<typeof setTimeout> | null;
}

let mockLocked = false;
let mockStateVersion = 1;
let mockDraftSeq = 1;
let mockOutboxSeq = 1;
let mockDefaultUndoWindowSecs = 5;
let mockAutoCompleteDelayMs = 5000;
let mockFailDeliveryMessage: string | null = null;
let mockFailRetryMessage: string | null = null;
let mockSendError: IpcError | null = null;

let mockDrafts: LocalDraftDto[] = [];
let mockOutbox: OutboxItemDto[] = [];
const mockUndoEntries = new Map<string, MockUndoEntry>();

function clearAllMockTimers(): void {
  for (const entry of mockUndoEntries.values()) {
    if (entry.timer !== null) {
      clearTimeout(entry.timer);
    }
  }
  mockUndoEntries.clear();
}

function unrefTimer(timer: ReturnType<typeof setTimeout>): void {
  if (typeof timer === "object" && timer !== null && "unref" in timer) {
    (timer as { unref: () => void }).unref();
  }
}

function cloneDraft(d: LocalDraftDto): LocalDraftDto {
  return { ...d, attachmentPaths: [...d.attachmentPaths] };
}

function cloneOutboxItem(item: OutboxItemDto): OutboxItemDto {
  return { ...item };
}

function snapshotOutbox(): OutboxItemDto[] {
  return mockOutbox.map(cloneOutboxItem);
}

function scheduleMockDelivery(
  outboxId: string,
  delayMs: number,
  failMessage: string | null = mockFailDeliveryMessage
): void {
  const existing = mockUndoEntries.get(outboxId);
  if (!existing) return;
  if (existing.timer !== null) {
    clearTimeout(existing.timer);
  }
  if (delayMs <= 0) {
    completeMockDelivery(outboxId, failMessage);
    return;
  }
  const timer = setTimeout(() => {
    completeMockDelivery(outboxId, failMessage);
  }, delayMs);
  unrefTimer(timer);
  existing.timer = timer;
}

function completeMockDelivery(
  outboxId: string,
  failMessage: string | null = mockFailDeliveryMessage
): void {
  const entry = mockUndoEntries.get(outboxId);
  if (entry) {
    if (entry.timer !== null) clearTimeout(entry.timer);
    mockUndoEntries.delete(outboxId);
  }
  const idx = mockOutbox.findIndex((item) => item.id === outboxId);
  if (idx < 0) return;

  mockStateVersion += 1;
  if (failMessage) {
    mockOutbox[idx] = {
      ...mockOutbox[idx],
      status: "failed",
      lastError: failMessage,
    };
    emitMock("main://outbox-updated", {
      stateVersion: mockStateVersion,
      items: snapshotOutbox(),
      eventKind: "failed",
      outboxId,
      message: failMessage,
    });
    return;
  }

  mockOutbox.splice(idx, 1);
  emitMock("main://outbox-updated", {
    stateVersion: mockStateVersion,
    items: snapshotOutbox(),
    eventKind: "sent",
    outboxId,
    message: "邮件已发送",
  });
}

// ---------- IPC 命令实现 ----------

export async function listDrafts(): Promise<ListDraftsOutput> {
  if (isTauri()) return tauriInvoke<ListDraftsOutput>("list_drafts");
  if (mockLocked) return Promise.reject(makeLockedError());
  return {
    drafts: [...mockDrafts]
      .sort((a, b) => b.updatedAt - a.updatedAt)
      .map(cloneDraft),
    stateVersion: mockStateVersion,
  };
}

export async function saveDraft(input: SaveDraftInput): Promise<SaveDraftOutput> {
  if (isTauri()) return tauriInvoke<SaveDraftOutput>("save_draft", { input });
  if (mockLocked) return Promise.reject(makeLockedError());

  const now = Math.floor(Date.now() / 1000);
  const existingIdx = input.id
    ? mockDrafts.findIndex((d) => d.id === input.id)
    : -1;

  const draftId =
    existingIdx >= 0
      ? mockDrafts[existingIdx].id
      : input.id && input.id.trim().length > 0
      ? input.id.trim()
      : `draft-${mockDraftSeq++}`;

  const draft: LocalDraftDto = {
    id: draftId,
    accountId: input.accountId,
    toAddr: input.toAddr,
    ccAddr: input.ccAddr ?? "",
    bccAddr: input.bccAddr ?? "",
    subject: input.subject,
    bodyText: input.bodyText,
    inReplyTo: input.inReplyTo ?? null,
    fromAddr: input.fromAddr ?? null,
    replyTo: input.replyTo ?? null,
    bodyFormat: input.bodyFormat ?? "plain",
    attachmentPaths: [...(input.attachmentPaths ?? [])],
    updatedAt: now,
  };

  if (existingIdx >= 0) {
    mockDrafts[existingIdx] = draft;
  } else {
    mockDrafts.unshift(draft);
  }

  mockStateVersion += 1;
  emitMock("main://draft-saved", {
    stateVersion: mockStateVersion,
    draftId: draft.id,
  });

  return {
    draft: cloneDraft(draft),
    stateVersion: mockStateVersion,
  };
}

export async function deleteDraft(input: DeleteDraftInput): Promise<DeleteDraftOutput> {
  if (isTauri()) return tauriInvoke<DeleteDraftOutput>("delete_draft", { input });
  if (mockLocked) return Promise.reject(makeLockedError());

  const idx = mockDrafts.findIndex((d) => d.id === input.draftId);
  const deleted = idx >= 0;
  if (deleted) {
    mockDrafts.splice(idx, 1);
    mockStateVersion += 1;
  }
  return {
    deleted,
    stateVersion: mockStateVersion,
  };
}

export async function sendMail(input: SendMailInput): Promise<SendMailOutput> {
  if (isTauri()) return tauriInvoke<SendMailOutput>("send_mail", { input });
  if (mockLocked) return Promise.reject(makeLockedError());
  if (mockSendError) return Promise.reject({ ...mockSendError });

  if (!input.accountId.trim()) {
    return Promise.reject(makeInvalidInputError("请选择发件账号"));
  }
  if (!input.to.trim()) {
    return Promise.reject(makeInvalidInputError("请填写至少一位收件人"));
  }

  if (input.localDraftId) {
    const draftIdx = mockDrafts.findIndex((d) => d.id === input.localDraftId);
    if (draftIdx >= 0) {
      mockDrafts.splice(draftIdx, 1);
    }
  }

  const now = Math.floor(Date.now() / 1000);
  const undoWindowSecs = Math.max(
    0,
    input.undoDelaySecs ?? mockDefaultUndoWindowSecs
  );
  const scheduledAt = input.scheduledAt ?? null;
  const outboxId = `outbox-${mockOutboxSeq++}`;
  const undoToken = `undo-${outboxId}-${Date.now()}`;

  const item: OutboxItemDto = {
    id: outboxId,
    accountId: input.accountId,
    toAddr: input.to,
    subject: input.subject,
    status: "queued",
    lastError: null,
    sendAt: scheduledAt ?? now + undoWindowSecs,
  };
  mockOutbox.unshift(item);

  mockStateVersion += 1;
  mockUndoEntries.set(outboxId, {
    outboxId,
    accountId: input.accountId,
    undoToken,
    timer: null,
  });

  emitMock("main://outbox-updated", {
    stateVersion: mockStateVersion,
    items: snapshotOutbox(),
    eventKind: "enqueued",
    undoToken,
    outboxId,
    message: scheduledAt ? "已加入定时发送队列" : "邮件已加入发送队列",
  });

  // 定时发送（远期时间戳）在 Mock 中不立即触发送达，方便 UI 与测试检查队列状态
  const isFutureSchedule = scheduledAt !== null && scheduledAt > now + undoWindowSecs + 1;
  if (!isFutureSchedule) {
    const delayMs =
      input.undoDelaySecs !== undefined && input.undoDelaySecs !== null
        ? Math.max(0, input.undoDelaySecs * 1000)
        : mockAutoCompleteDelayMs;
    if (delayMs > 0) {
      scheduleMockDelivery(outboxId, delayMs);
    } else {
      completeMockDelivery(outboxId);
    }
  }

  return {
    accepted: true,
    undoToken,
    outboxId,
    undoWindowSecs,
    scheduledAt,
    stateVersion: mockStateVersion,
  };
}

export async function undoSendMail(
  input: UndoSendMailInput
): Promise<UndoSendMailOutput> {
  if (isTauri()) return tauriInvoke<UndoSendMailOutput>("undo_send_mail", { input });
  if (mockLocked) return Promise.reject(makeLockedError());

  const entry = mockUndoEntries.get(input.outboxId);
  const idx = mockOutbox.findIndex(
    (item) => item.id === input.outboxId && item.accountId === input.accountId
  );

  if (
    !entry ||
    entry.accountId !== input.accountId ||
    entry.undoToken !== input.undoToken ||
    idx < 0
  ) {
    return {
      undone: false,
      message: "撤销窗口已结束或邮件已发出",
      stateVersion: mockStateVersion,
    };
  }

  if (entry.timer !== null) {
    clearTimeout(entry.timer);
  }
  mockUndoEntries.delete(input.outboxId);
  mockOutbox.splice(idx, 1);
  mockStateVersion += 1;

  emitMock("main://outbox-updated", {
    stateVersion: mockStateVersion,
    items: snapshotOutbox(),
    eventKind: "undone",
    outboxId: input.outboxId,
    message: "已撤销发送",
  });

  return {
    undone: true,
    message: "已撤销发送，内容已恢复至编辑区",
    stateVersion: mockStateVersion,
  };
}

export async function listOutbox(): Promise<ListOutboxOutput> {
  if (isTauri()) return tauriInvoke<ListOutboxOutput>("list_outbox");
  if (mockLocked) return Promise.reject(makeLockedError());
  return {
    items: snapshotOutbox(),
    stateVersion: mockStateVersion,
  };
}

export async function retryOutboxItem(
  input: OutboxItemActionInput
): Promise<OutboxItemActionOutput> {
  if (isTauri()) return tauriInvoke<OutboxItemActionOutput>("retry_outbox_item", { input });
  if (mockLocked) return Promise.reject(makeLockedError());

  const idx = mockOutbox.findIndex(
    (item) => item.id === input.outboxId && item.accountId === input.accountId
  );
  if (idx < 0) {
    return Promise.reject({
      code: "OUTBOX_NOT_FOUND",
      message: `未找到发件队列项 ${input.outboxId}`,
      retryable: false,
    } satisfies IpcError);
  }

  mockOutbox[idx] = {
    ...mockOutbox[idx],
    status: "queued",
    lastError: null,
  };
  mockStateVersion += 1;

  emitMock("main://outbox-updated", {
    stateVersion: mockStateVersion,
    items: snapshotOutbox(),
    eventKind: "updated",
    outboxId: input.outboxId,
    message: "已重新加入发送队列",
  });

  mockUndoEntries.set(input.outboxId, {
    outboxId: input.outboxId,
    accountId: input.accountId,
    undoToken: `retry-${Date.now()}`,
    timer: null,
  });
  scheduleMockDelivery(
    input.outboxId,
    Math.min(mockAutoCompleteDelayMs, 40),
    mockFailRetryMessage
  );

  return {
    accepted: true,
    stateVersion: mockStateVersion,
  };
}

export async function deleteOutboxItem(
  input: OutboxItemActionInput
): Promise<OutboxItemActionOutput> {
  if (isTauri()) return tauriInvoke<OutboxItemActionOutput>("delete_outbox_item", { input });
  if (mockLocked) return Promise.reject(makeLockedError());

  const idx = mockOutbox.findIndex(
    (item) => item.id === input.outboxId && item.accountId === input.accountId
  );
  if (idx < 0) {
    return {
      accepted: false,
      stateVersion: mockStateVersion,
    };
  }

  const entry = mockUndoEntries.get(input.outboxId);
  if (entry?.timer) clearTimeout(entry.timer);
  mockUndoEntries.delete(input.outboxId);
  mockOutbox.splice(idx, 1);
  mockStateVersion += 1;

  emitMock("main://outbox-updated", {
    stateVersion: mockStateVersion,
    items: snapshotOutbox(),
    eventKind: "updated",
    outboxId: input.outboxId,
    message: "已移除发件队列项",
  });

  return {
    accepted: true,
    stateVersion: mockStateVersion,
  };
}

// ---------- 事件订阅 ----------

export async function onOutboxUpdated(
  handler: (e: OutboxUpdatedEvent) => void
): Promise<UnlistenFn> {
  if (isTauri()) return tauriListen<OutboxUpdatedEvent>("main://outbox-updated", handler);
  return mockListen("main://outbox-updated", handler);
}

export async function onDraftSaved(
  handler: (e: DraftSavedEvent) => void
): Promise<UnlistenFn> {
  if (isTauri()) return tauriListen<DraftSavedEvent>("main://draft-saved", handler);
  return mockListen("main://draft-saved", handler);
}

// ---------- 测试与预览控制辅助（仅作用于 Mock 传输层） ----------

export function resetMockComposeState(options?: {
  locked?: boolean;
  drafts?: LocalDraftDto[];
  outbox?: OutboxItemDto[];
  defaultUndoWindowSecs?: number;
  autoCompleteDelayMs?: number;
  failDeliveryMessage?: string | null;
  failRetryMessage?: string | null;
  sendError?: IpcError | null;
}): void {
  clearAllMockTimers();
  mockLocked = options?.locked ?? false;
  mockStateVersion = 1;
  mockDraftSeq = 1;
  mockOutboxSeq = 1;
  mockDefaultUndoWindowSecs = options?.defaultUndoWindowSecs ?? 5;
  mockAutoCompleteDelayMs = options?.autoCompleteDelayMs ?? 5000;
  mockFailDeliveryMessage = options?.failDeliveryMessage ?? null;
  mockFailRetryMessage = options?.failRetryMessage ?? null;
  mockSendError = options?.sendError ?? null;
  mockDrafts = (options?.drafts ?? []).map(cloneDraft);
  mockOutbox = (options?.outbox ?? []).map(cloneOutboxItem);
}

export function flushMockOutboxDelivery(outboxId: string): void {
  completeMockDelivery(outboxId);
}

export function emitMockComposeEvent<K extends keyof MockComposeBus>(
  event: K,
  payload: Parameters<MockComposeBus[K][number]>[0]
): void {
  emitMock(event, payload);
}
