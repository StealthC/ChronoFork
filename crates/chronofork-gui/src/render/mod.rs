//! The frame grid and the renderer options shared by both frontends.
//!
//! Both documented frontends render the **same** grid: the plain/ANSI text renderer in
//! [`text`] and the self-contained HTML document renderer in [`html`]. [`Frame`] is the
//! canonical, already-validated grid that both consume, so "the ANSI frame equals the
//! plain frame after removing SGR sequences" and "the HTML `<pre>` content equals the
//! plain frames after entity decoding" are comparisons of one value, not of two
//! independent renderers.
//!
//! Frame rules (`gui.md` §4, asserted by tests and by the acceptance plan):
//!
//! 1. Exactly `height` lines, each exactly `width` printable ASCII columns, space padded,
//!    clipped at the region edge and never wrapped.
//! 2. Only printable ASCII (`0x20..=0x7E`) and `\n` are emitted; no locale, no width
//!    table and no Unicode layout exists.
//! 3. No host time, path, token or environment value can appear: none reaches the model.
//! 4. A [`Frame`] holds the plain grid only. ANSI emphasis is inserted when the text
//!    renderer serializes it, so the grid itself stays comparable byte for byte.
//!
//! Row structure of the grid (`gui.md` §4): the title line, a separator of `-`, the pane
//! regions, and the status line. The renderer owns that placement; the model owns which
//! pane occupies which region ([`crate::model::Mosaic`]).
//!
//! The text renderer fills the grid through `Grid`, the fixed-size cell matrix that is
//! itself the clipping and replacement rule: a write is dropped at the region edge and a
//! character outside printable ASCII becomes `?`, so both frontends inherit exactly one
//! such rule.

pub mod html;
pub mod text;

use crate::model::Rect;
use crate::{GuiError, MAX_HEIGHT, MAX_WIDTH, MIN_HEIGHT, MIN_WIDTH};

/// The canonical plain text grid of one rendered frame.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Frame {
    width: u32,
    height: u32,
    lines: Vec<String>,
}

impl Frame {
    /// Creates a frame from an already-rendered grid.
    ///
    /// # Errors
    ///
    /// Returns [`GuiError::LimitExceeded`] when `width` or `height` is outside the
    /// documented frame bounds, and [`GuiError::BadValue`] when the grid does not have
    /// exactly `height` lines, when a line is not exactly `width` printable ASCII columns,
    /// or when a line carries a byte a terminal or an HTML document would have to
    /// reinterpret.
    pub fn new(width: u32, height: u32, lines: Vec<String>) -> Result<Self, GuiError> {
        check_frame_bounds(width, height)?;
        let expected = usize::try_from(height).unwrap_or(usize::MAX);
        if lines.len() != expected {
            return Err(GuiError::bad_value(
                "frame lines",
                "the grid does not have exactly `height` lines",
            ));
        }
        let columns = usize::try_from(width).unwrap_or(usize::MAX);
        for line in &lines {
            if line.len() != columns {
                return Err(GuiError::bad_value(
                    "frame line",
                    "a line is not exactly `width` columns wide",
                ));
            }
            if !line.bytes().all(|byte| (0x20..=0x7e).contains(&byte)) {
                return Err(GuiError::bad_value(
                    "frame line",
                    "a line carries a byte that is not printable ASCII",
                ));
            }
        }
        Ok(Self {
            width,
            height,
            lines,
        })
    }

    /// Returns the frame width in cells.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Returns the frame height in lines.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Returns the grid lines in order.
    #[must_use]
    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    /// Returns one grid line by index.
    #[must_use]
    pub fn line(&self, index: usize) -> Option<&str> {
        self.lines.get(index).map(String::as_str)
    }

    /// Serializes the grid as `height` newline-terminated lines, with no trailing blank
    /// line.
    #[must_use]
    pub fn to_text(&self) -> String {
        let capacity = self
            .lines
            .iter()
            .map(|line| line.len().saturating_add(1))
            .sum();
        let mut text = String::with_capacity(capacity);
        for line in &self.lines {
            text.push_str(line);
            text.push('\n');
        }
        text
    }
}

/// The renderer options of one window.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RenderOptions {
    /// Emit ANSI SGR emphasis. The plain renderer is the default and the comparable form.
    pub ansi: bool,
}

impl RenderOptions {
    /// Returns the plain (non-ANSI) options.
    #[must_use]
    pub const fn plain() -> Self {
        Self { ansi: false }
    }

    /// Returns the options with ANSI emphasis set to `ansi`.
    #[must_use]
    pub const fn with_ansi(ansi: bool) -> Self {
        Self { ansi }
    }
}

