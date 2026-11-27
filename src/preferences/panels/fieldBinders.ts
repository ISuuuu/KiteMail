/**
 * 偏好设置面板共享字段绑定器（从 PreferencesModal.vue 抽出）。
 * 面板组件通过 props 接收 `usePreferences` 状态并直接调用这些函数，保持既有 IPC 行为不变。
 */
import type { FullPreferencesDto } from "../types";
import type { PrefsPanelState } from "./state";

export function darkModeSelectValue(state: PrefsPanelState): string {
  if (state.preferences.value.darkMode === true) return "dark";
  if (state.preferences.value.darkMode === false) return "light";
  return "system";
}

export function onDarkModeChange(state: PrefsPanelState, ev: Event) {
  const val = (ev.target as HTMLSelectElement).value;
  const darkMode = val === "dark" ? true : val === "light" ? false : null;
  void state.updatePreference({ darkMode });
}

export function onSelectString(
  state: PrefsPanelState,
  field: "readerPosition" | "readMarkMode" | "defaultView",
  ev: Event
) {
  const val = (ev.target as HTMLSelectElement).value;
  void state.updatePreference({ [field]: val } as Partial<FullPreferencesDto>);
}

/** ComboSelect 字符串值版本。 */
export function onSelectStringValue(
  state: PrefsPanelState,
  field: "readerPosition" | "readMarkMode" | "defaultView",
  value: string
) {
  void state.updatePreference({ [field]: value } as Partial<FullPreferencesDto>);
}

export function onCheckbox(state: PrefsPanelState, field: keyof FullPreferencesDto, ev: Event) {
  const checked = (ev.target as HTMLInputElement).checked;
  void state.updatePreference({ [field]: checked } as Partial<FullPreferencesDto>);
}

export function onNumberInput(
  state: PrefsPanelState,
  field:
    | "readerZoomPercent"
    | "uiTextScalePercent"
    | "autoSyncMinutes"
    | "undoSendSecs"
    | "appLockIdleMinutes",
  ev: Event
) {
  const num = Number((ev.target as HTMLInputElement).value);
  if (!Number.isFinite(num)) return;
  void state.updatePreference({ [field]: num });
}

export function onTextInput(
  state: PrefsPanelState,
  field: "attachmentReminderKeywords" | "replyHeaderTemplate",
  ev: Event
) {
  const val = (ev.target as HTMLInputElement).value;
  void state.updatePreference({ [field]: val });
}
