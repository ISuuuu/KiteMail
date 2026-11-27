/**
 * 面板共享状态类型：`usePreferences` 的返回值。
 * 仅声明类型，不改变既有状态结构。
 */
import type { usePreferences } from "../usePreferences";

export type PrefsPanelState = ReturnType<typeof usePreferences>;
