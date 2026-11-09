//! Parameter header parser and value serialization (SPEC §4.5).
//!
//! Standalone unit that understands both `#` and `//` comment headers,
//! malformed lines, and quoted descriptions. Implemented alongside the
//! parameters feature; it must stay independent of the UI and database.
