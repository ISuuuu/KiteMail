import type { BodyStatus } from "../ipc/types";

export type ReaderViewportMode =
  | "loading"
  | "unavailable"
  | "occluded"
  | "layout-pending"
  | "native-ready"
  | "fallback";

export interface ReaderViewportModeInput {
  tauriMode: boolean;
  bodyStatus: BodyStatus;
  hasHtml: boolean;
  threadCount: number;
  overlayOccluded: boolean;
  layoutError: string | null;
  localLayoutApplied: boolean;
  backendLayoutApplied?: boolean;
}

/**
 * 区分「导航就绪 (bodyStatus === ready)」与「原生布局/显示实际生效」：
 * 仅在本地布局回执已确认且未被后台拒绝/未报失败时才认定原生视图就绪，
 * 避免布局未生效前长期停留在纯文本占位或吞掉错误提示。
 */
export function resolveReaderViewportMode(input: ReaderViewportModeInput): ReaderViewportMode {
  if (input.bodyStatus === "loading") {
    return "loading";
  }
  if (input.bodyStatus === "unavailable" || Boolean(input.layoutError)) {
    return "unavailable";
  }
  const nativeEligible =
    input.tauriMode &&
    input.bodyStatus === "ready" &&
    (input.hasHtml || input.threadCount > 1);
  if (!nativeEligible) {
    return "fallback";
  }
  if (input.overlayOccluded) {
    return "occluded";
  }
  if (!input.localLayoutApplied || input.backendLayoutApplied === false) {
    return "layout-pending";
  }
  return "native-ready";
}

export interface EarlyRevealOptions {
  source: string;
  reveal: (source: string, stageMs: number) => Promise<void>;
  now?: () => number;
  onError?: (message: string, err: unknown) => void;
  onSuccess?: (source: string, stageMs: number) => void;
}

/**
 * 壳层挂载后立即触发显窗（不等待邮件启动快照，不经隐藏窗口 rAF 链），
 * 并在显窗调用失败时显式上报错误而不静默吞错。
 */
export async function triggerEarlyWindowReveal(options: EarlyRevealOptions): Promise<boolean> {
  const stageMs = options.now ? options.now() : typeof performance !== "undefined" ? performance.now() : 0;
  try {
    await options.reveal(options.source, stageMs);
    options.onSuccess?.(options.source, stageMs);
    return true;
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    options.onError?.(message, err);
    return false;
  }
}