/// Validates the documented frame bounds before any grid work starts.
///
/// The renderers call this first, so an out-of-range request is a typed
/// [`GuiError::LimitExceeded`] before a single cell is allocated; [`Frame::new`] applies
/// exactly the same rule to a grid that was produced elsewhere.
pub(crate) fn check_frame_bounds(width: u32, height: u32) -> Result<(), GuiError> {
    check_bound(width, MIN_WIDTH, MAX_WIDTH, "frame width")?;
    check_bound(height, MIN_HEIGHT, MAX_HEIGHT, "frame height")
}

/// The fixed-size cell matrix the text renderer fills before a [`Frame`] is built.
///
/// The grid **is** the clipping rule: a write is dropped at the region's right edge and at
/// the frame's last row, never wrapped, and every row always holds exactly `width` cells,
/// so a renderer cannot produce a frame whose shape differs from the configured one. A
/// character outside printable ASCII becomes `?` on write, which is where the "a byte a
/// view carries outside printable ASCII renders as `?`" rule is enforced for the plain grid
/// and, because the ANSI form is serialized from that same grid, for both frontends.
#[derive(Debug)]
pub(crate) struct Grid {
    rows: Vec<Vec<char>>,
    emphasized: Vec<bool>,
}

impl Grid {
    /// Creates a blank grid of `width` by `height` cells.
    ///
    /// The caller validates the bounds with [`check_frame_bounds`] first; a `width` or
    /// `height` that does not fit in `usize` yields an empty grid rather than an
    /// allocation attempt.
    pub(crate) fn new(width: u32, height: u32) -> Self {
        let columns = usize::try_from(width).unwrap_or(0);
        let rows = usize::try_from(height).unwrap_or(0);
        Self {
            rows: vec![vec![' '; columns]; rows],
            emphasized: vec![false; rows],
        }
    }

    /// Writes `text` into `region`, starting `offset` rows below the region's top edge.
    ///
    /// The write is clipped at the region's right edge and at the frame's last row; an
    /// offset outside the region writes nothing.
    pub(crate) fn put(&mut self, region: Rect, offset: u32, text: &str) {
        if offset >= region.height {
            return;
        }
        let Ok(row_index) = usize::try_from(region.y.saturating_add(offset)) else {
            return;
        };
        let Some(row) = self.rows.get_mut(row_index) else {
            return;
        };
        let end = region.x.saturating_add(region.width);
        for (index, character) in text.chars().enumerate() {
            let Ok(index) = u32::try_from(index) else {
                break;
            };
            let column = region.x.saturating_add(index);
            if column >= end {
                break;
            }
            let Ok(column) = usize::try_from(column) else {
                break;
            };
            let Some(cell) = row.get_mut(column) else {
                break;
            };
            *cell = printable(character);
        }
    }

    /// Marks the row at `region.y + offset` for ANSI emphasis.
    pub(crate) fn emphasize(&mut self, region: Rect, offset: u32) {
        if offset >= region.height {
            return;
        }
        let Ok(row_index) = usize::try_from(region.y.saturating_add(offset)) else {
            return;
        };
        if let Some(flag) = self.emphasized.get_mut(row_index) {
            *flag = true;
        }
    }

    /// Consumes the grid into its rows and the per-row emphasis flags.
    pub(crate) fn into_rows(self) -> (Vec<String>, Vec<bool>) {
        let rows = self
            .rows
            .into_iter()
            .map(|row| row.into_iter().collect())
            .collect();
        (rows, self.emphasized)
    }
}

/// Maps one character to the single printable ASCII cell it renders as.
pub(crate) fn printable(character: char) -> char {
    if (' '..='~').contains(&character) {
        character
    } else {
        '?'
    }
}

