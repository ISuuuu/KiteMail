/**
 * KiteMail 读信辅助、附件图库与打印预览模块类型定义。
 * 所有 IPC 命令入参/出参严格使用 camelCase，与 Rust 后端契约保持 100% 一致。
 */

import type { AttachmentDto, OpenMessageOutput } from "../ipc/types";

export interface CachedAttachmentDto {
  id: number;
  messageId: number;
  accountId: number;
  filename: string;
  mimeType: string;
  sizeBytes: number;
  sender: string;
  subject: string;
  dateTs: number;
  dateDisplay: string;
  downloaded: boolean;
  isImage: boolean;
}

export interface ListCachedAttachmentsInput {
  query?: string;
  offset?: number;
  limit?: number;
}

export interface ListCachedAttachmentsOutput {
  items: CachedAttachmentDto[];
  total: number;
  hasMore: boolean;
  stateVersion: number;
}

export interface FetchAttachmentPreviewInput {
  attachmentId: number;
  /** 仅要缩略图：不返回原文件数据，只返回受限尺寸的小图 */
  thumbnailOnly?: boolean;
}

/** 缩略图结果：只含小图 data URL，不含原文件 */
export interface FetchAttachmentThumbnailOutput {
  attachmentId: number;
  filename: string;
  mimeType: string;
  sizeBytes: number;
  thumbnailDataUrl: string | null;
}

export interface FetchAttachmentPreviewOutput {
  attachmentId: number;
  filename: string;
  mimeType: string;
  dataUrl: string;
  sizeBytes: number;
  thumbnailDataUrl?: string | null;
}

export interface SaveAttachmentInput {
  attachmentId: number;
  destPath?: string | null;
}

export interface SaveAttachmentOutput {
  saved: boolean;
  savedPath: string;
  message: string;
}

export interface SaveAllAttachmentsInput {
  attachmentIds: number[];
  destDir?: string | null;
}

export interface SaveAllAttachmentsOutput {
  savedCount: number;
  destDir: string;
  savedPaths: string[];
  message: string;
}

export interface GetMessagePrintHtmlInput {
  accountId: number;
  messageId: number;
  includeHeader?: boolean;
}

export interface GetMessagePrintHtmlOutput {
  subject: string;
  htmlDocument: string;
  plainText: string;
}

export interface OpenMessageWindowInput {
  accountId: number;
  messageId: number;
}

export interface OpenMessageWindowOutput {
  windowLabel: string;
  opened: boolean;
  message: OpenMessageOutput;
}

export type AttachmentFilterKind = "all" | "images" | "documents";

export type StandaloneReaderMode = "read" | "print";

export interface StandaloneMessageTarget {
  accountId: number;
  messageId: number;
}

export interface OpenAttachmentInput {
  attachmentId: number;
}

export interface OpenAttachmentOutput {
  opened: boolean;
  path: string;
  message: string;
}

export interface PickDirectoryOutput {
  cancelled: boolean;
  path: string | null;
}

export interface DrawerAttachmentItem {
  id: number;
  filename: string;
  mimeType: string;
  sizeBytes: number;
  downloaded: boolean;
  isImage: boolean;
  isPdf: boolean;
  messageId?: number;
}

export type { AttachmentDto, OpenMessageOutput };
