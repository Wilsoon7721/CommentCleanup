//! Finds the function a comment belongs to without a parser for each language

use crate::highlight::{Document, Hunk};
use fancy_regex::Regex;
use std::sync::LazyLock;

const MAX_HEADER_LINES: usize = 40;
const MAX_BODY_LINES: usize = 20_000;
const MAX_UPWARD_LINES: usize = 20_000;
const MAX_CANDIDATES: usize = 300;
const MAX_GAP_LINES: usize = 6;
const MAX_LINES: usize = 300;
const HEAD_LINES: usize = 30;
const AROUND_COMMENT: usize = 120;
const MAX_CHARS: usize = 20_000;

#[derive(Clone, Debug, PartialEq)]
pub struct FunctionSource {
    pub first: usize,
    pub last: usize,
    pub text: String,
    pub partial: bool,
}

const INDENT_LANGUAGES: [&str; 12] = [
    "python", "ruby", "lua", "elixir", "nim", "gdscript", "coffee", "julia", "crystal", "vb",
    "matlab", "haml",
];

static KEYWORD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(function|fn|def|func|fun|sub|proc)\b\s*[\w$*<(\[]").expect("built-in pattern")
});
static ARROW: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"=>\s*\{\s*$").expect("built-in pattern"));
static C_STYLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?x)^\s*(?:[\w$<>\[\],.*&:?~]+\s+)*?(?P<name>[\w$~]+)\s*(?:<[^()]*>\s*)?\(
          (?:.*\)\s*(?:(?:->|:)\s*[^{;]+?|throws\s+[^{;]+?|const|override|noexcept|final|\s)*\s*\{?\s*$|[^)]*$)",
    )
    .expect("built-in pattern")
});

const NOT_FUNCTIONS: [&str; 21] = [
    "if", "for", "while", "switch", "catch", "return", "else", "do", "using", "lock", "foreach",
    "with", "match", "synchronized", "when", "until", "unless", "elif", "new", "throw", "await",
];

fn code_of(doc: &Document, line: usize) -> String {
    doc.lines[line]
        .segments
        .iter()
        .filter(|segment| !segment.comment)
        .map(|segment| segment.text.as_str())
        .collect()
}

fn indent_of(text: &str) -> usize {
    text.len() - text.trim_start().len()
}

fn next_code_line(doc: &Document, after: usize) -> Option<usize> {
    (after + 1..doc.lines.len().min(after + 1 + MAX_GAP_LINES)).find(|&line| doc.lines[line].has_code())
}

fn uses_indentation(language: &str) -> bool {
    let language = language.to_lowercase();
    INDENT_LANGUAGES.iter().any(|name| language.contains(name))
}

fn header_column(doc: &Document, line: usize, indented: bool) -> Option<usize> {
    let code = code_of(doc, line);
    let found = |regex: &Regex| regex.find(&code).ok().flatten().map(|m| m.start());
    if let Some(column) = found(&KEYWORD) {
        return Some(column);
    }
    if indented {
        return None;
    }
    if let Some(column) = found(&ARROW) {
        return Some(column);
    }
    let captures = C_STYLE.captures(&code).ok().flatten()?;
    let name = captures.name("name")?;
    if NOT_FUNCTIONS.contains(&name.as_str()) {
        return None;
    }
    let trimmed = code.trim_end();
    let open = trimmed.ends_with('{') || trimmed.matches('(').count() > trimmed.matches(')').count();
    let brace_next = next_code_line(doc, line).is_some_and(|next| code_of(doc, next).trim() == "{");
    (open || brace_next).then_some(name.start())
}

fn without_strings(code: &str, language: &str) -> String {
    let single_quotes = !language.to_lowercase().contains("rust");
    let mut out = String::with_capacity(code.len());
    let mut quote = None;
    let mut chars = code.chars();
    while let Some(c) = chars.next() {
        match quote {
            Some(_) if c == '\\' => {
                chars.next();
            }
            Some(open) if c == open => quote = None,
            Some(_) => {}
            None if c == '"' || c == '`' || (c == '\'' && single_quotes) => quote = Some(c),
            None => out.push(c),
        }
    }
    out
}

