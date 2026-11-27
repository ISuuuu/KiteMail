/**
 * KiteMail 通讯录管理 IPC 客户端与可控 Mock 传输层。
 *
 * 严格遵循后端参数包装约定：
 *   list_contacts({ input })
 *   save_contact({ input })
 *   delete_contact({ input })
 *   import_contacts({ input })
 *   export_contacts({ input })
 */

import { isTauri, extractErrorMessage, isLockedError } from "../ipc/main-window";
import type {
  ContactDto,
  ContactExportFormat,
  ContactImportFormat,
  DeleteContactInput,
  DeleteContactOutput,
  ExportContactsInput,
  ExportContactsOutput,
  ImportContactsInput,
  ImportContactsOutput,
  ListContactsInput,
  ListContactsOutput,
  SaveContactInput,
  SaveContactOutput,
} from "./types";

export { isTauri, extractErrorMessage, isLockedError };

async function tauriInvoke<T>(cmd: string, args?: unknown): Promise<T> {
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<T>(cmd, args as Record<string, unknown>);
}

// ---------- Mock 数据与工具 ----------

const MOCK_NOW = Math.floor(Date.now() / 1000);

const DEFAULT_MOCK_CONTACTS: ContactDto[] = [
  {
    id: 1,
    name: "林晓明",
    email: "xiaoming@example.com",
    notes: "Windows 客户端基线负责人",
    phone: "+86 138-0000-1001",
    organization: "KiteMail 核心组",
    jobTitle: "高级客户端工程师",
    groups: ["核心团队", "工作"],
    isUserContact: true,
    messageCount: 18,
    lastContactTs: MOCK_NOW - 3600,
  },
  {
    id: 2,
    name: "陈思远",
    email: "siyuan@example.org",
    notes: "性能压测接口人",
    phone: "+86 139-0000-2002",
    organization: "效能工程部",
    jobTitle: "测试架构师",
    groups: ["工作"],
    isUserContact: true,
    messageCount: 9,
    lastContactTs: MOCK_NOW - 86400,
  },
  {
    id: null,
    name: "产品设计组",
    email: "design@kitemail.example",
    notes: "",
    phone: "",
    organization: "",
    jobTitle: "",
    groups: [],
    isUserContact: false,
    messageCount: 12,
    lastContactTs: MOCK_NOW - 7200,
  },
  {
    id: null,
    name: "Build Bot",
    email: "ci@kitemail.example",
    notes: "",
    phone: "",
    organization: "",
    jobTitle: "",
    groups: [],
    isUserContact: false,
    messageCount: 45,
    lastContactTs: MOCK_NOW - 1800,
  },
];

function cloneContact(c: ContactDto): ContactDto {
  return {
    ...c,
    groups: [...c.groups],
  };
}

function normalizeEmail(email: string): string {
  return email.trim().toLowerCase();
}

function isValidEmailFormat(email: string): boolean {
  return /^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email.trim());
}

function makeLockedError() {
  return {
    code: "LOCKED",
    message: "应用已锁定，请先解锁后再访问通讯录",
    retryable: false,
  };
}

function makeInvalidInputError(message: string) {
  return {
    code: "INVALID_INPUT",
    message,
    retryable: false,
  };
}

interface ParsedContactRow {
  name: string;
  email: string;
  phone: string;
  organization: string;
  jobTitle: string;
  groups: string[];
  notes: string;
}

function unfoldVCardLines(raw: string): string[] {
  const normalized = raw.replace(/\r\n/g, "\n").replace(/\r/g, "\n");
  const rawLines = normalized.split("\n");
  const unfolded: string[] = [];
  for (const line of rawLines) {
    if ((line.startsWith(" ") || line.startsWith("\t")) && unfolded.length > 0) {
      unfolded[unfolded.length - 1] += line.slice(1);
    } else {
      unfolded.push(line);
    }
  }
  return unfolded;
}

function unescapeVCardValue(val: string): string {
  return val
    .replace(/\\n/gi, "\n")
    .replace(/\\,/g, ",")
    .replace(/\\;/g, ";")
    .replace(/\\\\/g, "\\")
    .trim();
}

