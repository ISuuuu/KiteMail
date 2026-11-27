/**
 * KiteMail 命令面板与快捷键系统类型定义。
 */

export interface CommandPaletteItem {
  id: string;
  title: string;
  category: string;
  shortcut?: string;
  keywords?: string[];
  disabled?: boolean;
}

export interface EventTargetLike {
  tagName?: string;
  isContentEditable?: boolean;
  getAttribute?: (name: string) => string | null;
}

export interface KeyboardEventLike {
  key: string;
  code?: string;
  keyCode?: number;
  ctrlKey?: boolean;
  metaKey?: boolean;
  altKey?: boolean;
  shiftKey?: boolean;
  isComposing?: boolean;
  target?: EventTargetLike | EventTarget | null;
}

export interface ShortcutContext {
  singleKeyShortcuts: boolean;
  hasOpenModal?: boolean;
  isInputFocused?: boolean;
}

export interface ShortcutMatchResult {
  matched: boolean;
  commandId: string | null;
  isPaletteToggle: boolean;
  preventDefault: boolean;
}

export interface ShortcutHelpEntry {
  keys: string[];
  description: string;
  requiresSingleKey?: boolean;
}

export interface ShortcutHelpGroup {
  title: string;
  entries: ShortcutHelpEntry[];
}
