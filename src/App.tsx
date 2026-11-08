import { useEffect, useState } from "react";

import Sidebar, { type ViewId } from "./components/Sidebar";
import StatusBar from "./components/StatusBar";
import {
  errorMessage,
  getAppInfo,
  getSettings,
  onSettingsUpdated,
  type AppInfo,
  type Settings,
} from "./lib/ipc";
import EditorView from "./views/EditorView";
import LibraryView from "./views/LibraryView";
import RunsView from "./views/RunsView";
import SchedulesView from "./views/SchedulesView";
import SettingsView from "./views/SettingsView";
import "./App.css";

const VIEW_META: Record<ViewId, { title: string; description: string }> = {
  library: {
    title: "Library",
    description: "Folders, tags, and scripts from your connected repository.",
  },
  editor: {
    title: "Editor",
    description: "Edit scripts and inspect their declared parameters.",
  },
  runs: {
    title: "Runs",
    description: "Live output and history for every execution.",
  },
  schedules: {
    title: "Schedules",
    description: "Cron schedules bound to scripts and presets.",
  },
  settings: {
    title: "Settings",
    description: "App-wide defaults stored locally on this machine.",
  },
};

function App() {
  const [view, setView] = useState<ViewId>("library");
  const [appInfo, setAppInfo] = useState<AppInfo | null>(null);
  const [startupError, setStartupError] = useState<string | null>(null);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [settingsError, setSettingsError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    const unlisteners: Array<() => void> = [];

    getAppInfo()
      .then((info) => {
        if (!cancelled) {
          setAppInfo(info);
        }
      })
      .catch((error) => {
        if (!cancelled) {
          setStartupError(errorMessage(error));
        }
      });

    getSettings()
      .then((loaded) => {
        if (!cancelled) {
          setSettings(loaded);
        }
      })
      .catch((error) => {
        if (!cancelled) {
          setSettingsError(errorMessage(error));
        }
      });

    void onSettingsUpdated((saved) => {
      if (!cancelled) {
        setSettings(saved);
        setSettingsError(null);
      }
    })
      .then((unlisten) => {
        if (cancelled) {
          unlisten();
        } else {
          unlisteners.push(unlisten);
        }
      })
      .catch((error) => {
        if (!cancelled) {
          setSettingsError(errorMessage(error));
        }
      });

    return () => {
      cancelled = true;
      unlisteners.forEach((unlisten) => unlisten());
    };
  }, []);

  const meta = VIEW_META[view];

  function renderView() {
    switch (view) {
      case "library":
        return <LibraryView />;
      case "editor":
        return <EditorView />;
      case "runs":
        return <RunsView />;
      case "schedules":
        return <SchedulesView />;
      case "settings":
        return <SettingsView settings={settings} loadError={settingsError} />;
    }
  }

  return (
    <div className="app-shell">
      <Sidebar activeView={view} onNavigate={setView} />
      <div className="app-main">
        <header className="app-header">
          <div>
            <h1>{meta.title}</h1>
            <p>{meta.description}</p>
          </div>
          <span className="local-badge">Local only</span>
        </header>

        <main className="app-content">{renderView()}</main>

        <StatusBar appInfo={appInfo} error={startupError} />
      </div>
    </div>
  );
}

export default App;