function parseVCardContacts(content: string): ParsedContactRow[] {
  const lines = unfoldVCardLines(content);
  const rows: ParsedContactRow[] = [];
  let current: ParsedContactRow | null = null;
  let structuredName = "";

  for (const rawLine of lines) {
    const line = rawLine.trim();
    if (!line) continue;
    const upper = line.toUpperCase();
    if (upper === "BEGIN:VCARD") {
      current = {
        name: "",
        email: "",
        phone: "",
        organization: "",
        jobTitle: "",
        groups: [],
        notes: "",
      };
      structuredName = "";
      continue;
    }
    if (upper === "END:VCARD") {
      if (current) {
        if (!current.name && structuredName) {
          current.name = structuredName;
        }
        rows.push(current);
      }
      current = null;
      structuredName = "";
      continue;
    }
    if (!current) continue;

    const colonIdx = line.indexOf(":");
    if (colonIdx <= 0) continue;
    const propPart = line.slice(0, colonIdx).toUpperCase();
    const propName = propPart.split(";")[0];
    const value = unescapeVCardValue(line.slice(colonIdx + 1));

    switch (propName) {
      case "FN":
        if (!current.name) current.name = value;
        break;
      case "N": {
        const parts = value.split(";").map((s) => s.trim()).filter(Boolean);
        structuredName = parts.reverse().join(" ").trim();
        break;
      }
      case "EMAIL":
        if (!current.email) current.email = value.trim();
        break;
      case "TEL":
        if (!current.phone) current.phone = value;
        break;
      case "ORG":
        if (!current.organization) current.organization = value.replace(/;+$/g, "");
        break;
      case "TITLE":
        if (!current.jobTitle) current.jobTitle = value;
        break;
      case "NOTE":
        current.notes = current.notes ? `${current.notes}\n${value}` : value;
        break;
      case "CATEGORIES": {
        const cats = value
          .split(",")
          .map((g) => g.trim())
          .filter(Boolean);
        current.groups.push(...cats);
        break;
      }
    }
  }
  return rows;
}

function parseCsvMatrix(content: string): string[][] {
  const clean = content.replace(/^\uFEFF/, "");
  const rows: string[][] = [];
  let row: string[] = [];
  let cell = "";
  let inQuotes = false;

  for (let i = 0; i < clean.length; i++) {
    const ch = clean[i];
    if (inQuotes) {
      if (ch === '"') {
        if (clean[i + 1] === '"') {
          cell += '"';
          i += 1;
        } else {
          inQuotes = false;
        }
      } else {
        cell += ch;
      }
      continue;
    }

    if (ch === '"') {
      inQuotes = true;
    } else if (ch === ",") {
      row.push(cell);
      cell = "";
    } else if (ch === "\r") {
      if (clean[i + 1] === "\n") i += 1;
      row.push(cell);
      rows.push(row);
      row = [];
      cell = "";
    } else if (ch === "\n") {
      row.push(cell);
      rows.push(row);
      row = [];
      cell = "";
    } else {
      cell += ch;
    }
  }

  if (cell.length > 0 || row.length > 0) {
    row.push(cell);
    rows.push(row);
  }
  return rows.filter((r) => r.some((c) => c.trim().length > 0));
}

