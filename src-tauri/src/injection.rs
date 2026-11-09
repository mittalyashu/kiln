//! Parameter injection (SPEC §4.5).
//!
//! Turns resolved run values into environment variables and, for scripts that
//! opt in with `# @inject preamble`, into a temporary bash/zsh copy with array
//! assignments inserted after the shebang. The saved script is never edited:
//! callers execute [`PreparedScript::contents`] (or a file written by
//! [`PreparedScript::write_temporary`]) instead of rewriting the source file.

use std::collections::BTreeMap;
use std::io::{self, Write};
use std::path::Path;

use tempfile::{Builder, NamedTempFile};

use crate::params::{ParamDecl, ParamType, ScriptHeader, shebang_interpreter};

/// Parameter and secret values keyed by the name declared in the script header.
pub type ParamValues = BTreeMap<String, String>;

/// A script ready to execute with parameter values applied out of band.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedScript {
    /// The exact text to run. Equal to the saved source unless a preamble was
    /// inserted.
    pub contents: String,
    /// Whether `contents` is a generated copy rather than the saved script.
    pub is_temporary: bool,
    /// How many lines the generated preamble inserted after the shebang.
    /// Interpreter error messages refer to these shifted line numbers.
    pub line_shift: usize,
    /// A non-fatal reason a requested preamble was not generated, for example
    /// a non-bash/zsh interpreter.
    pub warning: Option<String>,
}

impl PreparedScript {
    /// The disclosure text the UI shows when a preamble shifts line numbers.
    pub fn line_shift_notice(&self) -> Option<String> {
        if self.line_shift == 0 {
            return None;
        }

        Some(format!(
            "Parameter preamble adds {} line{} after the shebang, so error line numbers are {} \
             higher than in the saved script.",
            self.line_shift,
            if self.line_shift == 1 { "" } else { "s" },
            self.line_shift
        ))
    }

    /// Writes the generated copy into a private temporary file.
    ///
    /// Returns `None` when the saved script can be run directly because no
    /// preamble was inserted. Keep the returned guard alive while the
    /// interpreter may still read the file; dropping it removes the copy.
    ///
    /// The copy is deliberately not executable: the runner must pass its path
    /// to the interpreter resolved from the shebang, as it does for the saved
    /// script. Because `$0` points at the temporary file, scripts that resolve
    /// files relative to `$0` should rely on the run's working directory
    /// instead.
    pub fn write_temporary(&self, file_name_hint: &str) -> io::Result<Option<TemporaryScript>> {
        if !self.is_temporary {
            return Ok(None);
        }

        let stem = sanitize_file_stem(file_name_hint);
        let mut file = Builder::new()
            .prefix(&format!("kiln-{stem}-"))
            .suffix(".sh")
            .tempfile()?;
        file.write_all(self.contents.as_bytes())?;

        Ok(Some(TemporaryScript { file }))
    }
}

/// A generated script file that deletes itself when dropped.
#[derive(Debug)]
pub struct TemporaryScript {
    file: NamedTempFile,
}

impl TemporaryScript {
    pub fn path(&self) -> &Path {
        self.file.path()
    }
}

/// Builds the environment additions for a run.
///
/// Every declared parameter produces a variable named exactly as declared.
/// Values fall back to the header's `default`, then to an empty string, so
/// `set -u` scripts can read optional parameters safely. List parameters also
/// produce `NAME_JSON` with a JSON array, so `<name>_JSON` is reserved by the
/// injection namespace. Secrets use the supplied value and are omitted when
/// none is available, because they have no script-side defaults.
///
/// Unset values are never re-quoted or otherwise rewritten; number and boolean
/// values remain the plain strings the caller supplied.
pub fn environment(header: &ScriptHeader, values: &ParamValues) -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();

    for param in &header.params {
        let value = param_value(param, values);
        if param.param_type == ParamType::List {
            let items = list_items(&value);
            env.insert(param.name.clone(), items.join("\n"));
            env.insert(
                format!("{}_JSON", param.name),
                serde_json::to_string(&items).expect("serializing a string array cannot fail"),
            );
        } else {
            env.insert(param.name.clone(), value);
        }
    }

    for secret in &header.secrets {
        if let Some(value) = values.get(&secret.name) {
            env.insert(secret.name.clone(), value.clone());
        }
    }

    env
}

