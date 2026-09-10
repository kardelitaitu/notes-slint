//! The .notes container: byte-preserving frontmatter split and rebuild.
//!
//! By construction this module NEVER parses frontmatter keys. It only splits
//! a document into a frontmatter block and a body, and rejoins them — both
//! as VERBATIM byte slices — because the do-no-harm rule outranks tidiness.
//! Manager decision D10: pin state lives ONLY in session.json, so a
//! 'pinned:' key inside a .notes file is preserved exactly as written and
//! deliberately NOT acted on. Line endings inside the frontmatter are
//! likewise never normalised (features.md 4.5): a CRLF .notes file splits
//! and rejoins byte-exactly.
//!
//! The split rule: a frontmatter block is a line that is exactly '---' as
//! the ENTIRE first line, then lines up to the NEXT line that is exactly
//! '---' (the closer). Anything else — no leading '---', no closer, '---'
//! with trailing spaces, a file that is only '---' — is NOT frontmatter and
//! comes back as the body. rebuild(split(t)) == t for EVERY input, including
//! a body that itself contains a '---' line (a Markdown horizontal rule must
//! not be eaten as frontmatter).

use std::path::Path;

/// The two halves of a .notes document. Both are SUBSLICES of the input:
/// split allocates nothing and normalises nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NoteParts<'a> {
    /// Some(frontmatter) spans from the opening '---' line THROUGH the
    /// closing '---' line, terminators included. None when the document has
    /// no frontmatter block (the whole text is then the body).
    pub frontmatter: Option<&'a str>,
    /// Everything after the closing '---' line, verbatim; or the whole text
    /// when there is no frontmatter block.
    pub body: &'a str,
}

/// Splits a document into frontmatter and body without allocating.
pub fn split(text: &str) -> NoteParts<'_> {
    let first_end = first_line_end(text);
    if line_content(&text[..first_end]) != "---" {
        return NoteParts {
            frontmatter: None,
            body: text,
        };
    }
    // Walk the lines after the opener, looking for the first line that is
    // exactly '---'. The first such line is the closer; a '---' inside a
    // later value line is just content, and a body '---' comes after the
    // closer and is therefore never eaten.
    let mut offset = first_end;
    while offset < text.len() {
        let next_end = first_line_end(&text[offset..]);
        if line_content(&text[offset..offset + next_end]) == "---" {
            return NoteParts {
                frontmatter: Some(&text[..offset + next_end]),
                body: &text[offset + next_end..],
            };
        }
        offset += next_end;
    }
    // No closer: not frontmatter. Do-no-harm beats guessing.
    NoteParts {
        frontmatter: None,
        body: text,
    }
}

/// Verbatim join: the exact inverse of split, for every input.
pub fn rebuild(frontmatter: Option<&str>, body: &str) -> String {
    match frontmatter {
        Some(fm) => {
            let mut out = String::with_capacity(fm.len() + body.len());
            out.push_str(fm);
            out.push_str(body);
            out
        }
        None => body.to_owned(),
    }
}

/// Lexical check: is this path a .notes document? Extension only,
/// case-insensitive; it never touches the filesystem, so a path need not
/// exist and a trailing separator is simply ignored by Path parsing.
pub fn is_notes_path(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("notes"))
}

/// Offset just past the first line's terminator (including the '\n' itself;
/// a last line without a newline ends at the text end).
fn first_line_end(text: &str) -> usize {
    match text.find('\n') {
        Some(i) => i + 1,
        None => text.len(),
    }
}

