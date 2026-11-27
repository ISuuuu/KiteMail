/**
 * KiteMail 通讯录管理模块类型定义与冻结 IPC 契约。
 * 所有枚举值与结构体字段严格使用 camelCase，与 Rust 后端保持一致。
 */

export type ContactImportFormat = "auto" | "vcard" | "csv";
export type ContactExportFormat = "vcard" | "csv";

export interface ContactDto {
  id: number | null;
  name: string;
  email: string;
  notes: string;
  phone: string;
  organization: string;
  jobTitle: string;
  groups: string[];
  isUserContact: boolean;
  messageCount: number;
  lastContactTs: number;
}

export interface ListContactsInput {
  query?: string;
  offset?: number;
  limit?: number;
}

export interface ListContactsOutput {
  items: ContactDto[];
  total: number;
  hasMore: boolean;
  stateVersion: number;
}

export interface SaveContactInput {
  id?: number | null;
  name: string;
  email: string;
  notes?: string;
  phone?: string;
  organization?: string;
  jobTitle?: string;
  groups?: string[];
}

export interface SaveContactOutput {
  contact: ContactDto;
  stateVersion: number;
}

export interface DeleteContactInput {
  contactId: number;
}

export interface DeleteContactOutput {
  deleted: boolean;
  stateVersion: number;
}

export interface ImportContactsInput {
  format: ContactImportFormat;
  content: string;
}

export interface ImportContactsOutput {
  added: number;
  skipped: number;
  totalParsed: number;
  stateVersion: number;
}

export interface ExportContactsInput {
  format: ContactExportFormat;
}

export interface ExportContactsOutput {
  format: ContactExportFormat;
  filename: string;
  content: string;
  count: number;
}

// ---------- 前端表单与状态类型 ----------

export interface ContactFormModel {
  id: number | null;
  name: string;
  email: string;
  phone: string;
  organization: string;
  jobTitle: string;
  groupsText: string;
  notes: string;
  isUserContact: boolean;
  messageCount: number;
  lastContactTs: number;
}

export type ContactFieldErrorKey = "name" | "email";
export type ContactValidationErrors = Partial<Record<ContactFieldErrorKey, string>>;

export interface ContactValidationResult {
  valid: boolean;
  errors: ContactValidationErrors;
  firstError: string | null;
}

export interface ContactStatusBanner {
  kind: "success" | "error" | "info";
  text: string;
}
