//! The self-contained HTML renderer.
//!
//! This module renders one HTML document from the same frames the text renderer produces:
//! a `<!DOCTYPE html>` document, `charset=utf-8`, inline `<style>`, and the frame grids
//! inside `<pre>` blocks. It is one file with **no** external reference of any kind — no
//! script, no event-handler attribute, no `http://` or `https://` URL, no `<iframe>`, no
//! external stylesheet and no CSS `url(...)`.
//!
//! Escaping is total: the five HTML metacharacters (`<`, `>`, `&`, `"`, `'`) are escaped,
//! a character outside printable ASCII is replaced by `?`, and the document size is bounded
//! by [`MAX_HTML_BYTES`] with a typed `limit_exceeded` rather than a silent truncation or a
//! truncated file. The escaping is proven with an adversarial backend descriptor supplied
//! by a scripted channel, because that is the only untrusted text a view can carry.
//!
//! The `<pre>` content of a frame is exactly [`Frame::to_text`] with that escaping applied,
//! so decoding the entities gives the plain grid back byte for byte. There is no second
//! drawing pass here: the document embeds the grids the terminal receives.

use crate::GuiError;
use crate::MAX_HTML_BYTES;
use crate::render::Frame;

/// The fixed document head: doctype, charset, inline style and the document heading.
///
/// The style is inline and contains no external reference, no `url(...)` and no script.
const DOCUMENT_HEAD: &str = "\
<!DOCTYPE html>
<html lang=\"en\">
<head>
<meta charset=\"utf-8\">
<title>chronofork gui</title>
<style>
body { background: #101216; color: #d8dee9; font-family: monospace; margin: 1rem; }
h1 { font-size: 1rem; font-weight: 600; color: #88c0d0; margin: 0 0 1rem 0; }
pre { line-height: 1.25; margin: 0 0 1rem 0; padding: 0.5rem; border: 1px solid #3b4252; }
</style>
</head>
<body>
<h1>chronofork gui</h1>
";

/// The fixed document tail.
const DOCUMENT_TAIL: &str = "</body>\n</html>\n";

/// Renders one self-contained HTML document from already-rendered plain frames.
///
/// The frames appear in the given order, each in its own `<pre class="frame">` element with
/// a stable `frame-<n>` identifier, so `n` frames always produce the same document and two
/// runs are byte-identical.
///
/// # Errors
///
/// Returns [`GuiError::LimitExceeded`] when the encoded document would exceed
/// [`MAX_HTML_BYTES`]. The bound is checked before each appended line, so no document above
/// the bound is ever allocated and nothing is silently truncated.
pub fn render(frames: &[Frame]) -> Result<String, GuiError> {
    let mut document = String::new();
    push_checked(&mut document, DOCUMENT_HEAD)?;
    for (index, frame) in frames.iter().enumerate() {
        let open = format!("<pre class=\"frame\" id=\"frame-{}\">", index + 1);
        push_checked(&mut document, &open)?;
        for line in frame.lines() {
            let needed = escaped_len(line).saturating_add(1);
            if document.len().saturating_add(needed) > MAX_HTML_BYTES {
                return Err(overflow());
            }
            escape_into(line, &mut document);
            document.push('\n');
        }
        push_checked(&mut document, "</pre>\n")?;
    }
    push_checked(&mut document, DOCUMENT_TAIL)?;
    Ok(document)
}

/// Appends `text` when the document stays inside the bound.
fn push_checked(document: &mut String, text: &str) -> Result<(), GuiError> {
    if document.len().saturating_add(text.len()) > MAX_HTML_BYTES {
        return Err(overflow());
    }
    document.push_str(text);
    Ok(())
}

/// Returns the typed failure for a document that would exceed [`MAX_HTML_BYTES`].
fn overflow() -> GuiError {
    GuiError::limit_exceeded(
        "html document bytes",
        u64::try_from(MAX_HTML_BYTES).unwrap_or(u64::MAX),
    )
}

/// Escapes `text` into `output`, escaping all five metacharacters and replacing every
/// character outside printable ASCII with `?`.
///
/// The replacement happens here as well as in the grid so the document stays fully escaped
/// even if a grid reaches this renderer by another path, and so no control sequence can
/// reach a consumer of the document.
fn escape_into(text: &str, output: &mut String) {
    for character in text.chars() {
        match character {
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '&' => output.push_str("&amp;"),
            '"' => output.push_str("&quot;"),
            '\'' => output.push_str("&#39;"),
            ' '..='~' => output.push(character),
            _ => output.push('?'),
        }
    }
}

/// Returns the number of bytes [`escape_into`] would append for `text`.
///
/// This is the pre-allocation bound check: it lets [`render`] reject an oversized document
/// before the bytes are written, so the bound is never exceeded even transiently. A test
/// keeps it in step with [`escape_into`].
fn escaped_len(text: &str) -> usize {
    text.chars()
        .map(|character| match character {
            '<' | '>' => 4,
            '&' => 5,
            '"' => 6,
            '\'' => 5,
            _ => 1,
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::{escape_into, escaped_len};
    use crate::render::Frame;

    #[test]
    fn html_escaped_len_agrees_with_the_escaper() {
        let cases = [
            "plain ascii",
            "<script>alert(1)</script>",
            "\"double\" and 'single'",
            "a & b && c",
            "control\u{0}bytes\u{1b}",
            "non-ascii \u{e9}\u{4e2d}",
            "",
        ];
        for text in cases {
            let mut encoded = String::new();
            escape_into(text, &mut encoded);
            assert_eq!(encoded.len(), escaped_len(text), "text: {text:?}");
        }
    }

    #[test]
    fn html_escaping_replaces_every_non_printable_character() {
        let mut encoded = String::new();
        escape_into("a\u{0}b\u{1b}[31mc\u{7f}d\u{4e2d}", &mut encoded);
        assert_eq!(encoded, "a?b?[31mc?d?");
        assert!(!encoded.contains('\u{1b}'));
        assert!(encoded.is_ascii());
    }

    #[test]
    fn html_document_shape_is_stable_for_a_known_grid() {
        let frame =
            Frame::new(40, 10, (0..10).map(|_| " ".repeat(40)).collect::<Vec<_>>()).expect("frame");
        let document = super::render(&[frame.clone(), frame]).expect("document");
        assert!(document.starts_with("<!DOCTYPE html>\n"));
        assert!(document.ends_with("</body>\n</html>\n"));
        assert!(document.contains("<meta charset=\"utf-8\">"));
        assert!(document.contains("<style>\n"));
        assert!(document.contains("<pre class=\"frame\" id=\"frame-1\">"));
        assert!(document.contains("<pre class=\"frame\" id=\"frame-2\">"));
        assert_eq!(document.matches("<pre").count(), 2);
    }
}
