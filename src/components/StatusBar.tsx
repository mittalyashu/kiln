import type { AppInfo } from "../lib/ipc";

interface StatusBarProps {
  appInfo: AppInfo | null;
  error: string | null;
}

function StatusBar({ appInfo, error }: StatusBarProps) {
  if (error) {
    return (
      <footer className="status-bar is-error" role="status">
        <span className="status-dot" aria-hidden="true" />
        <span>Backend unavailable: {error}</span>
      </footer>
    );
  }

  if (!appInfo) {
    return (
      <footer className="status-bar" role="status">
        <span className="status-dot is-pending" aria-hidden="true" />
        <span>Connecting to the local database…</span>
      </footer>
    );
  }

  return (
    <footer className="status-bar" role="status">
      <span className="status-dot" aria-hidden="true" />
      <span>
        Backend ready · schema v{appInfo.schemaVersion} · app v
        {appInfo.appVersion}
      </span>
      <span className="status-path" title={appInfo.databasePath}>
        {appInfo.databasePath}
      </span>
    </footer>
  );
}

export default StatusBar;