fn brace_end(doc: &Document, line: usize, column: usize) -> Option<usize> {
    let (mut parens, mut braces, mut opened) = (0usize, 0usize, false);
    let limit = doc.lines.len().min(line + MAX_BODY_LINES);
    for current in line..limit {
        let code = code_of(doc, current);
        let code = if current == line { code.get(column..).unwrap_or("") } else { code.as_str() };
        for c in without_strings(code, &doc.language).chars() {
            match c {
                '(' | '[' => parens += 1,
                ')' | ']' => parens = parens.saturating_sub(1),
                '{' if opened || parens == 0 => {
                    opened = true;
                    braces += 1;
                }
                '}' if opened => {
                    braces -= 1;
                    if braces == 0 {
                        return Some(current);
                    }
                }
                ';' if !opened && parens == 0 => return None,
                _ => {}
            }
        }
        if !opened && current >= line + MAX_HEADER_LINES {
            return None;
        }
    }
    None
}

fn indent_end(doc: &Document, line: usize) -> usize {
    let width = indent_of(&code_of(doc, line));
    let mut last = line;
    for current in line + 1..doc.lines.len().min(line + MAX_BODY_LINES) {
        if !doc.lines[current].has_code() {
            continue;
        }
        let code = code_of(doc, current);
        let trimmed = code.trim_start();
        let indent = indent_of(&code);
        if indent > width {
            last = current;
        } else if indent == width && trimmed.starts_with(['(', ')', ']']) {
            last = current;
        } else {
            if indent == width && trimmed.split_whitespace().next().is_some_and(|w| w.starts_with("end")) {
                last = current;
            }
            break;
        }
    }
    last
}

/// The last line of the function starting here, if a function starts here.
fn function_end(doc: &Document, line: usize, indented: bool) -> Option<usize> {
    let column = header_column(doc, line, indented)?;
    if indented { Some(indent_end(doc, line)) } else { brace_end(doc, line, column) }
}

fn range_of(doc: &Document, hunk: &Hunk) -> Option<(usize, usize)> {
    let indented = uses_indentation(&doc.language);
    if hunk.whole_line {
        let mut line = hunk.last;
        for _ in 0..MAX_GAP_LINES {
            let Some(next) = next_code_line(doc, line) else {
                break;
            };
            if let Some(end) = function_end(doc, next, indented) {
                return Some((next, end));
            }
            let code = code_of(doc, next);
            let code = code.trim_start();
            if !(code.starts_with('@') || code.starts_with("#[") || code.starts_with('[')) {
                break;
            }
            line = next;
        }
    }
    let lowest = hunk.first.saturating_sub(MAX_UPWARD_LINES);
    let mut tried = 0;
    for line in (lowest..=hunk.first).rev() {
        if !doc.lines[line].has_code() {
            continue;
        }
        if header_column(doc, line, indented).is_none() {
            continue;
        }
        tried += 1;
        if tried > MAX_CANDIDATES {
            return None;
        }
        if let Some(end) = function_end(doc, line, indented).filter(|&end| end >= hunk.last) {
            return Some((line, end));
        }
    }
    None
}

fn render(doc: &Document, lines: impl Iterator<Item = usize>) -> String {
    lines.map(|line| doc.lines[line].text()).collect::<Vec<_>>().join("\n")
}

fn cut_to_chars(text: String) -> (String, bool) {
    match text.char_indices().nth(MAX_CHARS) {
        Some((end, _)) => (text[..end].to_owned(), true),
        None => (text, false),
    }
}

