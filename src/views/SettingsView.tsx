import { useEffect, useState } from "react";

import {
  errorMessage,
  getDefaultSettings,
  saveSettings,
  type Settings,
} from "../lib/ipc";

interface SettingsViewProps {
  settings: Settings | null;
  loadError: string | null;
}

interface ToggleFieldProps {
  id: string;
  label: string;
  description: string;
  checked: boolean;
  onChange: (checked: boolean) => void;
}

function ToggleField({
  id,
  label,
  description,
  checked,
  onChange,
}: ToggleFieldProps) {
  return (
    <div className="setting-row">
      <div className="setting-copy">
        <label htmlFor={id}>{label}</label>
        <p>{description}</p>
      </div>
      <div className="setting-control">
        <input
          id={id}
          type="checkbox"
          className="toggle"
          checked={checked}
          onChange={(event) => onChange(event.currentTarget.checked)}
        />
      </div>
    </div>
  );
}

interface NumberFieldProps {
  id: string;
  label: string;
  description: string;
  value: number;
  min: number;
  max: number;
  unit: string;
  onChange: (value: number) => void;
}

function NumberField({
  id,
  label,
  description,
  value,
  min,
  max,
  unit,
  onChange,
}: NumberFieldProps) {
  return (
    <div className="setting-row">
      <div className="setting-copy">
        <label htmlFor={id}>{label}</label>
        <p>{description}</p>
      </div>
      <div className="setting-control">
        <input
          id={id}
          type="number"
          min={min}
          max={max}
          step={1}
          value={Number.isFinite(value) ? value : ""}
          onChange={(event) => onChange(event.currentTarget.valueAsNumber)}
        />
        <span className="setting-unit">{unit}</span>
      </div>
    </div>
  );
}

function SettingsView({ settings, loadError }: SettingsViewProps) {
  const [draft, setDraft] = useState<Settings | null>(settings);
  const [saving, setSaving] = useState(false);
  const [feedback, setFeedback] = useState<string | null>(null);
  const [saveError, setSaveError] = useState<string | null>(null);

  useEffect(() => {
    setDraft(settings);
  }, [settings]);

  if (loadError) {
    return (
      <section className="settings">
        <div className="form-error" role="alert">
          Settings could not be loaded: {loadError}
        </div>
      </section>
    );
  }

  if (!draft) {
    return (
      <section className="settings">
        <p className="muted">Loading settings…</p>
      </section>
    );
  }

  function update<Key extends keyof Settings>(key: Key, value: Settings[Key]) {
    setDraft((current) => (current ? { ...current, [key]: value } : current));
    setFeedback(null);
    setSaveError(null);
  }

  async function handleSave() {
    if (!draft) {
      return;
    }

    const numericSettings: Array<[string, number]> = [
      ["Auto-commit debounce", draft.autoCommitDebounceSeconds],
      ["Run history retention", draft.runRetentionPerScript],
      ["Log size cap", draft.logSizeCapMb],
    ];
    const missingNumber = numericSettings.find(
      ([, value]) => !Number.isFinite(value),
    );

    if (missingNumber) {
      setFeedback(null);
      setSaveError(`${missingNumber[0]} must be a number.`);
      return;
    }

    setSaving(true);
    setFeedback(null);
    setSaveError(null);

    try {
      const saved = await saveSettings(draft);
      setDraft(saved);
      setFeedback("Saved locally. Runs and history use these values.");
    } catch (error) {
      setSaveError(errorMessage(error));
    } finally {
      setSaving(false);
    }
  }

  async function handleRestoreDefaults() {
    setFeedback(null);
    setSaveError(null);

    try {
      setDraft(await getDefaultSettings());
      setFeedback("Defaults loaded. Save to apply them.");
    } catch (error) {
      setSaveError(errorMessage(error));
    }
  }

  return (
    <section className="settings">
      <p className="settings-intro">
        App-wide defaults. Values are stored in the local app database, never in
        the script repository.
      </p>

      <form
        className="settings-form"
        onSubmit={(event) => {
          event.preventDefault();
          void handleSave();
        }}
      >
        <fieldset className="settings-card">
          <legend>Git and commits</legend>

          <ToggleField
            id="auto-commit-enabled"
            label="Auto-commit script changes"
            description="Batch changed script files into one commit after the debounce window."
            checked={draft.autoCommitEnabled}
            onChange={(value) => update("autoCommitEnabled", value)}
          />

          <NumberField
            id="auto-commit-debounce"
            label="Auto-commit debounce"
            description="Wait this long after the last edit before committing."
            value={draft.autoCommitDebounceSeconds}
            min={5}
            max={3600}
            unit="seconds"
            onChange={(value) => update("autoCommitDebounceSeconds", value)}
          />

          <ToggleField
            id="auto-push"
            label="Auto-push after commit"
            description="Off by default. The Sync button always pushes explicitly."
            checked={draft.autoPush}
            onChange={(value) => update("autoPush", value)}
          />
        </fieldset>

        <fieldset className="settings-card">
          <legend>Runs and history</legend>

          <NumberField
            id="run-retention"
            label="Run history retention"
            description="Old runs and their logs are deleted with their records."
            value={draft.runRetentionPerScript}
            min={1}
            max={10000}
            unit="runs per script"
            onChange={(value) => update("runRetentionPerScript", value)}
          />

          <NumberField
            id="log-size-cap"
            label="Log size cap"
            description="Truncate a run's log once it reaches this size."
            value={draft.logSizeCapMb}
            min={1}
            max={1024}
            unit="MB per run"
            onChange={(value) => update("logSizeCapMb", value)}
          />

          <ToggleField
            id="notifications-enabled"
            label="Notify when a run finishes"
            description="Per-script overrides can still opt out later."
            checked={draft.notificationsEnabled}
            onChange={(value) => update("notificationsEnabled", value)}
          />
        </fieldset>

        <fieldset className="settings-card">
          <legend>Safety</legend>

          <ToggleField
            id="strict-mode"
            label="Strict mode"
            description="Block scripts whose content changed since they were last reviewed on this machine."
            checked={draft.strictMode}
            onChange={(value) => update("strictMode", value)}
          />

          <div className="setting-row is-stacked">
            <div className="setting-copy">
              <label htmlFor="path-override">PATH override</label>
              <p>
                Prepended to the captured login-shell environment for every run.
                Leave empty to inherit it unchanged.
              </p>
            </div>
            <input
              id="path-override"
              type="text"
              className="text-input"
              placeholder="/opt/homebrew/bin:/usr/local/bin"
              value={draft.pathOverride ?? ""}
              onChange={(event) =>
                update(
                  "pathOverride",
                  event.currentTarget.value === ""
                    ? null
                    : event.currentTarget.value,
                )
              }
            />
          </div>
        </fieldset>

        <div className="settings-actions">
          <button type="submit" className="button is-primary" disabled={saving}>
            {saving ? "Saving…" : "Save settings"}
          </button>
          <button
            type="button"
            className="button"
            onClick={() => void handleRestoreDefaults()}
          >
            Restore defaults
          </button>
          {feedback && <span className="save-note">{feedback}</span>}
        </div>

        {saveError && (
          <p className="form-error" role="alert">
            {saveError}
          </p>
        )}
      </form>
    </section>
  );
}

export default SettingsView;