function parseCsvContacts(content: string): ParsedContactRow[] {
  const matrix = parseCsvMatrix(content);
  if (matrix.length === 0) return [];

  const firstRow = matrix[0].map((h) => h.trim().toLowerCase());
  const looksLikeHeader = firstRow.some((h) =>
    ["email", "e-mail", "邮箱", "邮件", "name", "姓名", "名称"].includes(h)
  );

  let colName = 0;
  let colEmail = 1;
  let colPhone = 2;
  let colOrg = 3;
  let colTitle = 4;
  let colGroups = 5;
  let colNotes = 6;
  let startIdx = 0;

  if (looksLikeHeader) {
    startIdx = 1;
    colName = firstRow.findIndex((h) => ["name", "姓名", "名称", "display name", "fn"].includes(h));
    colEmail = firstRow.findIndex((h) => ["email", "e-mail", "mail", "邮箱", "邮件地址"].includes(h));
    colPhone = firstRow.findIndex((h) => ["phone", "tel", "mobile", "电话", "手机"].includes(h));
    colOrg = firstRow.findIndex((h) => ["organization", "org", "company", "公司", "组织", "单位"].includes(h));
    colTitle = firstRow.findIndex((h) => ["jobtitle", "job_title", "title", "职位", "头衔"].includes(h));
    colGroups = firstRow.findIndex((h) => ["groups", "group", "categories", "分组", "标签"].includes(h));
    colNotes = firstRow.findIndex((h) => ["notes", "note", "remark", "备注"].includes(h));
  }

  const readCell = (r: string[], idx: number): string =>
    idx >= 0 && idx < r.length ? r[idx].trim() : "";

  const rows: ParsedContactRow[] = [];
  for (let i = startIdx; i < matrix.length; i++) {
    const r = matrix[i];
    const email = readCell(r, colEmail);
    const name = readCell(r, colName);
    const groupsRaw = readCell(r, colGroups);
    rows.push({
      name: name || (email ? email.split("@")[0] : ""),
      email,
      phone: readCell(r, colPhone),
      organization: readCell(r, colOrg),
      jobTitle: readCell(r, colTitle),
      groups: groupsRaw
        ? groupsRaw
            .split(/[;|,]/)
            .map((g) => g.trim())
            .filter(Boolean)
        : [],
      notes: readCell(r, colNotes),
    });
  }
  return rows;
}

