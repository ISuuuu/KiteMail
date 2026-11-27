/**
 * KiteMail 写邮件 / 本地草稿 / 发件箱与撤销发送 TypeScript 契约类型定义。
 * 所有命令字段严格使用 camelCase，与 Rust 后端冻结 IPC 契约完全一致。
 */

export type BodyFormat = "plain" | "markdown" | "html";

export type ComposeMode = "new" | "reply" | "replyAll" | "forward";

export type OutboxItemStatus = "queued" | "sending" | "uncertain" | "failed";

export type OutboxEventKind = "enqueued" | "sent" | "failed" | "undone" | "updated";

export interface LocalDraftDto {
  id: string;
  accountId: string;
  toAddr: string;
  ccAddr: string;
  bccAddr: string;
  subject: string;
  bodyText: string;
  inReplyTo: string | null;
  fromAddr: string | null;
  replyTo: string | null;
  bodyFormat: BodyFormat;
  attachmentPaths: string[];
  updatedAt: number;
}

export interface ListDraftsOutput {
  drafts: LocalDraftDto[];
  stateVersion: number;
}

export interface SaveDraftInput {
  id?: string | null;
  accountId: string;
  toAddr: string;
  ccAddr?: string;
  bccAddr?: string;
  subject: string;
  bodyText: string;
  inReplyTo?: string | null;
  fromAddr?: string | null;
  replyTo?: string | null;
  bodyFormat?: BodyFormat;
  attachmentPaths?: string[];
  silent?: boolean;
}

export interface SaveDraftOutput {
  draft: LocalDraftDto;
  stateVersion: number;
}

export interface DeleteDraftInput {
  draftId: string;
}

export interface DeleteDraftOutput {
  deleted: boolean;
  stateVersion: number;
}

export interface SendMailInput {
  accountId: string;
  to: string;
  cc?: string;
  bcc?: string;
  subject: string;
  body: string;
  inReplyTo?: string | null;
  fromAddr?: string | null;
  replyTo?: string | null;
  bodyFormat?: BodyFormat;
  bodyHtml?: string | null;
  attachmentPaths?: string[];
  localDraftId?: string | null;
  undoDelaySecs?: number | null;
  scheduledAt?: number | null;
}

export interface SendMailOutput {
  accepted: boolean;
  undoToken: string;
  outboxId: string;
  undoWindowSecs: number;
  scheduledAt: number | null;
  stateVersion: number;
}

export interface UndoSendMailInput {
  accountId: string;
  outboxId: string;
  undoToken: string;
}

export interface UndoSendMailOutput {
  undone: boolean;
  message: string;
  stateVersion: number;
}

export interface OutboxItemDto {
  id: string;
  accountId: string;
  toAddr: string;
  subject: string;
  status: OutboxItemStatus;
  lastError: string | null;
  sendAt: number;
}

export interface ListOutboxOutput {
  items: OutboxItemDto[];
  stateVersion: number;
}

export interface OutboxItemActionInput {
  accountId: string;
  outboxId: string;
}

export interface OutboxItemActionOutput {
  accepted: boolean;
  stateVersion: number;
}

export interface OutboxUpdatedEvent {
  stateVersion: number;
  items: OutboxItemDto[];
  eventKind: OutboxEventKind;
  undoToken?: string | null;
  outboxId?: string | null;
  message?: string | null;
}

export interface DraftSavedEvent {
  stateVersion: number;
  draftId: string;
}

// ---------- 前端组合与 UI 状态类型 ----------

export interface ComposeAccountOption {
  id: string;
  displayName: string;
  email: string;
  signature?: string;
}

export interface ComposeOriginalMessage {
  accountId?: string | number;
  sender: string;
  recipients: string;
  cc?: string;
  subject: string;
  dateDisplay?: string;
  plainText?: string | null;
  htmlBody?: string | null;
  rfcMessageId?: string | null;
  replyTo?: string | null;
}

export interface ComposeSeed {
  mode: ComposeMode;
  accountId?: string | number | null;
  originalMessage?: ComposeOriginalMessage | null;
  draft?: LocalDraftDto | null;
  draftId?: string | null;
  to?: string;
  cc?: string;
  bcc?: string;
  subject?: string;
  bodyText?: string;
  bodyFormat?: BodyFormat;
  inReplyTo?: string | null;
  fromAddr?: string | null;
  replyTo?: string | null;
  attachmentPaths?: string[];
  scheduledAt?: number | null;
}

export interface ComposeFormState {
  localDraftId: string | null;
  mode: ComposeMode;
  accountId: string;
  toAddr: string;
  ccAddr: string;
  bccAddr: string;
  subject: string;
  bodyText: string;
  bodyFormat: BodyFormat;
  inReplyTo: string | null;
  fromAddr: string | null;
  replyTo: string | null;
  attachmentPaths: string[];
  scheduledAt: number | null;
  showCcBcc: boolean;
}

export interface ComposeFormSnapshot {
  localDraftId: string | null;
  mode: ComposeMode;
  accountId: string;
  toAddr: string;
  ccAddr: string;
  bccAddr: string;
  subject: string;
  bodyText: string;
  bodyFormat: BodyFormat;
  inReplyTo: string | null;
  fromAddr: string | null;
  replyTo: string | null;
  attachmentPaths: string[];
  scheduledAt: number | null;
  showCcBcc: boolean;
}

export interface UndoBannerState {
  outboxId: string;
  accountId: string;
  undoToken: string;
  to: string;
  subject: string;
  remainingSecs: number;
  totalSecs: number;
  scheduledAt: number | null;
  snapshot: ComposeFormSnapshot;
}

export interface MissingAttachmentCheckResult {
  shouldWarn: boolean;
  matchedKeywords: string[];
}
