//! Script header parser (SPEC §4.5, §6.7).
//!
//! Reads metadata, parameters, and secrets from the leading comment block of a
//! script. The parser is a pure function over the file contents: callers pass
//! the script text (and optionally its path, used to infer the comment style),
//! so the library scanner, the editor, and the runner can share it without
//! pulling in UI or database dependencies.
//!
//! Both `#` (bash, sh, zsh, python, ruby, ...) and `//` (node, deno, ...)
//! comment styles are supported, chosen from the shebang first and the file
//! extension second. Malformed lines never stop the parse; they are collected
//! as [`HeaderWarning`]s so the editor can show them inline.
//!
//! ```
//! let header = kiln_lib::params::parse(
//!     "# @name Deploy\n# @param TARGET string \"Where to deploy\" required\n",
//!     None,
//! );
//!
//! assert_eq!(header.name.as_deref(), Some("Deploy"));
//! assert_eq!(header.params[0].param_type, kiln_lib::params::ParamType::String);
//! assert!(header.params[0].required);
//! assert!(header.warnings.is_empty());
//! ```

use std::collections::HashSet;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Supported v1 parameter types (SPEC §4.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ParamType {
    String,
    Number,
    Boolean,
    List,
    Path,
    Select,
}

impl ParamType {
    fn parse(raw: &str) -> Option<Self> {
        Some(match raw {
            "string" => Self::String,
            "number" => Self::Number,
            "boolean" => Self::Boolean,
            "list" => Self::List,
            "path" => Self::Path,
            "select" => Self::Select,
            _ => return None,
        })
    }
}

/// Whether a `path` parameter picks a file or a folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PathKind {
    File,
    Dir,
}

/// Comment marker used by the script's language.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CommentStyle {
    /// `#` comments: bash, sh, zsh, python, ruby, ...
    Hash,
    /// `//` comments: node, deno, ...
    Slash,
}

/// A `@param` declaration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParamDecl {
    pub name: String,
    #[serde(rename = "type")]
    pub param_type: ParamType,
    /// Empty when the declaration omitted the quoted description; a warning
    /// records the omission.
    pub description: String,
    pub required: bool,
    pub default: Option<String>,
    /// Fixed choices for a `select` parameter.
    pub options: Vec<String>,
    /// Set for `path` parameters that declared `kind="file"` or `kind="dir"`.
    pub kind: Option<PathKind>,
    /// 1-based line number of the declaration inside the script.
    pub line: usize,
}

/// A `@secret` declaration. Secrets are always masked strings (SPEC §4.6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretDecl {
    pub name: String,
    /// Empty when the declaration omitted the quoted description; a warning
    /// records the omission.
    pub description: String,
    pub required: bool,
    /// 1-based line number of the declaration inside the script.
    pub line: usize,
}

/// A structured warning for a malformed or unknown header line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeaderWarning {
    /// 1-based line number inside the script.
    pub line: usize,
    pub message: String,
    /// The offending source line, trimmed for display.
    pub raw: String,
}

/// Everything the header declares. Missing metadata stays `None`, and
/// declarations that could not be parsed are reported in `warnings` instead.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptHeader {
    pub name: Option<String>,
    pub description: Option<String>,
    pub tags: Vec<String>,
    pub params: Vec<ParamDecl>,
    pub secrets: Vec<SecretDecl>,
    pub warnings: Vec<HeaderWarning>,
    pub comment_style: CommentStyle,
    /// True when the header contains `@inject preamble` (SPEC §4.5). The
    /// injection layer decides whether the interpreter supports it.
    pub preamble: bool,
}

impl ScriptHeader {
    fn warn(&mut self, line: &HeaderLine<'_>, message: impl Into<String>) {
        self.warnings.push(HeaderWarning {
            line: line.number,
            message: message.into(),
            raw: line.raw.trim().to_string(),
        });
    }
}

/// Parses the leading comment block of a script.
///
/// `path` is only used to infer the comment style when the file has no
/// shebang; passing `None` is fine for in-memory buffers.
pub fn parse(source: &str, path: Option<&Path>) -> ScriptHeader {
    let source = source.strip_prefix('\u{feff}').unwrap_or(source);
    let comment_style = detect_comment_style(source, path);

    let mut header = ScriptHeader {
        name: None,
        description: None,
        tags: Vec::new(),
        params: Vec::new(),
        secrets: Vec::new(),
        warnings: Vec::new(),
        comment_style,
        preamble: false,
    };

    // Parameter and secret names share the environment variable namespace, so
    // a duplicate across both kinds is still ambiguous.
    let mut declared_names = HashSet::new();

    let lines = header_lines(source, comment_style);
    if lines.is_empty()
        && let Some(warning) = style_mismatch_warning(source, comment_style)
    {
        header.warnings.push(warning);
    }

    for line in lines {
        let Some(directive) = line.text.strip_prefix('@') else {
            // Plain prose comments are allowed anywhere in the header.
            continue;
        };
        let (keyword, rest) = directive
            .split_once(char::is_whitespace)
            .unwrap_or((directive, ""));

        parse_directive(&line, keyword, rest, &mut header, &mut declared_names);
    }

    header
}

