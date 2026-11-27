/**
 * 阅读器工具栏会话内布局（对齐 GPUI pages.rs#render_appearance 的工具栏行与
 * mail-core ReaderToolbarLayout 语义：左组固定显示最多 7 个，右组窄时折叠，可隐藏）。
 * 对应偏好字段尚未进入前端 IPC 契约，本模块仅维护设置窗会话内状态。
 */

export type ToolbarSide = "left" | "right";

export interface ToolbarLayout {
  left: string[];
  right: string[];
}

export interface ToolbarItemMeta {
  key: string;
  label: string;
}

/** 与 GPUI `ToolbarItem::ALL` 顺序一致。 */
export const TOOLBAR_ITEMS: readonly ToolbarItemMeta[] = [
  { key: "reply", label: "回复" },
  { key: "replyAll", label: "全部回复" },
  { key: "forward", label: "转发" },
  { key: "star", label: "星标" },
  { key: "archive", label: "归档" },
  { key: "snooze", label: "稍后处理" },
  { key: "delete", label: "删除" },
  { key: "spam", label: "垃圾邮件" },
  { key: "readUnread", label: "已读 / 未读" },
  { key: "tags", label: "标签" },
  { key: "moveTo", label: "移动到" },
  { key: "find", label: "查找" },
  { key: "print", label: "打印" },
];

/** 阅读器工具栏按钮所在分区（对齐 TOOLBAR_ZONE_OPTIONS）。 */
export const TOOLBAR_ZONE_OPTIONS = [
  { label: "左组（固定显示）", value: "left" },
  { label: "右组（窄时折叠进 ⋯）", value: "right" },
  { label: "不显示", value: "hidden" },
] as const;

export const TOOLBAR_LEFT_MAX = 7;

/** 对齐 `ReaderToolbarLayout::default()`。 */
export function defaultToolbarLayout(): ToolbarLayout {
  return {
    left: ["reply", "replyAll", "forward", "star", "archive", "spam", "delete"],
    right: ["snooze", "tags", "readUnread", "moveTo", "find", "print"],
  };
}

export function toolbarSide(layout: ToolbarLayout, key: string): ToolbarSide | null {
  if (layout.left.includes(key)) return "left";
  if (layout.right.includes(key)) return "right";
  return null;
}

/** 在所属组内的位置（隐藏时返回 0，与 GPUI idx_at 一致）。 */
export function indexInZone(layout: ToolbarLayout, key: string): number {
  const side = toolbarSide(layout, key);
  if (!side) return 0;
  return Math.max(0, layout[side].indexOf(key));
}

/** 组内上移（-1）/下移（1）；隐藏或越界即不动（对齐 move_within）。 */
export function moveWithin(layout: ToolbarLayout, key: string, delta: -1 | 1): ToolbarLayout {
  const side = toolbarSide(layout, key);
  if (!side) return layout;
  const list = [...layout[side]];
  const pos = list.indexOf(key);
  const next = pos + delta;
  if (pos < 0 || next < 0 || next >= list.length) return layout;
  list.splice(pos, 1);
  list.splice(next, 0, key);
  return { ...layout, [side]: list };
}

/** 放置到某分区末尾；hidden 隐藏；左组满员且原本不在左组时不动（对齐 place）。 */
export function place(
  layout: ToolbarLayout,
  key: string,
  side: ToolbarSide | null
): ToolbarLayout {
  if (side === null) {
    return {
      left: layout.left.filter((k) => k !== key),
      right: layout.right.filter((k) => k !== key),
    };
  }
  if (
    side === "left" &&
    !layout.left.includes(key) &&
    layout.left.length >= TOOLBAR_LEFT_MAX
  ) {
    return layout;
  }
  return {
    left:
      side === "left"
        ? [...layout.left.filter((k) => k !== key), key]
        : layout.left.filter((k) => k !== key),
    right:
      side === "right"
        ? [...layout.right.filter((k) => k !== key), key]
        : layout.right.filter((k) => k !== key),
  };
}