/// Prepares `source` for execution without modifying it.
///
/// A preamble is generated only when the header opts in with
/// `# @inject preamble`, the script declares at least one list parameter, and
/// the shebang selects bash or zsh. Otherwise the source is returned unchanged
/// and `warning` explains why a requested preamble was skipped.
pub fn prepare(source: &str, header: &ScriptHeader, values: &ParamValues) -> PreparedScript {
    if !header.preamble {
        return unchanged(source);
    }

    let lists: Vec<&ParamDecl> = header
        .params
        .iter()
        .filter(|param| param.param_type == ParamType::List)
        .collect();
    if lists.is_empty() {
        return unchanged(source);
    }

    let Some(interpreter) = shebang_interpreter(source) else {
        return skipped(
            source,
            "@inject preamble was ignored because the script has no shebang; preamble injection \
             needs bash or zsh.",
        );
    };

    if !is_preamble_shell(interpreter) {
        return skipped(
            source,
            format!(
                "@inject preamble was ignored because it is only supported for bash and zsh \
                 (found \"{interpreter}\")."
            ),
        );
    }

    let newline = line_ending(source);
    let mut preamble = String::new();
    for param in &lists {
        preamble.push_str(&param.name);
        preamble.push_str("=(");
        for (index, item) in list_items(&param_value(param, values)).iter().enumerate() {
            if index > 0 {
                preamble.push(' ');
            }
            preamble.push_str(&bash_quote(item));
        }
        preamble.push(')');
        preamble.push_str(newline);
    }

    PreparedScript {
        contents: insert_after_shebang(source, &preamble, newline),
        is_temporary: true,
        line_shift: lists.len(),
        warning: None,
    }
}

fn param_value(param: &ParamDecl, values: &ParamValues) -> String {
    values
        .get(&param.name)
        .cloned()
        .or_else(|| param.default.clone())
        .unwrap_or_default()
}

/// Normalizes a list value: one item per line, blank lines ignored, items
/// trimmed (SPEC §4.5).
fn list_items(value: &str) -> Vec<String> {
    value
        .lines()
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect()
}

fn is_preamble_shell(interpreter: &str) -> bool {
    matches!(interpreter, "bash" | "zsh")
}

/// Quotes one list item for a bash/zsh array assignment.
fn bash_quote(item: &str) -> String {
    format!("'{}'", item.replace('\'', "'\\''"))
}

fn line_ending(source: &str) -> &'static str {
    match source.find('\n') {
        Some(index) if source[..index].ends_with('\r') => "\r\n",
        _ => "\n",
    }
}

/// Inserts a generated preamble directly after the shebang line, preserving
/// the source's line endings and everything after it byte for byte.
fn insert_after_shebang(source: &str, preamble: &str, newline: &str) -> String {
    let insert_at = source.find('\n').map_or(source.len(), |index| index + 1);
    let (head, tail) = source.split_at(insert_at);

    let mut contents = String::with_capacity(source.len() + preamble.len() + newline.len());
    contents.push_str(head);
    if !head.ends_with('\n') {
        contents.push_str(newline);
    }
    contents.push_str(preamble);
    contents.push_str(tail);
    contents
}

fn unchanged(source: &str) -> PreparedScript {
    PreparedScript {
        contents: source.to_string(),
        is_temporary: false,
        line_shift: 0,
        warning: None,
    }
}

fn skipped(source: &str, warning: impl Into<String>) -> PreparedScript {
    PreparedScript {
        contents: source.to_string(),
        is_temporary: false,
        line_shift: 0,
        warning: Some(warning.into()),
    }
}

fn sanitize_file_stem(hint: &str) -> String {
    let sanitized: String = hint
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = sanitized.trim_matches('-');

    if trimmed.is_empty() {
        "script".to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::parse;

    fn values(pairs: &[(&str, &str)]) -> ParamValues {
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_string(), (*value).to_string()))
            .collect()
    }

    const ALL_TYPES: &str = r#"#!/usr/bin/env bash
