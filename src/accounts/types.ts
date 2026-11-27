/**
 * KiteMail 多账号管理模块类型定义与冻结 IPC 契约。
 * 所有枚举值与结构体字段严格使用 camelCase，与 Rust 后端保持一致。
 */

export type AuthBackend = "password" | "microsoftGraphOAuth2";
export type SecurityKind = "tls" | "startTls";

export interface AccountConfigDto {
  id: string;
  authBackend: AuthBackend;
  oauthClientId: string | null;
  displayName: string;
  email: string;
  label: string;
  enabled: boolean;
  inUnified: boolean;
  imapHost: string;
  imapPort: number;
  imapSecurity: SecurityKind;
  imapUsername: string;
  smtpHost: string;
  smtpPort: number;
  smtpSecurity: SecurityKind;
  smtpUsername: string;
  smtpSeparateCredentials: boolean;
  signature: string;
  hasPassword: boolean;
}

export interface ProviderPresetDto {
  id: string;
  name: string;
  domains: string[];
  authBackend: AuthBackend;
  imapHost: string;
  imapPort: number;
  imapSecurity: SecurityKind;
  smtpHost: string;
  smtpPort: number;
  smtpSecurity: SecurityKind;
  note: string;
}

export interface SaveAccountInput {
  id?: string | null;
  authBackend: AuthBackend;
  oauthClientId?: string | null;
  displayName: string;
  email: string;
  label?: string;
  enabled?: boolean;
  inUnified?: boolean;
  imapHost: string;
  imapPort: number;
  imapSecurity: SecurityKind;
  imapUsername: string;
  smtpHost: string;
  smtpPort: number;
  smtpSecurity: SecurityKind;
  smtpUsername: string;
  smtpSeparateCredentials?: boolean;
  signature?: string;
  imapPassword?: string | null;
  smtpPassword?: string | null;
}

export interface ListAccountsOutput {
  accounts: AccountConfigDto[];
  presets: ProviderPresetDto[];
  stateVersion: number;
}

export interface VerifyAccountOutput {
  ok: boolean;
  message: string;
}

export interface SaveAccountOutput {
  account: AccountConfigDto;
  stateVersion: number;
}

export interface DeleteAccountInput {
  accountId: string;
}

export interface DeleteAccountOutput {
  deleted: boolean;
  stateVersion: number;
}

export interface SetAccountEnabledInput {
  accountId: string;
  enabled: boolean;
}

export interface SetAccountEnabledOutput {
  accountId: string;
  enabled: boolean;
  stateVersion: number;
}

export interface StartMicrosoftOAuthInput {
  customClientId?: string | null;
  loginHintEmail?: string | null;
  editingAccountId?: string | null;
}

export interface StartMicrosoftOAuthOutput {
  started: boolean;
  message: string;
}

export type AccountEventKind =
  | "verified"
  | "saved"
  | "deleted"
  | "updated"
  | "oauthReauthorized"
  | "oauthRebindRejected";

export interface AccountEvent {
  stateVersion: number;
  kind: AccountEventKind;
  ok: boolean;
  accountId?: string | null;
  email?: string | null;
  message: string;
}

// ---------- 表单、校验与预设匹配类型 ----------

export interface AccountFormModel {
  id: string | null;
  presetId: string;
  authBackend: AuthBackend;
  oauthClientId: string;
  displayName: string;
  email: string;
  label: string;
  enabled: boolean;
  inUnified: boolean;
  imapHost: string;
  imapPort: number;
  imapSecurity: SecurityKind;
  imapUsername: string;
  imapPassword: string;
  smtpHost: string;
  smtpPort: number;
  smtpSecurity: SecurityKind;
  smtpUsername: string;
  smtpPassword: string;
  smtpSeparateCredentials: boolean;
  signature: string;
  hasPassword: boolean;
}

export type LockableAccountField =
  | "authBackend"
  | "imapHost"
  | "imapPort"
  | "imapSecurity"
  | "smtpHost"
  | "smtpPort"
  | "smtpSecurity"
  | "imapUsername"
  | "smtpUsername";

export type AccountFieldErrorKey =
  | "email"
  | "displayName"
  | "imapHost"
  | "imapPort"
  | "imapUsername"
  | "imapPassword"
  | "smtpHost"
  | "smtpPort"
  | "smtpUsername"
  | "smtpPassword"
  | "oauthClientId";

export type AccountValidationErrors = Partial<Record<AccountFieldErrorKey, string>>;

export interface AccountValidationResult {
  valid: boolean;
  errors: AccountValidationErrors;
  firstError: string | null;
}

export interface PresetMatchResult {
  matched: boolean;
  domain: string;
  preset: ProviderPresetDto | null;
}

export type OAuthFlowStatus = "idle" | "pending" | "authorized" | "rejected" | "error";

export interface StatusBanner {
  kind: "success" | "error" | "info";
  text: string;
}