/// Detects the comment style from the shebang, then from the file extension,
/// defaulting to `#` when neither is conclusive.
pub fn detect_comment_style(source: &str, path: Option<&Path>) -> CommentStyle {
    let source = source.strip_prefix('\u{feff}').unwrap_or(source);

    if let Some(style) = source.lines().next().and_then(style_from_shebang) {
        return style;
    }

    if let Some(style) = path.and_then(style_from_extension) {
        return style;
    }

    CommentStyle::Hash
}

fn parse_directive(
    line: &HeaderLine<'_>,
    keyword: &str,
    rest: &str,
    header: &mut ScriptHeader,
    declared_names: &mut HashSet<String>,
) {
    match keyword {
        "name" => {
            let Some(value) = metadata_value(line, rest, keyword, header) else {
                return;
            };
            if header.name.is_some() {
                header.warn(line, "Duplicate @name; keeping the first value.");
            } else {
                header.name = Some(value);
            }
        }
        "description" => {
            let Some(value) = metadata_value(line, rest, keyword, header) else {
                return;
            };
            if header.description.is_some() {
                header.warn(line, "Duplicate @description; keeping the first value.");
            } else {
                header.description = Some(value);
            }
        }
        "tags" => {
            let Some(value) = metadata_value(line, rest, keyword, header) else {
                return;
            };
            for tag in value.split(',') {
                let tag = tag.trim();
                if !tag.is_empty() && !header.tags.iter().any(|existing| existing == tag) {
                    header.tags.push(tag.to_string());
                }
            }
        }
        "param" => parse_param(line, rest, header, declared_names),
        "secret" => parse_secret(line, rest, header, declared_names),
        "inject" => {
            let target = rest.trim();
            if target.eq_ignore_ascii_case("preamble") {
                if header.preamble {
                    header.warn(line, "Duplicate @inject preamble; ignoring the second one.");
                } else {
                    header.preamble = true;
                }
            } else if target.is_empty() {
                header.warn(
                    line,
                    "@inject needs a target; the only supported target is \"preamble\".",
                );
            } else {
                header.warn(
                    line,
                    format!(
                        "\"{target}\" is not a supported @inject target; expected \"preamble\"."
                    ),
                );
            }
        }
        _ => header.warn(line, format!("Unknown directive \"@{keyword}\".")),
    }
}

fn parse_param(
    line: &HeaderLine<'_>,
    rest: &str,
    header: &mut ScriptHeader,
    declared_names: &mut HashSet<String>,
) {
    let tokens = match tokenize(rest) {
        Ok(tokens) => tokens,
        Err(message) => {
            header.warn(line, format!("{message}."));
            return;
        }
    };
    let mut tokens = tokens.into_iter();

    let Some(Token::Word(name)) = tokens.next() else {
        header.warn(line, "@param is missing a name.");
        return;
    };

    if !is_valid_name(&name) {
        header.warn(
            line,
            format!(
                "\"{name}\" is not a valid parameter name; use letters, digits, and \
                 underscores, starting with a letter or underscore."
            ),
        );
        return;
    }

    let Some(Token::Word(raw_type)) = tokens.next() else {
        header.warn(line, format!("@param \"{name}\" is missing a type."));
        return;
    };

    let Some(param_type) = ParamType::parse(&raw_type) else {
        header.warn(
            line,
            format!(
                "\"{raw_type}\" is not a supported parameter type (expected string, \
                 number, boolean, list, path, or select)."
            ),
        );
        return;
    };

    let mut description = String::new();
    let mut has_description = false;
    let mut required = false;
    let mut default = None;
    let mut options = Vec::new();
    let mut has_options = false;
    let mut kind = None;
    let mut has_kind = false;

    for token in tokens {
        match token {
            Token::Quoted(value) => {
                if value.is_empty() {
                    // An empty quoted description counts as missing; the
                    // post-loop check warns about it.
                    continue;
                }
                if has_description {
                    header.warn(
                        line,
                        format!("@param \"{name}\" has more than one description."),
                    );
                } else {
                    description = value;
                    has_description = true;
                }
            }
            Token::Word(word) if word == "required" => required = true,
            Token::Word(word) => header.warn(
                line,
                format!(
                    "Unexpected token \"{word}\" in @param \"{name}\"; descriptions must be \
                     quoted."
                ),
            ),
            Token::Attribute { key, value } => match key.as_str() {
                "required" => {
                    if value.is_some() {
                        header.warn(
                            line,
                            "\"required\" does not take a value; treating it as required.",
                        );
                    }
                    required = true;
                }
                "default" => {
                    let Some(value) = value else {
                        header.warn(
                            line,
                            format!("@param \"{name}\" has an empty default attribute."),
                        );
                        continue;
                    };
                    if default.is_some() {
                        header.warn(
                            line,
                            format!("@param \"{name}\" has more than one default attribute."),
                        );
                    } else {
                        default = Some(value);
                    }
                }
                "options" => {
                    if has_options {
                        header.warn(
                            line,
                            format!("@param \"{name}\" has more than one options attribute."),
                        );
                        continue;
                    }
                    has_options = true;
                    let Some(value) = value else {
                        header.warn(
                            line,
                            format!("@param \"{name}\" has an empty options attribute."),
                        );
                        continue;
                    };
                    let parsed: Vec<String> = value
                        .split(',')
                        .map(str::trim)
                        .filter(|option| !option.is_empty())
                        .map(str::to_string)
                        .collect();
                    if parsed.is_empty() {
                        header.warn(
                            line,
                            format!("@param \"{name}\" needs at least one option."),
                        );
                    } else {
                        options = parsed;
                    }
                }
                "kind" => {
                    if has_kind {
                        header.warn(
                            line,
                            format!("@param \"{name}\" has more than one kind attribute."),
                        );
                        continue;
                    }
                    has_kind = true;
                    match value.as_deref().map(str::to_ascii_lowercase).as_deref() {
                        Some("file") => kind = Some(PathKind::File),
                        Some("dir") => kind = Some(PathKind::Dir),
                        Some(other) => {
                            header.warn(
                                line,
                                format!(
                                    "\"{other}\" is not a valid kind; expected \"file\" or \"dir\"."
                                ),
                            );
                        }
                        None => {
                            header.warn(
                                line,
                                format!("@param \"{name}\" has an empty kind attribute."),
                            );
                        }
                    }
                }
                other => header.warn(
                    line,
                    format!("Unknown attribute \"{other}\" in @param \"{name}\"."),
                ),
            },
        }
    }

    if !has_description {
        header.warn(
            line,
            format!("@param \"{name}\" is missing a quoted description."),
        );
    }

    if !options.is_empty() && param_type != ParamType::Select {
        header.warn(
            line,
            format!("options are only used by select parameters (\"{name}\" is {raw_type})."),
        );
    }

    if has_kind && param_type != ParamType::Path {
        header.warn(
            line,
            format!("kind is only used by path parameters (\"{name}\" is {raw_type})."),
        );
    }

    if param_type == ParamType::Select {
        if options.is_empty() {
            header.warn(
                line,
                format!("select parameter \"{name}\" needs options=\"a,b,c\"."),
            );
        } else if let Some(default_value) = &default
            && !default_value.trim().is_empty()
            && !options.iter().any(|option| option == default_value.trim())
        {
            header.warn(
                line,
                format!("select default \"{default_value}\" is not one of the declared options."),
            );
        }
    }

    if declared_names.contains(&name) {
        header.warn(
            line,
            format!("\"{name}\" is declared more than once; keeping the first declaration."),
        );
        return;
    }
    declared_names.insert(name.clone());

    header.params.push(ParamDecl {
        name,
        param_type,
        description,
        required,
        default,
        options,
        kind,
        line: line.number,
    });
}

