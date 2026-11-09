import PlaceholderView from "./PlaceholderView";

function EditorView() {
  return (
    <PlaceholderView
      eyebrow="Editor"
      title="Select a script to edit"
      body="Once the library is connected, the built-in editor will show the file, its parsed parameter header, and warnings for malformed declarations."
      hints={[
        "Syntax highlighting follows the script's shebang or extension",
        "External editors are watched for changes",
        "Parameter values never modify the saved script",
        "Scripts using @inject preamble report error lines shifted by the generated preamble",
      ]}
    />
  );
}

export default EditorView;