fn check_bound(value: u32, minimum: u32, maximum: u32, what: &'static str) -> Result<(), GuiError> {
    if value < minimum || value > maximum {
        return Err(GuiError::limit_exceeded(what, u64::from(maximum)));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Frame, Grid, RenderOptions};
    use crate::model::Rect;
    use crate::{GuiErrorCode, MAX_HEIGHT, MAX_WIDTH, MIN_HEIGHT, MIN_WIDTH};

    fn grid(width: u32, height: u32) -> Vec<String> {
        (0..height)
            .map(|_| " ".repeat(usize::try_from(width).expect("width")))
            .collect()
    }

    #[test]
    fn a_valid_grid_is_accepted_and_serialized_without_a_trailing_blank_line() {
        let width = 40;
        let height = 10;
        let mut lines = grid(width, height);
        lines[0] = format!("{:<40}", "chronofork gui");
        lines[9] = format!("{:<40}", "panes=0 focused=none");
        let frame = Frame::new(width, height, lines).expect("frame");
        assert_eq!(frame.width(), 40);
        assert_eq!(frame.height(), 10);
        assert_eq!(frame.lines().len(), 10);
        assert_eq!(
            frame.line(0),
            Some("chronofork gui                          ")
        );
        assert_eq!(frame.line(10), None);

        let text = frame.to_text();
        let rendered: Vec<&str> = text.split('\n').collect();
        assert_eq!(rendered.len(), 11);
        assert_eq!(rendered[10], "");
        for line in &rendered[..10] {
            assert_eq!(line.len(), 40);
        }
        assert!(!text.ends_with("\n\n"));
    }

    #[test]
    fn a_grid_outside_the_bounds_or_the_shape_is_rejected() {
        let error = Frame::new(MIN_WIDTH - 1, MIN_HEIGHT, grid(MIN_WIDTH, MIN_HEIGHT))
            .expect_err("too narrow");
        assert_eq!(error.code(), GuiErrorCode::LimitExceeded);
        let error = Frame::new(MIN_WIDTH, MIN_HEIGHT, grid(MIN_WIDTH, MIN_HEIGHT - 1))
            .expect_err("wrong line count");
        assert_eq!(error.code(), GuiErrorCode::BadValue);
        let error = Frame::new(
            MAX_WIDTH,
            MAX_HEIGHT,
            vec!["x".repeat(usize::try_from(MAX_WIDTH).expect("width"))],
        )
        .expect_err("wrong line count");
        assert_eq!(error.code(), GuiErrorCode::BadValue);
    }

    #[test]
    fn a_line_must_be_exactly_the_declared_width_and_printable_ascii() {
        let mut lines = grid(40, 10);
        lines[3].push(' ');
        let error = Frame::new(40, 10, lines).expect_err("too wide");
        assert_eq!(error.code(), GuiErrorCode::BadValue);

        let mut lines = grid(40, 10);
        lines[4] = format!("{:<40}", "pane\t1");
        let error = Frame::new(40, 10, lines).expect_err("control byte");
        assert_eq!(error.code(), GuiErrorCode::BadValue);

        let mut lines = grid(40, 10);
        lines[5] = format!("{:<40}", "paneé");
        let error = Frame::new(40, 10, lines).expect_err("non-ASCII byte");
        assert_eq!(error.code(), GuiErrorCode::BadValue);
    }

    #[test]
    fn the_documented_frame_bounds_are_accepted() {
        Frame::new(MIN_WIDTH, MIN_HEIGHT, grid(MIN_WIDTH, MIN_HEIGHT)).expect("minimum frame");
        Frame::new(MAX_WIDTH, MAX_HEIGHT, grid(MAX_WIDTH, MAX_HEIGHT)).expect("maximum frame");
    }

    #[test]
    fn render_options_default_to_plain_and_select_ansi_explicitly() {
        assert_eq!(RenderOptions::default(), RenderOptions::plain());
        assert!(!RenderOptions::plain().ansi);
        assert!(RenderOptions::with_ansi(true).ansi);
        assert!(!RenderOptions::with_ansi(false).ansi);
    }

    #[test]
    fn render_grid_writes_are_clipped_at_the_region_edge_and_never_wrap() {
        let mut grid = Grid::new(10, 3);
        let right = Rect::new(5, 1, 5, 1);
        grid.put(right, 0, "0123456789");
        let (rows, emphasized) = grid.into_rows();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0], "          ");
        assert_eq!(rows[1], "     01234", "clipped at the region right edge");
        assert_eq!(rows[2], "          ");
        assert_eq!(emphasized, [false, false, false]);

        // A write past the last row or past the region's rows does nothing.
        let mut grid = Grid::new(10, 3);
        let area = Rect::new(0, 2, 10, 1);
        grid.put(area, 1, "x");
        grid.put(area, 0, "y");
        grid.emphasize(area, 1);
        let (rows, emphasized) = grid.into_rows();
        assert_eq!(rows[2], "y         ");
        assert_eq!(emphasized, [false, false, false], "no row was marked");
    }

    #[test]
    fn render_grid_writes_replace_non_printable_characters_with_a_marker() {
        let mut grid = Grid::new(10, 2);
        let row = Rect::new(0, 0, 10, 1);
        grid.put(row, 0, "a\tb\u{1b}c\u{e9}d");
        let (rows, _) = grid.into_rows();
        assert_eq!(rows[0], "a?b?c?d   ");

        let mut grid = Grid::new(4, 2);
        grid.put(Rect::new(0, 1, 4, 1), 0, "\u{7f}ab");
        grid.emphasize(Rect::new(0, 1, 4, 1), 0);
        let (rows, emphasized) = grid.into_rows();
        assert_eq!(rows[1], "?ab ");
        assert_eq!(emphasized, [false, true]);
    }
}
