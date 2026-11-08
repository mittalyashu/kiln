import PlaceholderView from "./PlaceholderView";

function SchedulesView() {
  return (
    <PlaceholderView
      eyebrow="Schedules"
      title="No schedules yet"
      body="Schedules bind a script and preset to a cron expression. In v1 they fire only while the app is running, and missed runs are skipped rather than caught up."
      hints={[
        "Create, edit, and disable schedules",
        "See the next run time before enabling",
        "Scheduled runs use the same runner as manual runs",
      ]}
    />
  );
}

export default SchedulesView;