fn parse_secret(
    line: &HeaderLine<'_>,
    rest: &str,
    header: &mut ScriptHeader,
    declared_names: &mut HashSet<String>,
) {
    let tokens = match tokenize(rest) {
        Ok(tokens) => tokens,
        Err(message) => {
            header.warn(line, format!("{message}."));
            return;
        }
    };
    let mut tokens = tokens.into_iter();

    let Some(Token::Word(name)) = tokens.next() else {
        header.warn(line, "@secret is missing a name.");
        return;
    };

    if !is_valid_name(&name) {
        header.warn(
            line,
            format!(
                "\"{name}\" is not a valid secret name; use letters, digits, and underscores, \
                 starting with a letter or underscore."
            ),
        );
        return;
    }

    let mut description = String::new();
    let mut has_description = false;
    let mut required = false;

    for token in tokens {
        match token {
            Token::Quoted(value) => {
                if value.is_empty() {
                    // An empty quoted description counts as missing; the
                    // post-loop check warns about it.
                    continue;
                }
                if has_description {
                    header.warn(
                        line,
                        format!("@secret \"{name}\" has more than one description."),
                    );
                } else {
                    description = value;
                    has_description = true;
                }
            }
            Token::Word(word) if word == "required" => required = true,
            Token::Word(word) => header.warn(
                line,
                format!(
                    "Unexpected token \"{word}\" in @secret \"{name}\"; descriptions must be \
                     quoted."
                ),
            ),
            Token::Attribute { key, value } => {
                if key == "required" {
                    if value.is_some() {
                        header.warn(
                            line,
                            "\"required\" does not take a value; treating it as required.",
                        );
                    }
                    required = true;
                } else {
                    header.warn(
                        line,
                        format!("Unknown attribute \"{key}\" in @secret \"{name}\"."),
                    );
                }
            }
        }
    }

    if !has_description {
        header.warn(
            line,
            format!("@secret \"{name}\" is missing a quoted description."),
        );
    }

    if declared_names.contains(&name) {
        header.warn(
            line,
            format!("\"{name}\" is declared more than once; keeping the first declaration."),
        );
        return;
    }
    declared_names.insert(name.clone());

    header.secrets.push(SecretDecl {
        name,
        description,
        required,
        line: line.number,
    });
}

/// Returns the value of `@name`, `@description`, or `@tags`, unquoting a
/// leading quoted string and warning about text that trails it.
fn metadata_value(
    line: &HeaderLine<'_>,
    rest: &str,
    keyword: &str,
    header: &mut ScriptHeader,
) -> Option<String> {
    let rest = rest.trim();
    if rest.is_empty() {
        header.warn(line, format!("@{keyword} needs a value."));
        return None;
    }

    // Only double quotes delimit metadata values; a leading apostrophe is
    // ordinary prose (e.g. `@description 'Twas ...`).
    if !rest.starts_with('"') {
        return Some(rest.to_string());
    }

    match tokenize(rest) {
        Ok(tokens) => match tokens.split_first() {
            Some((Token::Quoted(value), remaining)) => {
                if value.is_empty() {
                    header.warn(line, format!("@{keyword} needs a value."));
                    return None;
                }
                if !remaining.is_empty() {
                    header.warn(
                        line,
                        format!("Unexpected text after the quoted @{keyword} value."),
                    );
                }
                Some(value.clone())
            }
            // Unreachable: the input starts with a quote, so the first token
            // is always quoted.
            _ => Some(rest.to_string()),
        },
        Err(message) => {
            header.warn(line, format!("{message}."));
            None
        }
    }
}

