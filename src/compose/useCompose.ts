import {
  computed,
  getCurrentInstance,
  onMounted,
  onUnmounted,
  ref,
  type ComputedRef,
  type Ref,
} from "vue";
import { extractErrorMessage, isLockedError } from "../ipc/main-window";
import {
  deleteDraft,
  deleteOutboxItem,
  listDrafts,
  listOutbox,
  onDraftSaved,
  onOutboxUpdated,
  retryOutboxItem,
  saveDraft,
  sendMail,
  undoSendMail,
  type UnlistenFn,
} from "./ipc";
import type {
  BodyFormat,
  ComposeAccountOption,
  ComposeFormSnapshot,
  ComposeFormState,
  ComposeSeed,
  LocalDraftDto,
  MissingAttachmentCheckResult,
  OutboxItemDto,
  SendMailInput,
  SendMailOutput,
  UndoBannerState,
} from "./types";

export const AUTO_SAVE_DEBOUNCE_MS = 1500;

export const DEFAULT_ATTACHMENT_REMINDER_KEYWORDS: readonly string[] = [
  "附件",
  "附上",
  "attached",
  "attachment",
  "attachments",
  "enclosed",
];

// ---------- 纯函数工具（供组件与无 DOM 单测共用） ----------

/** 按逗号/分号拆分收件人地址列表，尊重引号内逗号 */
export function splitAddressList(raw: string): string[] {
  const result: string[] = [];
  let current = "";
  let inQuotes = false;
  let angleDepth = 0;

  for (let i = 0; i < raw.length; i++) {
    const ch = raw[i];
    if (ch === '"' && raw[i - 1] !== "\\") {
      inQuotes = !inQuotes;
      current += ch;
      continue;
    }
    if (!inQuotes) {
      if (ch === "<") angleDepth += 1;
      else if (ch === ">" && angleDepth > 0) angleDepth -= 1;
      else if ((ch === "," || ch === ";") && angleDepth === 0) {
        const trimmed = current.trim();
        if (trimmed) result.push(trimmed);
        current = "";
        continue;
      }
    }
    current += ch;
  }

  const tail = current.trim();
  if (tail) result.push(tail);
  return result;
}