/// A line's content without its terminator, so '---', '---\n' and '---\r\n'
/// all count as a '---' line — CRLF frontmatter splits like LF frontmatter.
fn line_content(line: &str) -> &str {
    let core = line.strip_suffix('\n').unwrap_or(line);
    core.strip_suffix('\r').unwrap_or(core)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// rebuild(split(t)) == t is the contract for EVERY input. This helper
    /// asserts it and returns the split for further assertions.
    fn assert_round_trips(text: &str) -> NoteParts<'_> {
        let parts = split(text);
        assert_eq!(
            rebuild(parts.frontmatter, parts.body),
            text,
            "split/rebuild is not the identity for {text:?}"
        );
        parts
    }

    #[test]
    fn frontmatter_block_splits_and_rejoins() {
        let parts = assert_round_trips("---\ntitle: x\n---\nbody here\n");
        assert_eq!(parts.frontmatter, Some("---\ntitle: x\n---\n"));
        assert_eq!(parts.body, "body here\n");
    }

    #[test]
    fn no_leading_dashes_is_all_body() {
        let parts = assert_round_trips("just a note\n");
        assert_eq!(parts.frontmatter, None);
        assert_eq!(parts.body, "just a note\n");
    }

    #[test]
    fn only_dashes_is_all_body() {
        // A file that is only '---' has no closing line: not frontmatter.
        let parts = assert_round_trips("---");
        assert_eq!(parts.frontmatter, None);
        assert_eq!(parts.body, "---");
        let parts = assert_round_trips("---\n");
        assert_eq!(parts.frontmatter, None);
        assert_eq!(parts.body, "---\n");
    }

    #[test]
    fn unterminated_block_is_all_body() {
        let parts = assert_round_trips("---\ntitle: x\nno closer");
        assert_eq!(parts.frontmatter, None);
        assert_eq!(parts.body, "---\ntitle: x\nno closer");
    }

    #[test]
    fn dashes_with_trailing_space_is_all_body() {
        let parts = assert_round_trips("--- \ntitle: x\n---\nbody");
        assert_eq!(parts.frontmatter, None);
        assert_eq!(parts.body, "--- \ntitle: x\n---\nbody");
    }

    #[test]
    fn dashes_inside_a_frontmatter_value_do_not_close_early() {
        // Only a line that is EXACTLY '---' closes the block.
        let parts = assert_round_trips("---\ntags: a---b\n---\nbody");
        assert_eq!(parts.frontmatter, Some("---\ntags: a---b\n---\n"));
        assert_eq!(parts.body, "body");
    }

    #[test]
    fn body_horizontal_rule_is_not_eaten_as_frontmatter() {
        // The classic Markdown gotcha: the first exact '---' AFTER the opener
        // closes the block; a '---' in the body stays in the body.
        let text = "---\ntitle: t\n---\nintro\n---\nmore text\n";
        let parts = assert_round_trips(text);
        assert_eq!(parts.frontmatter, Some("---\ntitle: t\n---\n"));
        assert_eq!(parts.body, "intro\n---\nmore text\n");
    }

    #[test]
    fn a_note_whose_body_starts_with_dashes_still_round_trips() {
        // '---\n' as the opener with a later '---' IS parsed as frontmatter —
        // that is the format's rule — and the identity still holds exactly.
        let text = "---\nhello\n---\nrest";
        let parts = assert_round_trips(text);
        assert_eq!(parts.frontmatter, Some("---\nhello\n---\n"));
        assert_eq!(parts.body, "rest");
        // And with no closer it is all body.
        assert_round_trips("---\njust a horizontal rule");
    }

    #[test]
    fn crlf_frontmatter_splits_and_rejoins_byte_exactly() {
        // features.md 4.5 forbids normalising line endings: a CRLF .notes
        // file must keep every \r\n verbatim through split and rebuild.
        let text = "---\r\ntitle: x\r\n---\r\nbody\r\n";
        let parts = assert_round_trips(text);
        assert_eq!(parts.frontmatter, Some("---\r\ntitle: x\r\n---\r\n"));
        assert_eq!(parts.body, "body\r\n");
    }

    #[test]
    fn empty_input_splits_to_nothing() {
        let parts = assert_round_trips("");
        assert_eq!(parts.frontmatter, None);
        assert_eq!(parts.body, "");
    }

    #[test]
    fn rebuild_is_verbatim_for_none_and_some() {
        assert_eq!(rebuild(None, "body"), "body");
        assert_eq!(rebuild(Some("---\n---\n"), ""), "---\n---\n");
    }

    #[test]
    fn is_notes_path_is_lexical_and_case_insensitive() {
        assert!(is_notes_path(Path::new("note.notes")));
        assert!(is_notes_path(Path::new("note.NOTES")));
        assert!(is_notes_path(Path::new("note.Notes")));
        assert!(is_notes_path(Path::new("deep/dir/note.notes")));
        assert!(is_notes_path(Path::new("archive.notes/"))); // trailing separator ignored
        // A bare dotfile has no extension in Path semantics.
        assert!(!is_notes_path(Path::new(".notes")));
        assert!(!is_notes_path(Path::new(".NOTES")));
        assert!(!is_notes_path(Path::new("note"))); // no extension
        assert!(!is_notes_path(Path::new("note.txt")));
        assert!(!is_notes_path(Path::new("note.notes."))); // trailing dot
        assert!(!is_notes_path(Path::new("note.")));
    }

    /// The fixture set is the do-no-harm corpus: every decoded fixture text
    /// must be an identity under split/rebuild. Fixture drift (a fixture the
    /// manifest lists but the disk lacks) fails loudly here too.
    #[test]
    fn every_fixture_text_is_a_split_rebuild_identity() -> Result<(), Box<dyn std::error::Error>> {
        let manifest_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("manifest.json");
        let manifest: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&manifest_path).map_err(|e| {
                format!("manifest unreadable at {}: {e}", manifest_path.display())
            })?)?;
        let files = manifest
            .get("files")
            .and_then(|f| f.as_array())
            .ok_or("manifest has no files array")?;
        assert!(!files.is_empty(), "manifest lists no fixtures");
        for file in files {
            let name = file
                .get("file")
                .and_then(|f| f.as_str())
                .ok_or("manifest entry without a file name")?;
            let path = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests")
                .join("fixtures")
                .join(name);
            let bytes = std::fs::read(&path)
                .map_err(|e| format!("cannot read fixture {}: {e}", path.display()))?;
            let detected = crate::detect(&bytes, Some(1252));
            let text = crate::decode(&bytes, detected)?;
            let parts = split(&text);
            assert_eq!(
                rebuild(parts.frontmatter, parts.body),
                text,
                "{name}: split/rebuild is not the identity"
            );
        }
        Ok(())
    }
}