fn is_valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };

    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// A comment line inside the leading header block.
struct HeaderLine<'a> {
    /// 1-based line number.
    number: usize,
    /// Comment payload with the marker and surrounding whitespace removed.
    text: &'a str,
    /// The original source line.
    raw: &'a str,
}

fn header_lines(source: &str, style: CommentStyle) -> Vec<HeaderLine<'_>> {
    let mut lines = Vec::new();

    for (index, raw) in source.lines().enumerate() {
        if index == 0 && raw.starts_with("#!") {
            continue;
        }

        let trimmed = raw.trim();
        if trimmed.is_empty() {
            // Blank lines are allowed inside the header block.
            continue;
        }

        let Some(payload) = comment_payload(trimmed, style) else {
            // The leading comment block ends at the first line of code.
            break;
        };

        lines.push(HeaderLine {
            number: index + 1,
            text: payload.trim(),
            raw,
        });
    }

    lines
}

fn comment_payload(line: &str, style: CommentStyle) -> Option<&str> {
    match style {
        CommentStyle::Hash => line.strip_prefix('#'),
        CommentStyle::Slash => line.strip_prefix("//"),
    }
}

/// Detects the common mistake of writing the header with the other comment
/// style, which would otherwise parse to an empty header with no explanation.
fn style_mismatch_warning(source: &str, style: CommentStyle) -> Option<HeaderWarning> {
    let (other_marker, expected_marker) = match style {
        CommentStyle::Hash => ("//", "#"),
        CommentStyle::Slash => ("#", "//"),
    };

    let (index, raw) = source
        .lines()
        .enumerate()
        .find(|(index, raw)| !(*index == 0 && raw.starts_with("#!")) && !raw.trim().is_empty())?;

    let trimmed = raw.trim();
    if !trimmed.starts_with(other_marker) {
        return None;
    }

    Some(HeaderWarning {
        line: index + 1,
        message: format!(
            "Header line uses \"{other_marker}\" comments but this script's comment style is \
             \"{expected_marker}\"."
        ),
        raw: trimmed.to_string(),
    })
}

fn style_from_shebang(line: &str) -> Option<CommentStyle> {
    let command = line.strip_prefix("#!")?;
    Some(if is_javascript_runtime(interpreter_name(command)) {
        CommentStyle::Slash
    } else {
        CommentStyle::Hash
    })
}

fn interpreter_name(command: &str) -> &str {
    let mut parts = command.split_whitespace();
    let mut program = parts.next().unwrap_or_default();

    if basename(program) == "env" {
        // `env` forwards to the first argument that is neither a flag nor a
        // `NAME=value` assignment (e.g. `env -S node`).
        if let Some(next) = parts.find(|arg| !arg.starts_with('-') && !arg.contains('=')) {
            program = next;
        }
    }

    basename(program)
}

fn basename(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

fn is_javascript_runtime(interpreter: &str) -> bool {
    let base = interpreter.trim_end_matches(|c: char| c.is_ascii_digit());
    matches!(base, "node" | "nodejs" | "deno" | "bun" | "tsx" | "ts-node")
}

fn style_from_extension(path: &Path) -> Option<CommentStyle> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    match extension.as_str() {
        "js" | "mjs" | "cjs" | "jsx" | "ts" | "mts" | "cts" | "tsx" => Some(CommentStyle::Slash),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    /// A quoted string, with escapes already resolved.
    Quoted(String),
    /// A bare word: a parameter name, type, or the `required` flag.
    Word(String),
    /// A `key=value` attribute; `value` is `None` for a bare `key=`.
    Attribute { key: String, value: Option<String> },
}

fn tokenize(input: &str) -> Result<Vec<Token>, String> {
    let mut scanner = Scanner { input, pos: 0 };
    let mut tokens = Vec::new();

    while let Some(token) = scanner.next_token()? {
        tokens.push(token);
    }

    Ok(tokens)
}

/// Splits a directive payload into words, quoted strings, and attributes.
/// Quoted strings support escaped quotes (`\"`, `\'`) and escaped backslashes;
/// any other escape is kept literally, so Windows-style paths survive. A
/// literal backslash must be doubled, including one just before the closing
/// quote (`default="C:\\temp\\"`).
struct Scanner<'a> {
    input: &'a str,
    pos: usize,
}

