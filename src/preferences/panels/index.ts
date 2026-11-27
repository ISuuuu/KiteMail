/**
 * 偏好设置页面板注册表：旧弹窗 6 页签 → 面板组件。
 * 旧弹窗（PreferencesModal.vue）与独立设置窗共用同一面板实现。
 */
import type { Component } from "vue";
import type { PreferenceTabId } from "../types";
import AboutPanel from "./AboutPanel.vue";
import AppearancePanel from "./AppearancePanel.vue";
import ComposePanel from "./ComposePanel.vue";
import PrivacyPanel from "./PrivacyPanel.vue";
import StoragePanel from "./StoragePanel.vue";
import SyncPanel from "./SyncPanel.vue";
import "./panels.css";

export const PREFERENCE_PANELS: Record<PreferenceTabId, Component> = {
  appearance: AppearancePanel,
  sync: SyncPanel,
  compose: ComposePanel,
  privacy: PrivacyPanel,
  storage: StoragePanel,
  about: AboutPanel,
};

export { AboutPanel, AppearancePanel, ComposePanel, PrivacyPanel, StoragePanel, SyncPanel };