# @param TEXT string "Text"
# @param COUNT number "Count"
# @param DRY_RUN boolean "Dry run"
# @param ITEMS list "Items"
# @param INPUT path "Input" kind="file"
# @param MODE select "Mode" options="dev,prod"
# @secret TOKEN "Token"
"#;

    #[test]
    fn injects_every_type_as_declared() {
        let header = parse(ALL_TYPES, None);
        let env = environment(
            &header,
            &values(&[
                ("TEXT", "hello world"),
                ("COUNT", "3"),
                ("DRY_RUN", "true"),
                ("ITEMS", " a \n\nb\r\nc \n"),
                ("INPUT", "/tmp/input.txt"),
                ("MODE", "prod"),
                ("TOKEN", "s3cret"),
            ]),
        );

        assert_eq!(env["TEXT"], "hello world");
        assert_eq!(env["COUNT"], "3");
        assert_eq!(env["DRY_RUN"], "true");
        assert_eq!(env["ITEMS"], "a\nb\nc");
        assert_eq!(env["ITEMS_JSON"], r#"["a","b","c"]"#);
        assert_eq!(env["INPUT"], "/tmp/input.txt");
        assert_eq!(env["MODE"], "prod");
        assert_eq!(env["TOKEN"], "s3cret");
    }

    #[test]
    fn list_serialization_matches_the_spec() {
        let header = parse("#!/bin/bash\n# @param PRS list \"PRs\"\n", None);
        let env = environment(&header, &values(&[("PRS", "2062\n2063\n2064")]));

        assert_eq!(env["PRS"], "2062\n2063\n2064");
        assert_eq!(env["PRS_JSON"], r#"["2062","2063","2064"]"#);
    }

    #[test]
    fn escapes_list_items_in_the_json_array() {
        let header = parse("#!/bin/bash\n# @param ITEMS list \"Items\"\n", None);
        let env = environment(
            &header,
            &values(&[("ITEMS", "he said \"hi\"\nC:\\temp\\a\nline\\nbreak")]),
        );

        assert_eq!(env["ITEMS"], "he said \"hi\"\nC:\\temp\\a\nline\\nbreak");
        assert_eq!(
            env["ITEMS_JSON"],
            r#"["he said \"hi\"","C:\\temp\\a","line\\nbreak"]"#
        );
    }

    #[test]
    fn applies_defaults_and_empty_values() {
        let header = parse(
            r#"#!/bin/bash
# @param RETRIES number "Retries" default="3"
# @param NOTE string "Note"
# @param ITEMS list "Items" default="one"
"#,
            None,
        );

        let env = environment(&header, &values(&[("NOTE", "")]));
        assert_eq!(env["RETRIES"], "3");
        assert_eq!(env["NOTE"], "");
        assert_eq!(env["ITEMS"], "one");
        assert_eq!(env["ITEMS_JSON"], r#"["one"]"#);

        // A supplied value always wins over the default, including an empty one.
        let env = environment(&header, &values(&[("RETRIES", "5"), ("ITEMS", "")]));
        assert_eq!(env["RETRIES"], "5");
        assert_eq!(env["ITEMS"], "");
        assert_eq!(env["ITEMS_JSON"], "[]");
    }

    #[test]
    fn omits_missing_secrets() {
        let header = parse(
            "#!/bin/bash\n# @secret TOKEN \"Token\"\n# @param NAME string \"Name\"\n",
            None,
        );
        let env = environment(&header, &values(&[("NAME", "kiln")]));

        assert!(!env.contains_key("TOKEN"));
        assert_eq!(env["NAME"], "kiln");
    }

    #[test]
    fn prepares_a_bash_preamble_after_the_shebang() {
        let source = "#!/usr/bin/env bash\n# @inject preamble\n# @param PRS list \"PRs\"\nset -euo pipefail\necho hi\n";
        let header = parse(source, None);
        let prepared = prepare(source, &header, &values(&[("PRS", "2062\n2063")]));

        assert!(prepared.is_temporary);
        assert_eq!(prepared.line_shift, 1);
        assert_eq!(prepared.warning, None);
        assert_eq!(
            prepared.contents,
            "#!/usr/bin/env bash\nPRS=('2062' '2063')\n# @inject preamble\n# @param PRS list \"PRs\"\nset -euo pipefail\necho hi\n"
        );
        assert_eq!(
            source,
            "#!/usr/bin/env bash\n# @inject preamble\n# @param PRS list \"PRs\"\nset -euo pipefail\necho hi\n"
        );
    }

    #[test]
    fn quotes_preamble_items_safely() {
        let source = "#!/bin/bash\n# @inject preamble\n# @param ITEMS list \"Items\"\n";
        let header = parse(source, None);
        let prepared = prepare(
            source,
            &header,
            &values(&[("ITEMS", "it's fine\nhas space\n$HOME\n")]),
        );

        assert_eq!(
            prepared.contents,
            "#!/bin/bash\nITEMS=('it'\\''s fine' 'has space' '$HOME')\n# @inject preamble\n# @param ITEMS list \"Items\"\n"
        );
    }

    #[test]
    fn writes_empty_arrays_for_empty_lists() {
        let source = "#!/usr/bin/env zsh\n# @inject preamble\n# @param ITEMS list \"Items\"\n";
        let header = parse(source, None);
        let prepared = prepare(source, &header, &values(&[("ITEMS", " \n\n")]));

        assert!(prepared.is_temporary);
        assert!(
            prepared
                .contents
                .starts_with("#!/usr/bin/env zsh\nITEMS=()\n")
        );
    }

    #[test]
    fn counts_every_list_line_in_the_shift() {
        let source = "#!/bin/bash\n# @inject preamble\n# @param A list \"A\"\n# @param B list \"B\"\n# @param C string \"C\"\n";
        let header = parse(source, None);
        let prepared = prepare(
            source,
            &header,
            &values(&[("A", "1"), ("B", "2"), ("C", "x")]),
        );

        assert_eq!(prepared.line_shift, 2);
        assert_eq!(
            prepared.contents,
            "#!/bin/bash\nA=('1')\nB=('2')\n# @inject preamble\n# @param A list \"A\"\n# @param B list \"B\"\n# @param C string \"C\"\n"
        );
    }

    #[test]
    fn preserves_crlf_line_endings() {
        let source =
            "#!/bin/bash\r\n# @inject preamble\r\n# @param ITEMS list \"Items\"\r\nset -e\r\n";
        let header = parse(source, None);
        let prepared = prepare(source, &header, &values(&[("ITEMS", "a\nb")]));

        assert_eq!(
            prepared.contents,
            "#!/bin/bash\r\nITEMS=('a' 'b')\r\n# @inject preamble\r\n# @param ITEMS list \"Items\"\r\nset -e\r\n"
        );
    }

    #[test]
    fn handles_a_shebang_without_a_trailing_newline() {
        let source = "#!/bin/bash\n# @param ITEMS list \"Items\"\n# @inject preamble";
        let header = parse(source, None);
        let prepared = prepare(source, &header, &values(&[("ITEMS", "a")]));

        assert_eq!(
            prepared.contents,
            "#!/bin/bash\nITEMS=('a')\n# @param ITEMS list \"Items\"\n# @inject preamble"
        );
    }

    #[test]
    fn skips_the_preamble_for_non_shell_interpreters() {
        let source = "#!/usr/bin/env python3\n# @inject preamble\n# @param ITEMS list \"Items\"\nprint('hi')\n";
        let header = parse(source, None);
        let prepared = prepare(source, &header, &values(&[("ITEMS", "a\nb")]));

        assert!(!prepared.is_temporary);
        assert_eq!(prepared.line_shift, 0);
        assert_eq!(prepared.contents, source);
        let warning = prepared.warning.expect("warning");
        assert!(
            warning.contains("only supported for bash and zsh"),
            "{warning}"
        );
        assert!(warning.contains("python3"), "{warning}");
    }

    #[test]
    fn skips_the_preamble_without_a_shebang() {
        let source = "# @inject preamble\n# @param ITEMS list \"Items\"\n";
        let header = parse(source, None);
        let prepared = prepare(source, &header, &values(&[("ITEMS", "a")]));

        assert!(!prepared.is_temporary);
        assert_eq!(prepared.line_shift, 0);
        assert_eq!(prepared.contents, source);
        assert!(
            prepared.warning.unwrap().contains("has no shebang"),
            "expected a missing-shebang warning"
        );
    }

    #[test]
    fn skips_empty_preamble_blocks() {
        let source = "#!/bin/bash\n# @inject preamble\n# @param NAME string \"Name\"\n";
        let header = parse(source, None);
        let prepared = prepare(source, &header, &values(&[("NAME", "x")]));

        assert!(!prepared.is_temporary);
        assert_eq!(prepared.line_shift, 0);
        assert_eq!(prepared.contents, source);
        assert_eq!(prepared.warning, None);
    }

    #[test]
    fn leaves_scripts_without_the_preamble_untouched() {
        let source = "#!/bin/bash\n# @param ITEMS list \"Items\"\necho \"$ITEMS\"\n";
        let header = parse(source, None);
        let prepared = prepare(source, &header, &values(&[("ITEMS", "a\nb")]));

        assert!(!prepared.is_temporary);
        assert_eq!(prepared.line_shift, 0);
        assert_eq!(prepared.contents, source);
        assert_eq!(prepared.warning, None);
        assert_eq!(prepared.line_shift_notice(), None);
    }

    #[test]
    fn writes_and_removes_a_temporary_copy() {
        let source = "#!/bin/bash\n# @inject preamble\n# @param ITEMS list \"Items\"\necho \"${ITEMS[@]}\"\n";
        let header = parse(source, None);
        let prepared = prepare(source, &header, &values(&[("ITEMS", "a\nb")]));

        let saved_dir = tempfile::tempdir().expect("create saved dir");
        let saved_path = saved_dir.path().join("run me.sh");
        std::fs::write(&saved_path, source).expect("write saved script");

        let temporary = prepared
            .write_temporary("run me.sh")
            .expect("write temp copy")
            .expect("temporary copy");
        let temp_contents = std::fs::read_to_string(temporary.path()).expect("read temp copy");
        let saved_contents = std::fs::read_to_string(&saved_path).expect("read saved script");

        assert_eq!(temp_contents, prepared.contents);
        assert_eq!(saved_contents, source);
        assert_ne!(temporary.path(), saved_path);

        let temp_path = temporary.path().to_path_buf();
        assert!(temp_path.exists());
        drop(temporary);
        assert!(!temp_path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn keeps_the_temporary_copy_private() {
        use std::os::unix::fs::PermissionsExt;

        let source = "#!/bin/bash\n# @inject preamble\n# @param ITEMS list \"Items\"\n";
        let header = parse(source, None);
        let prepared = prepare(source, &header, &values(&[("ITEMS", "a")]));
        let temporary = prepared
            .write_temporary("script.sh")
            .expect("write temp copy")
            .expect("temporary copy");

        let mode = std::fs::metadata(temporary.path())
            .expect("stat temp copy")
            .permissions()
            .mode();

        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn writes_no_temporary_copy_without_a_preamble() {
        let source = "#!/bin/bash\n# @param ITEMS list \"Items\"\n";
        let header = parse(source, None);
        let prepared = prepare(source, &header, &values(&[("ITEMS", "a")]));

        assert!(
            prepared
                .write_temporary("script.sh")
                .expect("write temp copy")
                .is_none()
        );
    }

    #[test]
    fn sanitizes_temporary_file_names() {
        assert_eq!(sanitize_file_stem("my script.sh"), "my-script.sh");
        assert_eq!(sanitize_file_stem("../../etc/passwd"), "..-..-etc-passwd");
        assert_eq!(sanitize_file_stem(""), "script");
        assert_eq!(sanitize_file_stem("///"), "script");
    }

    #[test]
    fn describes_the_line_shift_for_the_ui() {
        let prepared = PreparedScript {
            contents: String::new(),
            is_temporary: true,
            line_shift: 1,
            warning: None,
        };
        assert_eq!(
            prepared.line_shift_notice().as_deref(),
            Some(
                "Parameter preamble adds 1 line after the shebang, so error line numbers are 1 \
                 higher than in the saved script."
            )
        );

        let prepared = PreparedScript {
            contents: String::new(),
            is_temporary: true,
            line_shift: 3,
            warning: None,
        };
        assert!(
            prepared
                .line_shift_notice()
                .unwrap()
                .contains("adds 3 lines")
        );
    }
}
