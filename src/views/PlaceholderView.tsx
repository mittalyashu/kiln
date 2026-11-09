interface PlaceholderViewProps {
  eyebrow: string;
  title: string;
  body: string;
  hints: string[];
}

function PlaceholderView({ eyebrow, title, body, hints }: PlaceholderViewProps) {
  return (
    <section className="placeholder">
      <p className="eyebrow">{eyebrow}</p>
      <h2>{title}</h2>
      <p className="placeholder-body">{body}</p>
      <ul className="placeholder-hints">
        {hints.map((hint) => (
          <li key={hint}>{hint}</li>
        ))}
      </ul>
    </section>
  );
}

export default PlaceholderView;
