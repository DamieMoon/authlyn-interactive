//! Tree-builder: consumes a `Vec<Tok>` and produces a `Vec<Node>`.
//!
//! Stack of open spans; an unclosed span unwinds to its literal opener so the
//! parser is total (leniency invariant). Split from `src/markup.rs` in Wave 3;
//! behavior preserved verbatim.

use super::tokenize::Tok;
use super::{Color, Node};

enum FrameKind {
    Root,
    Bold,
    Italic,
    Color(Color),
    Dialogue,
    Spoiler,
}

struct Frame {
    kind: FrameKind,
    /// The literal opener, re-emitted as text if this frame is never closed.
    opener: String,
    children: Vec<Node>,
}

pub(super) fn build_tree(tokens: Vec<Tok>) -> Vec<Node> {
    let mut stack: Vec<Frame> = vec![Frame {
        kind: FrameKind::Root,
        opener: String::new(),
        children: Vec::new(),
    }];

    for tok in tokens {
        match tok {
            Tok::Text(s) => push_text(&mut top(&mut stack).children, &s),
            Tok::Code(s) => top(&mut stack).children.push(Node::Code(s)),
            Tok::Bold => toggle(&mut stack, FrameKind::Bold, "**"),
            Tok::Italic => toggle(&mut stack, FrameKind::Italic, "*"),
            Tok::ColorOpen(c) => stack.push(Frame {
                kind: FrameKind::Color(c),
                opener: format!("[{}]", c.name()),
                children: Vec::new(),
            }),
            Tok::ColorClose(c) => {
                if matches!(top(&mut stack).kind, FrameKind::Color(open) if open == c) {
                    close_top(&mut stack);
                } else {
                    let lit = format!("[/{}]", c.name());
                    push_text(&mut top(&mut stack).children, &lit);
                }
            }
            Tok::Dialogue => toggle(&mut stack, FrameKind::Dialogue, "\""),
            Tok::Spoiler => toggle(&mut stack, FrameKind::Spoiler, "||"),
            Tok::Image(alt, url) => top(&mut stack).children.push(Node::Image(alt, url)),
            Tok::Link(text, url) => top(&mut stack).children.push(Node::Link(text, url)),
            Tok::Emoji(name) => top(&mut stack).children.push(Node::Emoji(name)),
            Tok::Mention(name) => top(&mut stack).children.push(Node::Mention(name)),
        }
    }

    // Unwind anything still open: its opener becomes literal text, its
    // children splice into the parent.
    while stack.len() > 1 {
        let frame = stack.pop().expect("len > 1");
        let parent = &mut top(&mut stack).children;
        push_text(parent, &frame.opener);
        for node in frame.children {
            push_node(parent, node);
        }
    }

    stack.pop().expect("root frame").children
}

fn top(stack: &mut [Frame]) -> &mut Frame {
    stack.last_mut().expect("stack always holds the root frame")
}

/// Toggle a bold/italic span: close it if it's the innermost open frame,
/// otherwise open a new one.
fn toggle(stack: &mut Vec<Frame>, kind: FrameKind, opener: &str) {
    let matches_top = matches!(
        (&top(stack).kind, &kind),
        (FrameKind::Bold, FrameKind::Bold)
            | (FrameKind::Italic, FrameKind::Italic)
            | (FrameKind::Dialogue, FrameKind::Dialogue)
            | (FrameKind::Spoiler, FrameKind::Spoiler)
    );
    if matches_top {
        close_top(stack);
    } else {
        stack.push(Frame {
            kind,
            opener: opener.to_string(),
            children: Vec::new(),
        });
    }
}

fn close_top(stack: &mut Vec<Frame>) {
    let frame = stack.pop().expect("close_top with a frame open");
    let node = match frame.kind {
        FrameKind::Bold => Node::Bold(frame.children),
        FrameKind::Italic => Node::Italic(frame.children),
        FrameKind::Color(c) => Node::Color(c, frame.children),
        FrameKind::Dialogue => Node::Dialogue(frame.children),
        FrameKind::Spoiler => Node::Spoiler(frame.children),
        FrameKind::Root => unreachable!("the root frame is never closed"),
    };
    push_node(&mut top(stack).children, node);
}

/// Append text, merging with a trailing `Text` node so the AST stays compact.
pub(super) fn push_text(children: &mut Vec<Node>, s: &str) {
    if s.is_empty() {
        return;
    }
    if let Some(Node::Text(last)) = children.last_mut() {
        last.push_str(s);
    } else {
        children.push(Node::Text(s.to_string()));
    }
}

pub(super) fn push_node(children: &mut Vec<Node>, node: Node) {
    match node {
        Node::Text(s) => push_text(children, &s),
        other => children.push(other),
    }
}
