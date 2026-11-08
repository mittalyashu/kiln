import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/**
 * Shared TypeScript mirror of the Rust IPC types. Keep these in sync with
 * `src-tauri/src/commands.rs` and `src-tauri/src/settings.rs`.
 */

export interface AppInfo {
  appVersion: string;
  dataDir: string;
  databasePath: string;
  schemaVersion: number;
}

export interface Settings {
  autoCommitEnabled: boolean;
  autoCommitDebounceSeconds: number;
  autoPush: boolean;
  runRetentionPerScript: number;
  logSizeCapMb: number;
  notificationsEnabled: boolean;
  strictMode: boolean;
  pathOverride: string | null;
}

/** Emitted after settings are persisted, with the saved values as payload. */
export const SETTINGS_UPDATED_EVENT = "settings://updated";

export function getAppInfo(): Promise<AppInfo> {
  return invoke<AppInfo>("get_app_info");
}

export function getSettings(): Promise<Settings> {
  return invoke<Settings>("get_settings");
}

export function saveSettings(settings: Settings): Promise<Settings> {
  return invoke<Settings>("save_settings", { settings });
}

export function getDefaultSettings(): Promise<Settings> {
  return invoke<Settings>("get_default_settings");
}

export function onSettingsUpdated(
  handler: (settings: Settings) => void,
): Promise<UnlistenFn> {
  return listen<Settings>(SETTINGS_UPDATED_EVENT, (event) =>
    handler(event.payload),
  );
}

export function errorMessage(error: unknown): string {
  if (typeof error === "string") {
    return error;
  }
  if (error instanceof Error) {
    return error.message;
  }
  return String(error);
}