impl<'a> Scanner<'a> {
    fn rest(&self) -> &'a str {
        &self.input[self.pos..]
    }

    fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    fn skip_whitespace(&mut self) {
        while let Some(c) = self.peek() {
            if !c.is_whitespace() {
                break;
            }
            self.bump();
        }
    }

    fn next_token(&mut self) -> Result<Option<Token>, String> {
        self.skip_whitespace();

        let Some(c) = self.peek() else {
            return Ok(None);
        };

        if c == '"' || c == '\'' {
            return Ok(Some(Token::Quoted(self.read_quoted()?)));
        }

        let start = self.pos;
        while let Some(c) = self.peek() {
            if c.is_whitespace() {
                break;
            }

            match c {
                '=' => {
                    let key = self.input[start..self.pos].to_string();
                    self.bump();
                    let value = if matches!(self.peek(), Some('"') | Some('\'')) {
                        Some(self.read_quoted()?)
                    } else {
                        let value_start = self.pos;
                        while let Some(v) = self.peek() {
                            if v.is_whitespace() {
                                break;
                            }
                            self.bump();
                        }
                        let value = &self.input[value_start..self.pos];
                        // `key=` with nothing after it has no value, while
                        // `key=""` is an explicit empty string.
                        if value.is_empty() {
                            None
                        } else {
                            Some(value.to_string())
                        }
                    };
                    return Ok(Some(Token::Attribute { key, value }));
                }
                '"' | '\'' => return Err("unexpected quote inside a word".to_string()),
                _ => {
                    self.bump();
                }
            }
        }

        Ok(Some(Token::Word(self.input[start..self.pos].to_string())))
    }

    fn read_quoted(&mut self) -> Result<String, String> {
        let quote = self
            .bump()
            .ok_or_else(|| "unterminated quoted string".to_string())?;
        let mut value = String::new();

        loop {
            match self.bump() {
                None => return Err("unterminated quoted string".to_string()),
                Some(c) if c == quote => return Ok(value),
                Some('\\') => match self.bump() {
                    None => return Err("unterminated quoted string".to_string()),
                    Some(next) if next == quote || next == '\\' => value.push(next),
                    Some(next) => {
                        value.push('\\');
                        value.push(next);
                    }
                },
                Some(c) => value.push(c),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPEC_EXAMPLE: &str = r#"#!/usr/bin/env bash
# @name        Re-run failed CI for PRs
# @description Re-runs failed workflow runs on the latest commit of each PR.
# @tags        github, ci
# @param PRS        list     "PR numbers to process"   required
# @param ENV        select   "Target environment"      options="dev,staging,prod" default="dev"
# @param DRY_RUN    boolean  "Print instead of run"    default="false"
# @param RETRIES    number   "Retry count"             default="3"
# @param INPUT_DIR  path     "Input directory"         kind="dir"
# @secret GITHUB_TOKEN "GitHub token"
set -euo pipefail
"#;

    fn find_param<'a>(header: &'a ScriptHeader, name: &str) -> &'a ParamDecl {
        header
            .params
            .iter()
            .find(|param| param.name == name)
            .unwrap_or_else(|| panic!("parameter {name} missing"))
    }

    fn warning_text(header: &ScriptHeader) -> String {
        header
            .warnings
            .iter()
            .map(|warning| warning.message.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn parses_the_spec_example() {
        let header = parse(SPEC_EXAMPLE, None);

        assert_eq!(header.comment_style, CommentStyle::Hash);
        assert_eq!(header.name.as_deref(), Some("Re-run failed CI for PRs"));
        assert_eq!(
            header.description.as_deref(),
            Some("Re-runs failed workflow runs on the latest commit of each PR.")
        );
        assert_eq!(header.tags, ["github", "ci"]);
        assert!(header.warnings.is_empty(), "{:?}", header.warnings);
        assert_eq!(header.params.len(), 5);
        assert_eq!(header.secrets.len(), 1);

        let prs = find_param(&header, "PRS");
        assert_eq!(prs.param_type, ParamType::List);
        assert_eq!(prs.description, "PR numbers to process");
        assert!(prs.required);
        assert_eq!(prs.default, None);
        assert_eq!(prs.line, 5);

        let env = find_param(&header, "ENV");
        assert_eq!(env.param_type, ParamType::Select);
        assert_eq!(env.description, "Target environment");
        assert_eq!(env.options, ["dev", "staging", "prod"]);
        assert_eq!(env.default.as_deref(), Some("dev"));
        assert!(!env.required);

        let dry_run = find_param(&header, "DRY_RUN");
        assert_eq!(dry_run.param_type, ParamType::Boolean);
        assert_eq!(dry_run.default.as_deref(), Some("false"));

        let retries = find_param(&header, "RETRIES");
        assert_eq!(retries.param_type, ParamType::Number);
        assert_eq!(retries.default.as_deref(), Some("3"));

        let input_dir = find_param(&header, "INPUT_DIR");
        assert_eq!(input_dir.param_type, ParamType::Path);
        assert_eq!(input_dir.kind, Some(PathKind::Dir));

        let secret = &header.secrets[0];
        assert_eq!(secret.name, "GITHUB_TOKEN");
        assert_eq!(secret.description, "GitHub token");
        assert!(!secret.required);
        assert_eq!(secret.line, 10);
    }

    #[test]
    fn parses_slash_comments_for_node() {
        let source = r#"#!/usr/bin/env node
// @name Deploy service
// @description Deploys the service to staging.
// @tags deploy, node
// @param ENV select "Target environment" options="dev,staging,prod" default="dev"
// @param FORCE boolean "Skip checks"
// @secret DEPLOY_TOKEN "Deploy token" required
console.log("hi");
"#;
        let header = parse(source, None);

        assert_eq!(header.comment_style, CommentStyle::Slash);
        assert_eq!(header.name.as_deref(), Some("Deploy service"));
        assert_eq!(header.tags, ["deploy", "node"]);
        assert_eq!(header.params.len(), 2);
        assert_eq!(header.params[0].line, 5);
        assert!(header.secrets[0].required);
        assert!(header.warnings.is_empty(), "{:?}", header.warnings);
    }

    #[test]
    fn selects_comment_style_from_shebang_then_extension() {
        assert_eq!(
            parse("#!/usr/bin/env node\n", None).comment_style,
            CommentStyle::Slash
        );
        assert_eq!(
            parse("#!/usr/bin/env -S deno run --allow-net\n", None).comment_style,
            CommentStyle::Slash
        );
        assert_eq!(
            parse("#!/usr/bin/env python3\n", None).comment_style,
            CommentStyle::Hash
        );
        assert_eq!(
            parse("", Some(Path::new("scripts/deploy.ts"))).comment_style,
            CommentStyle::Slash
        );
        assert_eq!(
            parse("", Some(Path::new("scripts/deploy.sh"))).comment_style,
            CommentStyle::Hash
        );
        // An explicit shebang wins over the extension.
        assert_eq!(
            parse("#! /bin/bash\n", Some(Path::new("scripts/deploy.js"))).comment_style,
            CommentStyle::Hash
        );
    }

    #[test]
    fn supports_every_v1_parameter_type() {
        let source = r#"#!/bin/sh
# @param TEXT string "Text"
# @param COUNT number "Count"
# @param ENABLED boolean "Enabled"
# @param ITEMS list "Items"
# @param FILE path "File" kind="file"
# @param MODE select "Mode" options="a,b"
"#;
        let header = parse(source, None);
        let types: Vec<ParamType> = header.params.iter().map(|param| param.param_type).collect();

        assert_eq!(
            types,
            [
                ParamType::String,
                ParamType::Number,
                ParamType::Boolean,
                ParamType::List,
                ParamType::Path,
                ParamType::Select,
            ]
        );
        assert_eq!(find_param(&header, "FILE").kind, Some(PathKind::File));
        assert_eq!(find_param(&header, "MODE").options, ["a", "b"]);
        assert!(header.warnings.is_empty(), "{:?}", header.warnings);
    }

    #[test]
    fn parses_attributes_in_any_order() {
        let source = r#"#!/bin/bash
# @param RETRIES number default="3" required "Retry count"
# @param ENV select default="dev" options="dev,prod" "Environment"
"#;
        let header = parse(source, None);

        let retries = find_param(&header, "RETRIES");
        assert_eq!(retries.description, "Retry count");
        assert!(retries.required);
        assert_eq!(retries.default.as_deref(), Some("3"));

        let env = find_param(&header, "ENV");
        assert_eq!(env.description, "Environment");
        assert_eq!(env.default.as_deref(), Some("dev"));
        assert_eq!(env.options, ["dev", "prod"]);
        assert!(header.warnings.is_empty(), "{:?}", header.warnings);
    }

    #[test]
    fn handles_escaped_and_single_quotes() {
        let source = r#"#!/bin/bash
# @param GREETING string "Say \"hello\" and \\ bye" default="a\"b"
# @param SINGLE string 'It\'s fine'
"#;
        let header = parse(source, None);

        let greeting = find_param(&header, "GREETING");
        assert_eq!(greeting.description, r#"Say "hello" and \ bye"#);
        assert_eq!(greeting.default.as_deref(), Some(r#"a"b"#));

        let single = find_param(&header, "SINGLE");
        assert_eq!(single.description, "It's fine");
        assert!(header.warnings.is_empty(), "{:?}", header.warnings);
    }

    #[test]
    fn allows_blank_lines_inside_the_header() {
        let source = r#"#!/bin/bash

# @name Blank lines

# @param ITEMS list "Items"

set -euo pipefail
"#;
        let header = parse(source, None);

        assert_eq!(header.name.as_deref(), Some("Blank lines"));
        assert_eq!(header.params.len(), 1);
        assert_eq!(header.params[0].line, 5);
        assert!(header.warnings.is_empty(), "{:?}", header.warnings);
    }

    #[test]
    fn ignores_plain_comment_lines() {
        let source = r#"#!/bin/bash
# This script re-runs failed CI jobs.
#   It mentions @param NOPE string "x" in prose.
# @name Named
"#;
        let header = parse(source, None);

        assert_eq!(header.name.as_deref(), Some("Named"));
        assert!(header.params.is_empty());
        assert!(header.warnings.is_empty(), "{:?}", header.warnings);
    }

    #[test]
    fn stops_at_the_first_non_comment_line() {
        let source = r#"#!/bin/bash
# @name Before
set -e
# @name After
"#;
        let header = parse(source, None);

        assert_eq!(header.name.as_deref(), Some("Before"));
    }

    #[test]
    fn warns_on_unknown_directives() {
        let source = "#!/bin/bash\n# @name Named\n# @bogus value\n";
        let header = parse(source, None);

        assert_eq!(header.warnings.len(), 1, "{:?}", header.warnings);
        let warning = &header.warnings[0];
        assert_eq!(warning.line, 3);
        assert!(warning.message.contains("@bogus"));
        assert_eq!(warning.raw, "# @bogus value");
    }

    #[test]
    fn warns_on_malformed_parameters() {
        let source = r#"#!/bin/bash
# @param
# @param FOO
# @param FOO nope "Bad type"
# @param 1BAD string "Bad name"
# @param OK select "No options"
"#;
        let header = parse(source, None);
        let text = warning_text(&header);

        assert_eq!(header.warnings.len(), 5, "{:?}", header.warnings);
        assert!(text.contains("@param is missing a name."));
        assert!(text.contains("@param \"FOO\" is missing a type."));
        assert!(text.contains("not a supported parameter type"));
        assert!(text.contains("not a valid parameter name"));
        assert!(text.contains("needs options"));
        assert_eq!(header.params.len(), 1);
        assert_eq!(header.params[0].name, "OK");
    }

    #[test]
    fn warns_on_unterminated_quotes() {
        let header = parse("#!/bin/bash\n# @param NAME string \"oops\n", None);

        assert!(header.params.is_empty());
        assert!(warning_text(&header).contains("unterminated quoted string"));

        let header = parse("#!/bin/bash\n# @name \"oops\n", None);

        assert_eq!(header.name, None);
        assert!(warning_text(&header).contains("unterminated quoted string"));
    }

    #[test]
    fn warns_on_unknown_attributes() {
        let source = r#"#!/bin/bash
# @param NAME string "N" wat="1"
# @secret TOKEN "T" default="x"
"#;
        let header = parse(source, None);
        let text = warning_text(&header);

        assert!(text.contains("Unknown attribute \"wat\""));
        assert!(text.contains("Unknown attribute \"default\""));
        assert_eq!(header.params.len(), 1);
        assert_eq!(header.secrets.len(), 1);
    }

    #[test]
    fn warns_on_duplicate_names_across_params_and_secrets() {
        let source = r#"#!/bin/bash
# @param NAME string "N"
# @secret NAME "S"
# @param NAME string "Again"
# @secret OTHER "O"
# @secret OTHER "Again"
"#;
        let header = parse(source, None);

        assert_eq!(header.params.len(), 1);
        assert_eq!(header.secrets.len(), 1);
        assert_eq!(header.secrets[0].name, "OTHER");
        assert_eq!(header.warnings.len(), 3, "{:?}", header.warnings);
        assert!(warning_text(&header).contains("declared more than once"));
    }

    #[test]
    fn warns_on_select_defaults_outside_options() {
        let source = r#"#!/bin/bash
# @param ENV select "Env" options="dev,prod" default="staging"
"#;
        let header = parse(source, None);

        assert_eq!(header.params.len(), 1);
        assert!(
            warning_text(&header).contains("not one of the declared options"),
            "{:?}",
            header.warnings
        );
    }

    #[test]
    fn warns_on_invalid_kind() {
        let header = parse(
            "#!/bin/bash\n# @param PATH path \"P\" kind=\"folder\"\n",
            None,
        );

        assert_eq!(header.params.len(), 1);
        assert_eq!(header.params[0].kind, None);
        assert!(
            warning_text(&header).contains("expected \"file\" or \"dir\""),
            "{:?}",
            header.warnings
        );

        let header = parse(
            "#!/bin/bash\n# @param NAME string \"N\" kind=\"file\"\n",
            None,
        );

        assert!(
            warning_text(&header).contains("kind is only used by path parameters"),
            "{:?}",
            header.warnings
        );
    }

    #[test]
    fn parses_secrets() {
        let source = r#"#!/usr/bin/env bash
# @secret GITHUB_TOKEN "GitHub token"
# @secret NPM_TOKEN "NPM token" required
"#;
        let header = parse(source, None);

        assert_eq!(header.secrets.len(), 2);
        assert_eq!(header.secrets[0].name, "GITHUB_TOKEN");
        assert_eq!(header.secrets[0].description, "GitHub token");
        assert!(!header.secrets[0].required);
        assert!(header.secrets[1].required);
        assert_eq!(header.secrets[1].line, 3);
        assert!(header.warnings.is_empty(), "{:?}", header.warnings);
    }

    #[test]
    fn parses_metadata_quoting() {
        let source = r#"#!/bin/bash
# @name "Quoted name"
# @description "Quoted description"
# @tags "github, ci"
"#;
        let header = parse(source, None);

        assert_eq!(header.name.as_deref(), Some("Quoted name"));
        assert_eq!(header.description.as_deref(), Some("Quoted description"));
        assert_eq!(header.tags, ["github", "ci"]);
        assert!(header.warnings.is_empty(), "{:?}", header.warnings);
    }

    #[test]
    fn handles_crlf_line_endings() {
        let source = "#!/usr/bin/env bash\r\n# @name CRLF\r\n# @param X string \"X\"\r\n";
        let header = parse(source, None);

        assert_eq!(header.name.as_deref(), Some("CRLF"));
        assert_eq!(header.params.len(), 1);
        assert_eq!(header.params[0].description, "X");
        assert_eq!(header.params[0].line, 3);
        assert!(header.warnings.is_empty(), "{:?}", header.warnings);
    }

    #[test]
    fn returns_an_empty_header_for_an_empty_script() {
        for source in ["", "#!/usr/bin/env bash\n"] {
            let header = parse(source, None);

            assert_eq!(header.comment_style, CommentStyle::Hash);
            assert_eq!(header.name, None);
            assert_eq!(header.description, None);
            assert!(header.tags.is_empty());
            assert!(header.params.is_empty());
            assert!(header.secrets.is_empty());
            assert!(header.warnings.is_empty());
            assert!(!header.preamble);
        }
    }

    #[test]
    fn parses_inject_preamble() {
        let header = parse("#!/bin/bash\n# @inject preamble\n", None);

        assert!(header.preamble);
        assert!(header.warnings.is_empty(), "{:?}", header.warnings);

        let header = parse("#!/bin/bash\n# @inject env\n", None);

        assert!(!header.preamble);
        assert!(warning_text(&header).contains("not a supported @inject target"));

        let header = parse("#!/bin/bash\n# @inject\n", None);

        assert!(!header.preamble);
        assert!(warning_text(&header).contains("needs a target"));
    }

    #[test]
    fn warns_on_empty_attribute_values() {
        let source = r#"#!/bin/bash
# @param COUNT number "Count" default=
# @param DIR path "Dir" kind=
# @param MODE select "Mode" options=
"#;
        let header = parse(source, None);
        let text = warning_text(&header);

        assert!(text.contains("empty default attribute"), "{text}");
        assert!(text.contains("empty kind attribute"), "{text}");
        assert!(text.contains("empty options attribute"), "{text}");
        assert_eq!(find_param(&header, "COUNT").default, None);
        assert_eq!(find_param(&header, "DIR").kind, None);
    }

    #[test]
    fn warns_on_duplicate_attributes() {
        let source = r#"#!/bin/bash
# @param MODE select "Mode" options="a" options="b" default="a" default="b"
# @param FILE path "File" kind="file" kind="dir"
"#;
        let header = parse(source, None);
        let text = warning_text(&header);

        assert!(text.contains("more than one options attribute"), "{text}");
        assert!(text.contains("more than one default attribute"), "{text}");
        assert!(text.contains("more than one kind attribute"), "{text}");
        assert_eq!(find_param(&header, "MODE").options, ["a"]);
        assert_eq!(find_param(&header, "MODE").default.as_deref(), Some("a"));
        assert_eq!(find_param(&header, "FILE").kind, Some(PathKind::File));
    }

    #[test]
    fn warns_on_empty_quoted_descriptions() {
        let source = r#"#!/bin/bash
# @name ""
# @param NAME string ""
# @secret TOKEN ""
"#;
        let header = parse(source, None);
        let text = warning_text(&header);

        assert_eq!(header.name, None);
        assert!(text.contains("@name needs a value."), "{text}");
        assert!(
            text.contains("@param \"NAME\" is missing a quoted description."),
            "{text}"
        );
        assert!(
            text.contains("@secret \"TOKEN\" is missing a quoted description."),
            "{text}"
        );
        assert_eq!(header.params[0].description, "");
        assert_eq!(header.secrets[0].description, "");
    }

    #[test]
    fn treats_leading_apostrophes_in_metadata_as_prose() {
        let header = parse("#!/bin/bash\n# @description 'Twas the night\n", None);

        assert_eq!(header.description.as_deref(), Some("'Twas the night"));
        assert!(header.warnings.is_empty(), "{:?}", header.warnings);
    }

    #[test]
    fn handles_a_byte_order_mark() {
        let source = "\u{feff}#!/usr/bin/env bash\n# @name BOM\n";
        let header = parse(source, None);

        assert_eq!(header.comment_style, CommentStyle::Hash);
        assert_eq!(header.name.as_deref(), Some("BOM"));
        assert!(header.warnings.is_empty(), "{:?}", header.warnings);
    }

    #[test]
    fn parses_slash_header_detected_from_path() {
        let source = "// @name From extension\n// @param NAME string \"N\"\n";
        let header = parse(source, Some(Path::new("scripts/tool.mjs")));

        assert_eq!(header.comment_style, CommentStyle::Slash);
        assert_eq!(header.name.as_deref(), Some("From extension"));
        assert_eq!(header.params.len(), 1);
        assert!(header.warnings.is_empty(), "{:?}", header.warnings);
    }

    #[test]
    fn warns_when_the_header_uses_the_other_comment_style() {
        let header = parse("# @name Wrong style\n", Some(Path::new("scripts/tool.ts")));

        assert_eq!(header.comment_style, CommentStyle::Slash);
        assert!(header.name.is_none());
        assert_eq!(header.warnings.len(), 1, "{:?}", header.warnings);
        assert_eq!(header.warnings[0].line, 1);
        assert!(
            header.warnings[0]
                .message
                .contains("comment style is \"//\""),
            "{:?}",
            header.warnings
        );
    }

    #[test]
    fn trims_select_defaults_before_comparing_options() {
        let header = parse(
            "#!/bin/bash\n# @param MODE select \"Mode\" options=\"a, b\" default=\" b\"\n",
            None,
        );

        assert_eq!(header.params.len(), 1);
        assert!(header.warnings.is_empty(), "{:?}", header.warnings);
    }

    #[test]
    fn merges_tags_without_duplicates() {
        let header = parse(
            "#!/bin/bash\n# @tags github, ci\n# @tags ci, deploy\n",
            None,
        );

        assert_eq!(header.tags, ["github", "ci", "deploy"]);
    }

    #[test]
    fn serializes_a_camel_case_contract_for_the_frontend() {
        let header = parse(SPEC_EXAMPLE, None);
        let json = serde_json::to_value(&header).expect("serialize header");

        assert_eq!(json["commentStyle"], "hash");
        assert_eq!(json["preamble"], false);
        assert_eq!(json["params"][0]["name"], "PRS");
        assert_eq!(json["params"][0]["type"], "list");
        assert_eq!(json["params"][0]["description"], "PR numbers to process");
        assert_eq!(json["params"][0]["required"], true);
        assert!(json["params"][0]["kind"].is_null());
        assert_eq!(json["secrets"][0]["name"], "GITHUB_TOKEN");
    }
}