/// The source of the function this comment documents or sits inside.
pub fn containing(doc: &Document, hunk: &Hunk) -> Option<FunctionSource> {
    let (first, last) = range_of(doc, hunk)?;
    let inside = hunk.first >= first && hunk.last <= last;
    let (text, shortened) = if last - first < MAX_LINES {
        (render(doc, first..=last), false)
    } else {
        let head_end = first + HEAD_LINES;
        let mut text = render(doc, first..head_end);
        let around = if inside {
            hunk.first.saturating_sub(AROUND_COMMENT).max(head_end)..=(hunk.last + AROUND_COMMENT).min(last)
        } else {
            head_end..=(head_end + AROUND_COMMENT).min(last)
        };
        if *around.start() > head_end {
            text.push_str("\n… (lines left out)");
        }
        text.push('\n');
        text.push_str(&render(doc, around));
        text.push_str("\n… (lines left out)");
        (text, true)
    };
    let (text, cut) = cut_to_chars(text);
    Some(FunctionSource { first, last, text, partial: shortened || cut })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::highlight::Highlighter;
    use crate::vscode;
    use std::path::Path;
    use std::sync::{Arc, OnceLock};

    fn highlighter() -> Arc<Highlighter> {
        static SHARED: OnceLock<Arc<Highlighter>> = OnceLock::new();
        SHARED
            .get_or_init(|| Arc::new(Highlighter::load(vscode::scan(false)).expect("grammars").0))
            .clone()
    }

    fn found(file: &str, source: &str, hunk: usize) -> Option<(usize, usize)> {
        let h = highlighter();
        let doc = h.highlight(source, &h.language_for(Path::new(file)), "dark-plus").unwrap();
        containing(&doc, &doc.hunks[hunk]).map(|f| (f.first, f.last))
    }

    #[test]
    fn comment_above_a_function_takes_that_function() {
        let source = "// adds\nfunction add(a, b) {\n  return a + b;\n}\n\nconst x = 1;\n";
        assert_eq!(found("a.js", source, 0), Some((1, 3)));
    }

    #[test]
    fn comment_inside_takes_the_enclosing_function() {
        let source = "function a() {\n  b();\n}\n\nfunction c() {\n  // note\n  d();\n}\n";
        assert_eq!(found("a.js", source, 0), Some((4, 7)));
    }

    #[test]
    fn rust_lifetimes_do_not_look_like_strings() {
        let source = "// first\nfn pick<'a>(x: &'a str) -> &'a str {\n    x\n}\n";
        assert_eq!(found("a.rs", source, 0), Some((1, 3)));
    }

    #[test]
    fn typescript_methods_and_long_signatures() {
        let source = "class A {\n  // does it\n  async run(\n    a: string,\n  ): Promise<void> {\n    go();\n  }\n}\n";
        assert_eq!(found("a.ts", source, 0), Some((2, 6)));
    }

    #[test]
    fn arrow_functions_with_a_body() {
        let source = "// runs\nconst run = async (a) => {\n  go(a);\n};\n";
        assert_eq!(found("a.ts", source, 0), Some((1, 3)));
    }

    #[test]
    fn brace_on_the_next_line() {
        let source = "class A {\n    // gets\n    public int Get(int a)\n    {\n        return a;\n    }\n}\n";
        assert_eq!(found("a.cs", source, 0), Some((2, 5)));
    }

    #[test]
    fn braces_in_strings_are_not_counted() {
        let source = "function a() {\n  // note\n  b(\"}\");\n  c();\n}\nd();\n";
        assert_eq!(found("a.js", source, 0), Some((0, 4)));
    }

    #[test]
    fn python_uses_indentation() {
        let source = "# adds\ndef add(a, b):\n    c = a + b\n\n    return c\n\nx = 1\n";
        assert_eq!(found("a.py", source, 0), Some((1, 4)));
    }

    #[test]
    fn python_comment_inside() {
        let source = "def a():\n    pass\n\ndef b():\n    # note\n    pass\n";
        assert_eq!(found("a.py", source, 0), Some((3, 5)));
    }

    #[test]
    fn ruby_includes_its_end() {
        let source = "# adds\ndef add(a)\n  a + 1\nend\n\nputs 1\n";
        assert_eq!(found("a.rb", source, 0), Some((1, 3)));
    }

    #[test]
    fn control_flow_and_calls_are_not_functions() {
        assert_eq!(found("a.js", "// note\nif (x) {\n  y();\n}\n", 0), None);
        assert_eq!(found("a.js", "// note\nfoo(1, 2);\nbar();\n", 0), None);
    }

    #[test]
    fn a_comment_after_the_function_does_not_claim_it() {
        let source = "function a() {\n  b();\n}\n// after\nconst c = 1;\n";
        assert_eq!(found("a.js", source, 0), None);
    }

    #[test]
    fn long_functions_keep_the_part_around_the_comment() {
        let mut source = String::from("function big() {\n");
        for i in 0..1000 {
            source.push_str(&format!("  a{i}();\n"));
        }
        source.push_str("  // note\n  z();\n}\n");
        let h = highlighter();
        let doc = h.highlight(&source, "javascript", "dark-plus").unwrap();
        let function = containing(&doc, &doc.hunks[0]).unwrap();
        assert!(function.partial);
        assert!(function.text.contains("function big()"));
        assert!(function.text.contains("// note"));
        assert!(!function.text.contains("a500()"));
    }
}