/** 从显示名+地址字符串提取规范化小写邮箱 */
export function extractEmailAddress(raw: string): string {
  const trimmed = raw.trim();
  if (!trimmed) return "";
  const angleMatch = trimmed.match(/<([^<>@\s]+@[^<>@\s]+)>/);
  if (angleMatch) {
    return angleMatch[1].trim().toLowerCase();
  }
  const plainMatch = trimmed.match(/([^\s<>,;"']+@[^\s<>,;"']+)/);
  return plainMatch ? plainMatch[1].trim().toLowerCase() : trimmed.toLowerCase();
}

/** 合并并去重地址列表，同时过滤自身邮箱与 excludedEmails */
export function mergeUniqueAddresses(
  sources: readonly string[],
  excludedEmails: ReadonlySet<string> = new Set()
): string[] {
  const seen = new Set<string>();
  const out: string[] = [];

  for (const src of sources) {
    for (const entry of splitAddressList(src)) {
      const email = extractEmailAddress(entry);
      if (!email || excludedEmails.has(email) || seen.has(email)) {
        continue;
      }
      seen.add(email);
      out.push(entry);
    }
  }
  return out;
}

/** 规范化回复/转发主题前缀，避免产生 Re: Re: 或 Fwd: Fwd: */
export function normalizeSubjectPrefix(subject: string, prefix: "Re:" | "Fwd:"): string {
  const trimmed = subject.trim();
  if (prefix === "Re:") {
    if (/^(re|回复)\s*[:：]/i.test(trimmed)) return trimmed;
    return trimmed ? `Re: ${trimmed}` : "Re:";
  }
  if (/^(fwd?|转发)\s*[:：]/i.test(trimmed)) return trimmed;
  return trimmed ? `Fwd: ${trimmed}` : "Fwd:";
}

/** 简易 HTML 转文本（用于回复/转发纯 HTML 原邮件时的引用） */
export function stripHtmlToText(html: string): string {
  return html
    .replace(/<style[\s\S]*?<\/style>/gi, "")
    .replace(/<script[\s\S]*?<\/script>/gi, "")
    .replace(/<br\s*\/?>/gi, "\n")
    .replace(/<\/(p|div|li|tr|h[1-6]|blockquote)>/gi, "\n")
    .replace(/<[^>]+>/g, "")
    .replace(/&nbsp;/gi, " ")
    .replace(/&lt;/gi, "<")
    .replace(/&gt;/gi, ">")
    .replace(/&amp;/gi, "&")
    .replace(/&quot;/gi, '"')
    .replace(/\n{3,}/g, "\n\n")
    .trim();
}

function escapeHtml(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

function formatInlineMarkdown(escapedLine: string): string {
  return escapedLine
    .replace(/`([^`]+)`/g, "<code>$1</code>")
    .replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>")
    .replace(/\*([^*]+)\*/g, "<em>$1</em>")
    .replace(/\[([^\]]+)\]\((https?:\/\/[^\s)]+)\)/g, '<a href="$2" target="_blank" rel="noopener noreferrer">$1</a>');
}

/** 零依赖轻量 Markdown 转安全 HTML（先转义再渲染常用语法） */
export function renderSimpleMarkdownToHtml(markdown: string): string {
  const lines = markdown.replace(/\r\n/g, "\n").split("\n");
  const blocks: string[] = [];
  let inCodeBlock = false;
  let codeLines: string[] = [];
  let listItems: string[] = [];

  const flushList = () => {
    if (listItems.length > 0) {
      blocks.push(`<ul>${listItems.map((li) => `<li>${li}</li>`).join("")}</ul>`);
      listItems = [];
    }
  };

  for (const line of lines) {
    if (line.trim().startsWith("```")) {
      flushList();
      if (!inCodeBlock) {
        inCodeBlock = true;
        codeLines = [];
      } else {
        blocks.push(`<pre><code>${escapeHtml(codeLines.join("\n"))}</code></pre>`);
        inCodeBlock = false;
        codeLines = [];
      }
      continue;
    }

    if (inCodeBlock) {
      codeLines.push(line);
      continue;
    }

    const listMatch = line.match(/^\s*[-*]\s+(.+)$/);
    if (listMatch) {
      listItems.push(formatInlineMarkdown(escapeHtml(listMatch[1])));
      continue;
    }
    flushList();

    const headingMatch = line.match(/^(#{1,3})\s+(.+)$/);
    if (headingMatch) {
      const level = headingMatch[1].length;
      blocks.push(`<h${level}>${formatInlineMarkdown(escapeHtml(headingMatch[2]))}</h${level}>`);
      continue;
    }

    const quoteMatch = line.match(/^>\s?(.*)$/);
    if (quoteMatch) {
      blocks.push(`<blockquote>${formatInlineMarkdown(escapeHtml(quoteMatch[1]))}</blockquote>`);
      continue;
    }

    if (!line.trim()) {
      continue;
    }

    blocks.push(`<p>${formatInlineMarkdown(escapeHtml(line))}</p>`);
  }

  flushList();
  if (inCodeBlock && codeLines.length > 0) {
    blocks.push(`<pre><code>${escapeHtml(codeLines.join("\n"))}</code></pre>`);
  }

  return blocks.join("\n");
}

function quoteLines(text: string): string {
  if (!text.trim()) return ">";
  return text
    .replace(/\r\n/g, "\n")
    .split("\n")
    .map((line) => (line.length > 0 ? `> ${line}` : ">"))
    .join("\n");
}

/** 从 ComposeSeed 智能构建写信表单预填状态（支持新建、回复、全部回复、转发与草稿加载） */
export function buildReplyPrefill(
  seed: ComposeSeed,
  accounts: readonly ComposeAccountOption[] = []
): ComposeFormState {
  if (seed.draft) {
    const d = seed.draft;
    return {
      localDraftId: d.id,
      mode: seed.mode ?? "new",
      accountId: d.accountId || accounts[0]?.id || "1",
      toAddr: d.toAddr,
      ccAddr: d.ccAddr,
      bccAddr: d.bccAddr,
      subject: d.subject,
      bodyText: d.bodyText,
      bodyFormat: d.bodyFormat,
      inReplyTo: d.inReplyTo,
      fromAddr: d.fromAddr,
      replyTo: d.replyTo,
      attachmentPaths: [...d.attachmentPaths],
      scheduledAt: seed.scheduledAt ?? null,
      showCcBcc: Boolean(d.ccAddr.trim() || d.bccAddr.trim()),
    };
  }

  const rawAccountId = seed.accountId ?? seed.originalMessage?.accountId ?? accounts[0]?.id ?? "1";
  const accountId = String(rawAccountId);
  const activeAccount =
    accounts.find((a) => a.id === accountId) ?? accounts[0] ?? null;

  const selfEmails = new Set<string>();
  if (activeAccount?.email) {
    selfEmails.add(activeAccount.email.trim().toLowerCase());
  }
  if (seed.fromAddr) {
    const fromEmail = extractEmailAddress(seed.fromAddr);
    if (fromEmail) selfEmails.add(fromEmail);
  }

  const fromAddr =
    seed.fromAddr ??
    (activeAccount
      ? activeAccount.displayName
        ? `${activeAccount.displayName} <${activeAccount.email}>`
        : activeAccount.email
      : null);

  const sigBlock =
    activeAccount?.signature && activeAccount.signature.trim().length > 0
      ? `\n\n-- \n${activeAccount.signature.trim()}`
      : "";

  const orig = seed.originalMessage;
  if (!orig || seed.mode === "new") {
    const ccAddr = seed.cc ?? "";
    const bccAddr = seed.bcc ?? "";
    const bodyBase = seed.bodyText ?? "";
    return {
      localDraftId: seed.draftId ?? null,
      mode: seed.mode ?? "new",
      accountId,
      toAddr: seed.to ?? "",
      ccAddr,
      bccAddr,
      subject: seed.subject ?? "",
      bodyText: bodyBase || (sigBlock ? sigBlock : ""),
      bodyFormat: seed.bodyFormat ?? "plain",
      inReplyTo: seed.inReplyTo ?? null,
      fromAddr,
      replyTo: seed.replyTo ?? null,
      attachmentPaths: [...(seed.attachmentPaths ?? [])],
      scheduledAt: seed.scheduledAt ?? null,
      showCcBcc: Boolean(ccAddr.trim() || bccAddr.trim()),
    };
  }

  const rawOrigBody =
    orig.plainText && orig.plainText.trim().length > 0
      ? orig.plainText.trim()
      : orig.htmlBody
      ? stripHtmlToText(orig.htmlBody)
      : "";

  if (seed.mode === "reply") {
    const replyTarget = orig.replyTo?.trim() || orig.sender;
    const toAddr = seed.to ?? replyTarget;
    const ccAddr = seed.cc ?? "";
    const bccAddr = seed.bcc ?? "";
    const header = [
      "--- 原始邮件 ---",
      `发件人: ${orig.sender}`,
      orig.dateDisplay ? `时间: ${orig.dateDisplay}` : null,
      `主题: ${orig.subject}`,
    ]
      .filter(Boolean)
      .join("\n");
    const quoted = `\n\n${sigBlock ? `${sigBlock.trimStart()}\n\n` : ""}${header}\n${quoteLines(rawOrigBody)}`;

    return {
      localDraftId: seed.draftId ?? null,
      mode: "reply",
      accountId,
      toAddr,
      ccAddr,
      bccAddr,
      subject: seed.subject ?? normalizeSubjectPrefix(orig.subject, "Re:"),
      bodyText: seed.bodyText ?? quoted,
      bodyFormat: seed.bodyFormat ?? "plain",
      inReplyTo: seed.inReplyTo ?? orig.rfcMessageId ?? null,
      fromAddr,
      replyTo: seed.replyTo ?? null,
      attachmentPaths: [...(seed.attachmentPaths ?? [])],
      scheduledAt: seed.scheduledAt ?? null,
      showCcBcc: Boolean(ccAddr.trim() || bccAddr.trim()),
    };
  }

  if (seed.mode === "replyAll") {
    const primaryTarget = orig.replyTo?.trim() || orig.sender;
    const toList = mergeUniqueAddresses([primaryTarget, orig.recipients], selfEmails);
    // 若过滤自身后收件人为空（例如回复自己发出的邮件），保留原 recipients
    const finalToList =
      toList.length > 0
        ? toList
        : mergeUniqueAddresses([orig.recipients, primaryTarget]);

    const toEmails = new Set(finalToList.map(extractEmailAddress));
    const ccExcluded = new Set<string>([...selfEmails, ...toEmails]);
    const ccList = mergeUniqueAddresses([orig.cc ?? ""], ccExcluded);

    const toAddr = seed.to ?? finalToList.join(", ");
    const ccAddr = seed.cc ?? ccList.join(", ");
    const bccAddr = seed.bcc ?? "";
    const header = [
      "--- 原始邮件 ---",
      `发件人: ${orig.sender}`,
      orig.dateDisplay ? `时间: ${orig.dateDisplay}` : null,
      `收件人: ${orig.recipients}`,
      orig.cc ? `抄送: ${orig.cc}` : null,
      `主题: ${orig.subject}`,
    ]
      .filter(Boolean)
      .join("\n");
    const quoted = `\n\n${sigBlock ? `${sigBlock.trimStart()}\n\n` : ""}${header}\n${quoteLines(rawOrigBody)}`;

    return {
      localDraftId: seed.draftId ?? null,
      mode: "replyAll",
      accountId,
      toAddr,
      ccAddr,
      bccAddr,
      subject: seed.subject ?? normalizeSubjectPrefix(orig.subject, "Re:"),
      bodyText: seed.bodyText ?? quoted,
      bodyFormat: seed.bodyFormat ?? "plain",
      inReplyTo: seed.inReplyTo ?? orig.rfcMessageId ?? null,
      fromAddr,
      replyTo: seed.replyTo ?? null,
      attachmentPaths: [...(seed.attachmentPaths ?? [])],
      scheduledAt: seed.scheduledAt ?? null,
      showCcBcc: Boolean(ccAddr.trim() || bccAddr.trim()),
    };
  }

  // forward
  const fwdHeader = [
    "---------- 转发的邮件 ----------",
    `发件人: ${orig.sender}`,
    orig.dateDisplay ? `时间: ${orig.dateDisplay}` : null,
    `主题: ${orig.subject}`,
    `收件人: ${orig.recipients}`,
    orig.cc ? `抄送: ${orig.cc}` : null,
  ]
    .filter(Boolean)
    .join("\n");
  const fwdBody = `\n\n${sigBlock ? `${sigBlock.trimStart()}\n\n` : ""}${fwdHeader}\n\n${rawOrigBody}`;
  const ccAddr = seed.cc ?? "";
  const bccAddr = seed.bcc ?? "";

  return {
    localDraftId: seed.draftId ?? null,
    mode: "forward",
    accountId,
    toAddr: seed.to ?? "",
    ccAddr,
    bccAddr,
    subject: seed.subject ?? normalizeSubjectPrefix(orig.subject, "Fwd:"),
    bodyText: seed.bodyText ?? fwdBody,
    bodyFormat: seed.bodyFormat ?? "plain",
    inReplyTo: seed.inReplyTo ?? null,
    fromAddr,
    replyTo: seed.replyTo ?? null,
    attachmentPaths: [...(seed.attachmentPaths ?? [])],
    scheduledAt: seed.scheduledAt ?? null,
    showCcBcc: Boolean(ccAddr.trim() || bccAddr.trim()),
  };
}

/** 附件关键词忘带提醒检测（忽略 `>` 引用行与空列表以外情形） */
export function detectMissingAttachmentReminder(
  subject: string,
  body: string,
  attachmentPaths: readonly string[],
  keywords: readonly string[] = DEFAULT_ATTACHMENT_REMINDER_KEYWORDS
): MissingAttachmentCheckResult {
  if (attachmentPaths.length > 0) {
    return { shouldWarn: false, matchedKeywords: [] };
  }

  const nonQuotedBody = body
    .replace(/\r\n/g, "\n")
    .split("\n")
    .filter((line) => !/^\s*>/.test(line))
    .join("\n");

  const haystack = `${subject}\n${nonQuotedBody}`;
  const matched: string[] = [];

  for (const kw of keywords) {
    const trimmed = kw.trim();
    if (!trimmed) continue;
    const isAsciiWord = /^[a-z0-9_-]+$/i.test(trimmed);
    const regex = isAsciiWord
      ? new RegExp(`\\b${trimmed.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}\\b`, "i")
      : new RegExp(trimmed.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"), "i");
    if (regex.test(haystack)) {
      matched.push(trimmed);
    }
  }

  return {
    shouldWarn: matched.length > 0,
    matchedKeywords: matched,
  };
}

export function buildSendBodyPayload(
  bodyText: string,
  bodyFormat: BodyFormat
): { body: string; bodyFormat: BodyFormat; bodyHtml: string | null } {
  if (bodyFormat === "markdown") {
    return {
      body: bodyText,
      bodyFormat,
      bodyHtml: renderSimpleMarkdownToHtml(bodyText),
    };
  }
  if (bodyFormat === "html") {
    return {
      body: stripHtmlToText(bodyText),
      bodyFormat,
      bodyHtml: bodyText,
    };
  }
  return {
    body: bodyText,
    bodyFormat: "plain",
    bodyHtml: null,
  };
}

function createEmptyForm(accountId = "1", fromAddr: string | null = null): ComposeFormState {
  return {
    localDraftId: null,
    mode: "new",
    accountId,
    toAddr: "",
    ccAddr: "",
    bccAddr: "",
    subject: "",
    bodyText: "",
    bodyFormat: "plain",
    inReplyTo: null,
    fromAddr,
    replyTo: null,
    attachmentPaths: [],
    scheduledAt: null,
    showCcBcc: false,
  };
}

function snapshotForm(form: ComposeFormState): ComposeFormSnapshot {
  return {
    ...form,
    attachmentPaths: [...form.attachmentPaths],
  };
}

function hasMeaningfulDraftContent(form: ComposeFormState): boolean {
  return Boolean(
    form.toAddr.trim() ||
      form.ccAddr.trim() ||
      form.bccAddr.trim() ||
      form.subject.trim() ||
      form.bodyText.trim() ||
      form.attachmentPaths.length > 0
  );
}

function unrefTimer(timer: ReturnType<typeof setTimeout> | ReturnType<typeof setInterval>): void {
  if (typeof timer === "object" && timer !== null && "unref" in timer) {
    (timer as { unref: () => void }).unref();
  }
}

export interface UseComposeOptions {
  accounts?: readonly ComposeAccountOption[];
  autoSaveDebounceMs?: number;
  onLocked?: () => void;
}

export interface SendComposeOptions {
  ignoreAttachmentWarning?: boolean;
  undoDelaySecs?: number | null;
}

export type SendComposeResult =
  | { sent: true; output: SendMailOutput }
  | { sent: false; needsAttachmentConfirm?: boolean; error?: string };

export interface UseComposeReturn {
  open: Ref<boolean>;
  accounts: Ref<ComposeAccountOption[]>;
  form: Ref<ComposeFormState>;
  dirty: Ref<boolean>;
  savingDraft: Ref<boolean>;
  lastSavedAt: Ref<number | null>;
  drafts: Ref<LocalDraftDto[]>;
  outboxItems: Ref<OutboxItemDto[]>;
  sending: Ref<boolean>;
  undoing: Ref<boolean>;
  busyOutboxId: Ref<string | null>;
  error: Ref<string | null>;
  statusNotice: Ref<string | null>;
  attachmentWarningIgnored: Ref<boolean>;
  missingAttachmentCheck: ComputedRef<MissingAttachmentCheckResult>;
  undoBanner: Ref<UndoBannerState | null>;
  stateVersion: Ref<number>;
  setAccounts: (next: readonly ComposeAccountOption[]) => void;
  init: () => Promise<void>;
  openCompose: (seed?: ComposeSeed) => void;
  openDraft: (draft: LocalDraftDto) => void;
  updateField: <K extends keyof ComposeFormState>(key: K, value: ComposeFormState[K]) => void;
  addAttachmentPaths: (paths: readonly string[]) => void;
  removeAttachmentAt: (index: number) => void;
  ignoreAttachmentWarning: () => void;
  saveDraftNow: (silent?: boolean) => Promise<LocalDraftDto | null>;
  discardDraft: () => Promise<void>;
  closeWithAutoSave: () => Promise<void>;
  send: (options?: SendComposeOptions) => Promise<SendComposeResult>;
  undoSend: () => Promise<boolean>;
  dismissUndoBanner: () => void;
  refreshDrafts: () => Promise<void>;
  deleteDraftById: (draftId: string) => Promise<void>;
  refreshOutbox: () => Promise<void>;
  retryOutbox: (accountId: string, outboxId: string) => Promise<boolean>;
  deleteOutbox: (accountId: string, outboxId: string) => Promise<boolean>;
  dispose: () => void;
}

export function useCompose(options: UseComposeOptions = {}): UseComposeReturn {
  const debounceMs = Math.max(0, options.autoSaveDebounceMs ?? AUTO_SAVE_DEBOUNCE_MS);
  const open = ref(false);
  const accounts = ref<ComposeAccountOption[]>([...(options.accounts ?? [])]);
  const form = ref<ComposeFormState>(
    createEmptyForm(
      accounts.value[0]?.id ?? "1",
      accounts.value[0]
        ? `${accounts.value[0].displayName} <${accounts.value[0].email}>`
        : null
    )
  );

  const dirty = ref(false);
  const savingDraft = ref(false);
  const lastSavedAt = ref<number | null>(null);
  const drafts = ref<LocalDraftDto[]>([]);
  const outboxItems = ref<OutboxItemDto[]>([]);
  const sending = ref(false);
  const undoing = ref(false);
  const busyOutboxId = ref<string | null>(null);
  const error = ref<string | null>(null);
  const statusNotice = ref<string | null>(null);
  const attachmentWarningIgnored = ref(false);
  const undoBanner = ref<UndoBannerState | null>(null);
  const stateVersion = ref(0);

  let formRevision = 0;
  let pendingSendRequestId: string | null = null;
  let pendingSendRevision: number | null = null;
  let autoSaveTimer: ReturnType<typeof setTimeout> | null = null;
  let undoTickTimer: ReturnType<typeof setInterval> | null = null;
  let initialized = false;
  let disposed = false;
  const unlistenFns: UnlistenFn[] = [];

  const missingAttachmentCheck = computed<MissingAttachmentCheckResult>(() => {
    if (attachmentWarningIgnored.value) {
      return { shouldWarn: false, matchedKeywords: [] };
    }
    return detectMissingAttachmentReminder(
      form.value.subject,
      form.value.bodyText,
      form.value.attachmentPaths
    );
  });

  function clearAutoSaveTimer(): void {
    if (autoSaveTimer !== null) {
      clearTimeout(autoSaveTimer);
      autoSaveTimer = null;
    }
  }

  function clearUndoTickTimer(): void {
    if (undoTickTimer !== null) {
      clearInterval(undoTickTimer);
      undoTickTimer = null;
    }
  }

  function handleIpcFailure(err: unknown): string {
    if (isLockedError(err)) {
      options.onLocked?.();
    }
    const msg = extractErrorMessage(err);
    error.value = msg;
    return msg;
  }

  function scheduleAutoSave(): void {
    clearAutoSaveTimer();
    if (!open.value || !dirty.value || !hasMeaningfulDraftContent(form.value)) {
      return;
    }
    autoSaveTimer = setTimeout(() => {
      autoSaveTimer = null;
      void saveDraftNow(true);
    }, debounceMs);
    unrefTimer(autoSaveTimer);
  }

  function setAccounts(next: readonly ComposeAccountOption[]): void {
    accounts.value = [...next];
    if (
      accounts.value.length > 0 &&
      !accounts.value.some((a) => a.id === form.value.accountId)
    ) {
      const first = accounts.value[0];
      form.value.accountId = first.id;
      form.value.fromAddr = first.displayName
        ? `${first.displayName} <${first.email}>`
        : first.email;
    }
  }

  async function refreshDrafts(): Promise<void> {
    try {
      const res = await listDrafts();
      drafts.value = res.drafts;
      stateVersion.value = Math.max(stateVersion.value, res.stateVersion);
    } catch (err) {
      handleIpcFailure(err);
    }
  }

  async function refreshOutbox(): Promise<void> {
    try {
      const res = await listOutbox();
      outboxItems.value = res.items;
      stateVersion.value = Math.max(stateVersion.value, res.stateVersion);
    } catch (err) {
      handleIpcFailure(err);
    }
  }

  async function init(): Promise<void> {
    if (initialized || disposed) return;
    initialized = true;

    const u1 = await onOutboxUpdated((ev) => {
      if (ev.stateVersion < stateVersion.value) return;
      stateVersion.value = ev.stateVersion;
      outboxItems.value = ev.items;

      if (undoBanner.value && ev.outboxId === undoBanner.value.outboxId) {
        if (ev.eventKind === "sent" || ev.eventKind === "undone") {
          clearUndoTickTimer();
          undoBanner.value = null;
        } else if (ev.eventKind === "failed") {
          clearUndoTickTimer();
          undoBanner.value = null;
          error.value = ev.message ?? "邮件发送失败，请在发件队列中重试";
        }
      }
    });

    const u2 = await onDraftSaved((ev) => {
      stateVersion.value = Math.max(stateVersion.value, ev.stateVersion);
      if (!savingDraft.value) {
        void refreshDrafts();
      }
    });

    unlistenFns.push(u1, u2);
    await Promise.all([refreshDrafts(), refreshOutbox()]);
  }

  function openCompose(seed: ComposeSeed = { mode: "new" }): void {
    clearAutoSaveTimer();
    error.value = null;
    statusNotice.value = null;
    attachmentWarningIgnored.value = false;
    form.value = buildReplyPrefill(seed, accounts.value);
    dirty.value = Boolean(seed.draftId || seed.to || seed.subject || seed.bodyText);
    lastSavedAt.value = seed.draft?.updatedAt ?? null;
    formRevision += 1;
    open.value = true;
    if (dirty.value) {
      scheduleAutoSave();
    }
  }

  function openDraft(draft: LocalDraftDto): void {
    openCompose({ mode: "new", draft });
    dirty.value = false;
  }

  function updateField<K extends keyof ComposeFormState>(
    key: K,
    value: ComposeFormState[K]
  ): void {
    form.value = {
      ...form.value,
      [key]: value,
    };
    if (key === "accountId") {
      const acc = accounts.value.find((a) => a.id === value);
      if (acc) {
        form.value.fromAddr = acc.displayName
          ? `${acc.displayName} <${acc.email}>`
          : acc.email;
      }
    }
    if (key === "subject" || key === "bodyText" || key === "attachmentPaths") {
      attachmentWarningIgnored.value = false;
    }
    error.value = null;
    dirty.value = true;
    formRevision += 1;
    scheduleAutoSave();
  }

  function addAttachmentPaths(paths: readonly string[]): void {
    const merged = [...form.value.attachmentPaths];
    for (const raw of paths) {
      const trimmed = raw.trim();
      if (trimmed && !merged.includes(trimmed)) {
        merged.push(trimmed);
      }
    }
    updateField("attachmentPaths", merged);
  }

  function removeAttachmentAt(index: number): void {
    if (index < 0 || index >= form.value.attachmentPaths.length) return;
    const next = form.value.attachmentPaths.filter((_, i) => i !== index);
    updateField("attachmentPaths", next);
  }

  function ignoreAttachmentWarning(): void {
    attachmentWarningIgnored.value = true;
  }

  async function saveDraftNow(silent = false): Promise<LocalDraftDto | null> {
    clearAutoSaveTimer();
    if (!hasMeaningfulDraftContent(form.value) && !form.value.localDraftId) {
      dirty.value = false;
      return null;
    }

    const revisionAtStart = formRevision;
    savingDraft.value = true;
    if (!silent) error.value = null;

    try {
      const res = await saveDraft({
        id: form.value.localDraftId,
        accountId: form.value.accountId,
        toAddr: form.value.toAddr,
        ccAddr: form.value.ccAddr,
        bccAddr: form.value.bccAddr,
        subject: form.value.subject,
        bodyText: form.value.bodyText,
        inReplyTo: form.value.inReplyTo,
        fromAddr: form.value.fromAddr,
        replyTo: form.value.replyTo,
        bodyFormat: form.value.bodyFormat,
        attachmentPaths: [...form.value.attachmentPaths],
        silent,
      });

      stateVersion.value = Math.max(stateVersion.value, res.stateVersion);
      form.value.localDraftId = res.draft.id;
      lastSavedAt.value = res.draft.updatedAt;

      const idx = drafts.value.findIndex((d) => d.id === res.draft.id);
      if (idx >= 0) {
        drafts.value[idx] = res.draft;
      } else {
        drafts.value = [res.draft, ...drafts.value];
      }

      if (formRevision === revisionAtStart) {
        dirty.value = false;
      }
      if (!silent) {
        statusNotice.value = "草稿已保存";
      }
      return res.draft;
    } catch (err) {
      if (!silent || isLockedError(err)) {
        handleIpcFailure(err);
      }
      return null;
    } finally {
      savingDraft.value = false;
    }
  }

  async function deleteDraftById(draftId: string): Promise<void> {
    try {
      const res = await deleteDraft({ draftId });
      stateVersion.value = Math.max(stateVersion.value, res.stateVersion);
      if (res.deleted) {
        drafts.value = drafts.value.filter((d) => d.id !== draftId);
        if (form.value.localDraftId === draftId) {
          form.value.localDraftId = null;
        }
      }
    } catch (err) {
      handleIpcFailure(err);
    }
  }

  async function discardDraft(): Promise<void> {
    clearAutoSaveTimer();
    const id = form.value.localDraftId;
    if (id) {
      await deleteDraftById(id);
    }
    const acc = accounts.value.find((a) => a.id === form.value.accountId) ?? accounts.value[0];
    form.value = createEmptyForm(
      acc?.id ?? "1",
      acc ? `${acc.displayName} <${acc.email}>` : null
    );
    dirty.value = false;
    lastSavedAt.value = null;
    error.value = null;
    statusNotice.value = null;
    open.value = false;
  }

  async function closeWithAutoSave(): Promise<void> {
    clearAutoSaveTimer();
    if (dirty.value && hasMeaningfulDraftContent(form.value)) {
      await saveDraftNow(true);
    }
    open.value = false;
  }

  function startUndoCountdown(banner: UndoBannerState): void {
    clearUndoTickTimer();
    undoBanner.value = banner;
    if (banner.remainingSecs <= 0) return;

    undoTickTimer = setInterval(() => {
      if (!undoBanner.value) {
        clearUndoTickTimer();
        return;
      }
      const nextSecs = undoBanner.value.remainingSecs - 1;
      if (nextSecs <= 0) {
        clearUndoTickTimer();
        undoBanner.value = null;
      } else {
        undoBanner.value = {
          ...undoBanner.value,
          remainingSecs: nextSecs,
        };
      }
    }, 1000);
    unrefTimer(undoTickTimer);
  }

  async function send(sendOpts: SendComposeOptions = {}): Promise<SendComposeResult> {
    error.value = null;
    statusNotice.value = null;

    if (!form.value.accountId.trim()) {
      const msg = "请选择发件账号";
      error.value = msg;
      return { sent: false, error: msg };
    }
    if (!form.value.toAddr.trim()) {
      const msg = "请填写至少一位收件人";
      error.value = msg;
      return { sent: false, error: msg };
    }

    if (
      !sendOpts.ignoreAttachmentWarning &&
      !attachmentWarningIgnored.value &&
      missingAttachmentCheck.value.shouldWarn
    ) {
      return { sent: false, needsAttachmentConfirm: true };
    }

    clearAutoSaveTimer();
    sending.value = true;

    // 幂等键：同一份编辑内容（formRevision 未变）的重试复用同一个 id，
    // 编辑内容或切换会话后再发则重新生成，避免回执超时重试生成第二条发件记录。
    if (pendingSendRequestId === null || pendingSendRevision !== formRevision) {
      pendingSendRequestId = crypto.randomUUID();
      pendingSendRevision = formRevision;
    }

    const currentSnapshot = snapshotForm(form.value);
    const { body, bodyFormat, bodyHtml } = buildSendBodyPayload(
      form.value.bodyText,
      form.value.bodyFormat
    );

    const payload: SendMailInput = {
      accountId: form.value.accountId,
      to: form.value.toAddr.trim(),
      cc: form.value.ccAddr.trim() || undefined,
      bcc: form.value.bccAddr.trim() || undefined,
      subject: form.value.subject,
      body,
      inReplyTo: form.value.inReplyTo,
      fromAddr: form.value.fromAddr,
      replyTo: form.value.replyTo,
      bodyFormat,
      bodyHtml,
      attachmentPaths: [...form.value.attachmentPaths],
      localDraftId: form.value.localDraftId,
      undoDelaySecs: sendOpts.undoDelaySecs ?? null,
      scheduledAt: form.value.scheduledAt,
      clientRequestId: pendingSendRequestId,
    };

    try {
      const res = await sendMail(payload);
      pendingSendRequestId = null;
      stateVersion.value = Math.max(stateVersion.value, res.stateVersion);

      if (currentSnapshot.localDraftId) {
        drafts.value = drafts.value.filter(
          (d) => d.id !== currentSnapshot.localDraftId
        );
      }

      if (res.accepted && res.undoWindowSecs > 0) {
        startUndoCountdown({
          outboxId: res.outboxId,
          accountId: currentSnapshot.accountId,
          undoToken: res.undoToken,
          to: currentSnapshot.toAddr,
          subject: currentSnapshot.subject,
          remainingSecs: res.undoWindowSecs,
          totalSecs: res.undoWindowSecs,
          scheduledAt: res.scheduledAt,
          snapshot: {
            ...currentSnapshot,
            localDraftId: null,
          },
        });
      }

      dirty.value = false;
      open.value = false;
      await refreshOutbox();
      return { sent: true, output: res };
    } catch (err) {
      const msg = handleIpcFailure(err);
      return { sent: false, error: msg };
    } finally {
      sending.value = false;
    }
  }

  async function undoSend(): Promise<boolean> {
    const current = undoBanner.value;
    if (!current || undoing.value) return false;

    undoing.value = true;
    error.value = null;

    try {
      const res = await undoSendMail({
        accountId: current.accountId,
        outboxId: current.outboxId,
        undoToken: current.undoToken,
      });
      stateVersion.value = Math.max(stateVersion.value, res.stateVersion);
      clearUndoTickTimer();
      undoBanner.value = null;

      if (res.undone) {
        form.value = {
          ...current.snapshot,
          attachmentPaths: [...current.snapshot.attachmentPaths],
        };
        formRevision += 1;
        dirty.value = true;
        statusNotice.value = res.message || "已撤销发送，内容已恢复至编辑区";
        open.value = true;
        await refreshOutbox();
        return true;
      }

      statusNotice.value = res.message;
      await refreshOutbox();
      return false;
    } catch (err) {
      handleIpcFailure(err);
      return false;
    } finally {
      undoing.value = false;
    }
  }

  function dismissUndoBanner(): void {
    clearUndoTickTimer();
    undoBanner.value = null;
  }

  async function retryOutbox(accountId: string, outboxId: string): Promise<boolean> {
    busyOutboxId.value = outboxId;
    error.value = null;
    try {
      const res = await retryOutboxItem({ accountId, outboxId });
      stateVersion.value = Math.max(stateVersion.value, res.stateVersion);
      await refreshOutbox();
      return res.accepted;
    } catch (err) {
      handleIpcFailure(err);
      return false;
    } finally {
      busyOutboxId.value = null;
    }
  }

  async function deleteOutbox(accountId: string, outboxId: string): Promise<boolean> {
    busyOutboxId.value = outboxId;
    error.value = null;
    try {
      const res = await deleteOutboxItem({ accountId, outboxId });
      stateVersion.value = Math.max(stateVersion.value, res.stateVersion);
      if (undoBanner.value?.outboxId === outboxId) {
        dismissUndoBanner();
      }
      await refreshOutbox();
      return res.accepted;
    } catch (err) {
      handleIpcFailure(err);
      return false;
    } finally {
      busyOutboxId.value = null;
    }
  }

  function dispose(): void {
    disposed = true;
    clearAutoSaveTimer();
    clearUndoTickTimer();
    while (unlistenFns.length > 0) {
      const fn = unlistenFns.pop();
      fn?.();
    }
  }

  if (getCurrentInstance()) {
    onMounted(() => {
      void init();
    });
    onUnmounted(() => {
      dispose();
    });
  }

  return {
    open,
    accounts,
    form,
    dirty,
    savingDraft,
    lastSavedAt,
    drafts,
    outboxItems,
    sending,
    undoing,
    busyOutboxId,
    error,
    statusNotice,
    attachmentWarningIgnored,
    missingAttachmentCheck,
    undoBanner,
    stateVersion,
    setAccounts,
    init,
    openCompose,
    openDraft,
    updateField,
    addAttachmentPaths,
    removeAttachmentAt,
    ignoreAttachmentWarning,
    saveDraftNow,
    discardDraft,
    closeWithAutoSave,
    send,
    undoSend,
    dismissUndoBanner,
    refreshDrafts,
    deleteDraftById,
    refreshOutbox,
    retryOutbox,
    deleteOutbox,
    dispose,
  };
}
