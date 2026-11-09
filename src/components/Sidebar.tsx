export type ViewId = "library" | "editor" | "runs" | "schedules" | "settings";

interface NavItem {
  id: ViewId;
  label: string;
  glyph: string;
}

const NAV_ITEMS: NavItem[] = [
  { id: "library", label: "Library", glyph: "▤" },
  { id: "editor", label: "Editor", glyph: "✎" },
  { id: "runs", label: "Runs", glyph: "▶" },
  { id: "schedules", label: "Schedules", glyph: "◷" },
  { id: "settings", label: "Settings", glyph: "⚙" },
];

interface SidebarProps {
  activeView: ViewId;
  onNavigate: (view: ViewId) => void;
}

function Sidebar({ activeView, onNavigate }: SidebarProps) {
  return (
    <aside className="sidebar">
      <div className="brand">
        <span className="brand-mark" aria-hidden="true">
          K
        </span>
        <div>
          <p className="brand-name">Kiln</p>
          <p className="brand-tagline">Script library</p>
        </div>
      </div>

      <nav className="nav" aria-label="Main navigation">
        {NAV_ITEMS.map((item) => (
          <button
            key={item.id}
            type="button"
            className={
              item.id === activeView ? "nav-item is-active" : "nav-item"
            }
            aria-current={item.id === activeView ? "page" : undefined}
            onClick={() => onNavigate(item.id)}
          >
            <span className="nav-glyph" aria-hidden="true">
              {item.glyph}
            </span>
            {item.label}
          </button>
        ))}
      </nav>

      <p className="sidebar-footnote">
        Scripts stay in your git repo. Settings, runs, and history stay on this
        machine.
      </p>
    </aside>
  );
}

export default Sidebar;
