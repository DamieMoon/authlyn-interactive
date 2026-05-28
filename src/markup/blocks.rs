//! Block-level parse (line-leading markers + fenced code blocks). Composes the
//! tokenizer + tree builder for the inline content of each non-fence block.
//!
//! Split from `src/markup.rs` in Wave 3; behavior preserved verbatim.

use super::tokenize::tokenize;
use super::tree::{build_tree, push_node};
use super::Node;

/// Parse `input` line-by-line into block-level nodes, recombining ordinary
/// text lines so a multi-line paragraph stays a single inline run (preserving
/// its embedded newlines, which the renderer shows via `white-space: pre-wrap`).
pub(super) fn parse_blocks(input: &str) -> Vec<Node> {
    let mut out: Vec<Node> = Vec::new();
    // Buffer of consecutive plain lines awaiting inline parsing as one run.
    let mut para: Vec<&str> = Vec::new();
    let mut lines = input.split('\n').peekable();

    // Flush the pending plain-line buffer as one inline run.
    let flush_para = |para: &mut Vec<&str>, out: &mut Vec<Node>| {
        if !para.is_empty() {
            let joined = para.join("\n");
            for node in build_tree(tokenize(&joined)) {
                push_node(out, node);
            }
            para.clear();
        }
    };

    while let Some(line) = lines.next() {
        if line.trim_end() == "```" {
            // Open a fence: consume verbatim until the closing ``` or EOF.
            flush_para(&mut para, &mut out);
            let mut body: Vec<&str> = Vec::new();
            let mut closed = false;
            for inner in lines.by_ref() {
                if inner.trim_end() == "```" {
                    closed = true;
                    break;
                }
                body.push(inner);
            }
            if closed {
                out.push(Node::CodeBlock(body.join("\n")));
            } else {
                // Unterminated fence: lenient fallback — re-emit the opening
                // line and its captured body as literal text.
                let mut lit = String::from("```");
                for b in body {
                    lit.push('\n');
                    lit.push_str(b);
                }
                push_node(&mut out, Node::Text(lit));
            }
            continue;
        }

        if let Some(block) = parse_line_block(line) {
            flush_para(&mut para, &mut out);
            out.push(block);
        } else {
            para.push(line);
        }
    }
    flush_para(&mut para, &mut out);
    out
}

/// If `line` is a line-leading block (`#`/`##`/`###` heading or `-#` subtext),
/// parse its inline content and return the block node. A marker not followed by
/// a space (e.g. a bare `#` or `#foo`) is *not* a block — returns `None` so the
/// line falls through to literal/inline handling.
fn parse_line_block(line: &str) -> Option<Node> {
    if let Some(rest) = line.strip_prefix("-# ") {
        return Some(Node::Subtext(build_tree(tokenize(rest))));
    }
    for (marker, level) in [("### ", 3u8), ("## ", 2), ("# ", 1)] {
        if let Some(rest) = line.strip_prefix(marker) {
            return Some(Node::Heading(level, build_tree(tokenize(rest))));
        }
    }
    None
}