function escapeCsvCell(value: string): string {
  if (/[",\r\n]/.test(value)) {
    return `"${value.replace(/"/g, '""')}"`;
  }
  return value;
}

function escapeVCardValue(value: string): string {
  return value
    .replace(/\\/g, "\\\\")
    .replace(/\r?\n/g, "\\n")
    .replace(/;/g, "\\;")
    .replace(/,/g, "\\,");
}

function formatContactsAsVCard(contacts: readonly ContactDto[]): string {
  return contacts
    .map((c) => {
      const lines = [
        "BEGIN:VCARD",
        "VERSION:3.0",
        `FN:${escapeVCardValue(c.name || c.email)}`,
        `EMAIL;TYPE=INTERNET:${c.email}`,
      ];
      if (c.phone) lines.push(`TEL;TYPE=CELL:${escapeVCardValue(c.phone)}`);
      if (c.organization) lines.push(`ORG:${escapeVCardValue(c.organization)}`);
      if (c.jobTitle) lines.push(`TITLE:${escapeVCardValue(c.jobTitle)}`);
      if (c.groups.length > 0) {
        lines.push(`CATEGORIES:${c.groups.map(escapeVCardValue).join(",")}`);
      }
      if (c.notes) lines.push(`NOTE:${escapeVCardValue(c.notes)}`);
      lines.push("END:VCARD");
      return lines.join("\r\n");
    })
    .join("\r\n");
}

function formatContactsAsCsv(contacts: readonly ContactDto[]): string {
  const header = ["Name", "Email", "Phone", "Organization", "JobTitle", "Groups", "Notes"].join(",");
  const rows = contacts.map((c) =>
    [
      escapeCsvCell(c.name),
      escapeCsvCell(c.email),
      escapeCsvCell(c.phone),
      escapeCsvCell(c.organization),
      escapeCsvCell(c.jobTitle),
      escapeCsvCell(c.groups.join(";")),
      escapeCsvCell(c.notes),
    ].join(",")
  );
  return [header, ...rows].join("\r\n");
}

// ---------- Mock 状态管理 ----------

export interface MockContactsOptions {
  contacts?: ContactDto[];
  locked?: boolean;
  stateVersion?: number;
}

let mockLocked = false;
let mockStateVersion = 1;
let mockNextId = 3;
let mockContacts: ContactDto[] = DEFAULT_MOCK_CONTACTS.map(cloneContact);

function computeNextId(list: readonly ContactDto[]): number {
  let maxId = 0;
  for (const c of list) {
    if (c.id !== null && c.id > maxId) maxId = c.id;
  }
  return maxId + 1;
}

export function __resetContactsMock(options?: MockContactsOptions): void {
  mockContacts = (options?.contacts ?? DEFAULT_MOCK_CONTACTS).map(cloneContact);
  mockLocked = options?.locked ?? false;
  mockStateVersion = options?.stateVersion ?? 1;
  mockNextId = computeNextId(mockContacts);
}

// ---------- 公共 IPC API ----------

export async function listContacts(
  input: ListContactsInput = {}
): Promise<ListContactsOutput> {
  if (isTauri()) return tauriInvoke<ListContactsOutput>("list_contacts", { input });
  if (mockLocked) return Promise.reject(makeLockedError());

  const q = (input.query ?? "").trim().toLowerCase();
  const offset = Math.max(0, input.offset ?? 0);
  const limit = Math.max(1, input.limit ?? 100);

  const filtered = mockContacts.filter((c) => {
    if (!q) return true;
    return (
      c.name.toLowerCase().includes(q) ||
      c.email.toLowerCase().includes(q) ||
      c.phone.toLowerCase().includes(q) ||
      c.organization.toLowerCase().includes(q) ||
      c.jobTitle.toLowerCase().includes(q) ||
      c.notes.toLowerCase().includes(q) ||
      c.groups.some((g) => g.toLowerCase().includes(q))
    );
  });

  const sorted = [...filtered].sort((a, b) => {
    if (a.isUserContact !== b.isUserContact) return a.isUserContact ? -1 : 1;
    return b.lastContactTs - a.lastContactTs || a.name.localeCompare(b.name, "zh-CN");
  });

  const items = sorted.slice(offset, offset + limit).map(cloneContact);
  return {
    items,
    total: sorted.length,
    hasMore: offset + items.length < sorted.length,
    stateVersion: mockStateVersion,
  };
}

export async function saveContact(
  input: SaveContactInput
): Promise<SaveContactOutput> {
  if (isTauri()) return tauriInvoke<SaveContactOutput>("save_contact", { input });
  if (mockLocked) return Promise.reject(makeLockedError());

  const email = input.email.trim();
  if (!isValidEmailFormat(email)) {
    return Promise.reject(makeInvalidInputError("请输入有效的联系人邮箱地址"));
  }
  const name = input.name.trim() || email.split("@")[0] || email;
  const normalizedEmail = normalizeEmail(email);

  const groups = (input.groups ?? [])
    .map((g) => g.trim())
    .filter((g, idx, arr) => g.length > 0 && arr.indexOf(g) === idx);

  if (input.id != null) {
    const idx = mockContacts.findIndex((c) => c.id === input.id);
    if (idx < 0) {
      return Promise.reject(makeInvalidInputError(`未找到联系人 #${input.id}`));
    }
    const existing = mockContacts[idx];
    const duplicate = mockContacts.find(
      (c) =>
        c.isUserContact &&
        c.id !== input.id &&
        normalizeEmail(c.email) === normalizedEmail
    );
    if (duplicate) {
      return Promise.reject(makeInvalidInputError("该邮箱已被其他个人联系人占用"));
    }
    const updated: ContactDto = {
      ...existing,
      name,
      email,
      notes: input.notes?.trim() ?? "",
      phone: input.phone?.trim() ?? "",
      organization: input.organization?.trim() ?? "",
      jobTitle: input.jobTitle?.trim() ?? "",
      groups,
      isUserContact: true,
    };
    mockContacts[idx] = updated;
    mockStateVersion += 1;
    return {
      contact: cloneContact(updated),
      stateVersion: mockStateVersion,
    };
  }

  // 检查是否已存在同邮箱个人联系人或自动聚合联系人
  const existingUser = mockContacts.find(
    (c) => c.isUserContact && normalizeEmail(c.email) === normalizedEmail
  );
  if (existingUser) {
    return Promise.reject(makeInvalidInputError("该邮箱已在个人通讯录中"));
  }

  const autoIdx = mockContacts.findIndex(
    (c) => !c.isUserContact && normalizeEmail(c.email) === normalizedEmail
  );
  const autoExisting = autoIdx >= 0 ? mockContacts[autoIdx] : undefined;

  const created: ContactDto = {
    id: mockNextId++,
    name,
    email,
    notes: input.notes?.trim() ?? "",
    phone: input.phone?.trim() ?? "",
    organization: input.organization?.trim() ?? "",
    jobTitle: input.jobTitle?.trim() ?? "",
    groups,
    isUserContact: true,
    messageCount: autoExisting?.messageCount ?? 0,
    lastContactTs: autoExisting?.lastContactTs ?? 0,
  };

  if (autoIdx >= 0) {
    mockContacts[autoIdx] = created;
  } else {
    mockContacts.push(created);
  }
  mockStateVersion += 1;
  return {
    contact: cloneContact(created),
    stateVersion: mockStateVersion,
  };
}

export async function deleteContact(
  input: DeleteContactInput
): Promise<DeleteContactOutput> {
  if (isTauri()) return tauriInvoke<DeleteContactOutput>("delete_contact", { input });
  if (mockLocked) return Promise.reject(makeLockedError());

  const idx = mockContacts.findIndex(
    (c) => c.id === input.contactId && c.isUserContact
  );
  if (idx < 0) {
    return { deleted: false, stateVersion: mockStateVersion };
  }
  mockContacts.splice(idx, 1);
  mockStateVersion += 1;
  return { deleted: true, stateVersion: mockStateVersion };
}

export async function importContacts(
  input: ImportContactsInput
): Promise<ImportContactsOutput> {
  if (isTauri()) return tauriInvoke<ImportContactsOutput>("import_contacts", { input });
  if (mockLocked) return Promise.reject(makeLockedError());

  const content = input.content.trim();
  if (!content) {
    return Promise.reject(makeInvalidInputError("导入内容不能为空"));
  }

  let resolvedFormat: ContactImportFormat = input.format;
  if (resolvedFormat === "auto") {
    resolvedFormat = /BEGIN:VCARD/i.test(content) ? "vcard" : "csv";
  }

  const parsed =
    resolvedFormat === "vcard"
      ? parseVCardContacts(content)
      : parseCsvContacts(content);

  let added = 0;
  let skipped = 0;
  const seenEmails = new Set(
    mockContacts
      .filter((c) => c.isUserContact)
      .map((c) => normalizeEmail(c.email))
  );

  for (const row of parsed) {
    if (!isValidEmailFormat(row.email)) {
      skipped += 1;
      continue;
    }
    const norm = normalizeEmail(row.email);
    if (seenEmails.has(norm)) {
      skipped += 1;
      continue;
    }
    seenEmails.add(norm);

    const autoIdx = mockContacts.findIndex(
      (c) => !c.isUserContact && normalizeEmail(c.email) === norm
    );
    const autoExisting = autoIdx >= 0 ? mockContacts[autoIdx] : undefined;

    const newContact: ContactDto = {
      id: mockNextId++,
      name: row.name || row.email.split("@")[0],
      email: row.email.trim(),
      notes: row.notes,
      phone: row.phone,
      organization: row.organization,
      jobTitle: row.jobTitle,
      groups: row.groups,
      isUserContact: true,
      messageCount: autoExisting?.messageCount ?? 0,
      lastContactTs: autoExisting?.lastContactTs ?? 0,
    };

    if (autoIdx >= 0) {
      mockContacts[autoIdx] = newContact;
    } else {
      mockContacts.push(newContact);
    }
    added += 1;
  }

  if (added > 0) {
    mockStateVersion += 1;
  }

  return {
    added,
    skipped,
    totalParsed: parsed.length,
    stateVersion: mockStateVersion,
  };
}

export async function exportContacts(
  input: ExportContactsInput
): Promise<ExportContactsOutput> {
  if (isTauri()) return tauriInvoke<ExportContactsOutput>("export_contacts", { input });
  if (mockLocked) return Promise.reject(makeLockedError());

  const format: ContactExportFormat = input.format;
  const exportable = mockContacts.filter((c) => c.isUserContact);
  const content =
    format === "vcard"
      ? formatContactsAsVCard(exportable)
      : formatContactsAsCsv(exportable);

  return {
    format,
    filename: format === "vcard" ? "kitemail-contacts.vcf" : "kitemail-contacts.csv",
    content,
    count: exportable.length,
  };
}
