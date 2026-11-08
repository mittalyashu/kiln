import PlaceholderView from "./PlaceholderView";

function LibraryView() {
  return (
    <PlaceholderView
      eyebrow="Library"
      title="No script repository connected yet"
      body="The folder tree, tag filters, and search will live here. Connect a local or remote git repository to get started."
      hints={[
        "Create a new local script repository",
        "Connect an existing repository by folder or remote URL",
        "Scripts are plain files, so they keep working in a terminal",
      ]}
    />
  );
}

export default LibraryView;
