import PlaceholderView from "./PlaceholderView";

function RunsView() {
  return (
    <PlaceholderView
      eyebrow="Runs"
      title="No runs yet"
      body="Every execution will be recorded with its status, duration, exit code, parameters, and live logs. Runs stay in the background so the UI never blocks."
      hints={[
        "The same script can run concurrently",
        "Cancel kills the whole process group",
        "Retention defaults to the last 50 runs per script",
      ]}
    />
  );
}

export default RunsView;
