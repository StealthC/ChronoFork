#![allow(clippy::expect_used)]

//! Background planes, window and scroll-mode coverage.
//!
//! A plane is a 32- or 64-cell-wide nametable selected by register `0x02`
//! (plane A) or `0x04` (plane B), filled with 2-byte cell entries (tile,
//! palette, flips, priority) and tiles of 4bpp packed pixels. The window
//! replaces plane A inside the rectangle selected by registers `0x11`/`0x12`.
//!
//! The rendered values are frame palette indices: bits `5..0` are the CRAM
//! entry, bit 6 marks a shadowed pixel and bit 7 a highlighted pixel. Plane
//! colour 0 is transparent, so the lower-priority plane or the register `0x07`
//! background shows through.

use chronofork_megadrive::vdp::{CONTROL_PORT_OFFSET, DATA_PORT_OFFSET, Vdp, VdpError};

const WIDTH_H40: usize = 320;
const PLANE_A_BASE: u16 = 0xE000;
const PLANE_B_BASE: u16 = 0x2000;
const WINDOW_BASE: u16 = 0xF000;

fn write_register(vdp: &mut Vdp, register: u8, value: u8) -> Result<(), VdpError> {
    vdp.write16(
        CONTROL_PORT_OFFSET,
        0x8000 | (u16::from(register) << 8) | u16::from(value),
    )
}

fn write_vram(vdp: &mut Vdp, address: u16, words: &[u16]) -> Result<(), VdpError> {
    write_register(vdp, 0x0F, 2)?;
    vdp.write16(CONTROL_PORT_OFFSET, 0x4000 | (address & 0x3FFF))?;
    vdp.write16(CONTROL_PORT_OFFSET, (address >> 14) & 0x0003)?;
    for word in words {
        vdp.write16(DATA_PORT_OFFSET, *word)?;
    }
    Ok(())
}

/// Fills a tile with one colour (two pixels per byte, 4bpp packed).
fn write_solid_tile(vdp: &mut Vdp, tile: u16, colour: u8) -> Result<(), VdpError> {
    let packed = (colour << 4) | colour;
    let word = u16::from_be_bytes([packed, packed]);
    let words = [word; 16];
    write_vram(vdp, tile.wrapping_mul(32), &words)
}

/// Writes one plane nametable cell at `(cell_x, cell_y)`.
fn write_cell(
    vdp: &mut Vdp,
    base: u16,
    columns: u16,
    cell_x: u16,
    cell_y: u16,
    word: u16,
) -> Result<(), VdpError> {
    let address = base.wrapping_add((cell_y * columns + cell_x) * 2);
    write_vram(vdp, address, &[word])
}

fn base_vdp() -> Vdp {
    let mut vdp = Vdp::new();
    // H40, 32x32 planes, full-screen scroll, background colour 0.
    write_register(&mut vdp, 0x02, 0x38).expect("plane A base");
    write_register(&mut vdp, 0x04, 0x01).expect("plane B base");
    write_register(&mut vdp, 0x03, 0x3C).expect("window base");
    write_register(&mut vdp, 0x05, 0x7C).expect("SAT base at 0xF800");
    write_register(&mut vdp, 0x0C, 0x01).expect("H40 mode");
    write_register(&mut vdp, 0x10, 0x00).expect("32x32 planes");
    write_register(&mut vdp, 0x0B, 0x00).expect("full-screen scroll");
    write_register(&mut vdp, 0x0D, 0x01).expect("hscroll table at 0x0400");
    write_register(&mut vdp, 0x07, 0x00).expect("background colour");
    vdp
}

fn scanline(vdp: &mut Vdp, line: u16) -> [u8; WIDTH_H40] {
    let mut out = [0_u8; WIDTH_H40];
    vdp.render_scanline(line, &mut out);
    out
}

#[test]
fn plane_a_cell_renders_expected_indices() {
    let mut vdp = base_vdp();
    // Plane A cell 0: tile 0, palette 1, no flips, no priority.
    write_cell(&mut vdp, PLANE_A_BASE, 32, 0, 0, 0x0800).expect("cell");
    write_solid_tile(&mut vdp, 0, 3).expect("tile 0");

    let out = scanline(&mut vdp, 0);
    for (x, pixel) in out[..8].iter().enumerate() {
        assert_eq!(*pixel, 0x13, "pixel {x} is palette-1 colour 3");
    }
}

#[test]
fn horizontal_scroll_offsets_the_plane() {
    let mut vdp = base_vdp();
    write_cell(&mut vdp, PLANE_A_BASE, 32, 0, 0, 0x0800).expect("cell 0");
    write_cell(&mut vdp, PLANE_A_BASE, 32, 1, 0, 0x0801).expect("cell 1");
    write_cell(&mut vdp, PLANE_A_BASE, 32, 2, 0, 0x0800).expect("cell 2");
    write_solid_tile(&mut vdp, 0, 3).expect("tile 0");
    write_solid_tile(&mut vdp, 1, 5).expect("tile 1");
    // Full-screen horizontal scroll for plane A is the first table word.
    write_vram(&mut vdp, 0x0400, &[8]).expect("hscroll A");

    let out = scanline(&mut vdp, 0);
    for pixel in &out[..8] {
        assert_eq!(*pixel, 0x15, "the scrolled-in cell shows tile 1");
    }
    for pixel in &out[8..16] {
        assert_eq!(*pixel, 0x13, "the next cell still shows tile 0");
    }
}

#[test]
fn plane_b_shows_where_plane_a_is_transparent() {
    let mut vdp = base_vdp();
    // Plane A cell 0 selects an all-zero (transparent) tile.
    write_cell(&mut vdp, PLANE_A_BASE, 32, 0, 0, 0x0002).expect("A cell");
    // Plane B cell 0: tile 0, palette 2.
    write_cell(&mut vdp, PLANE_B_BASE, 32, 0, 0, 0x1000).expect("B cell");
    write_solid_tile(&mut vdp, 0, 3).expect("tile 0");

    let out = scanline(&mut vdp, 0);
    for pixel in &out[..8] {
        assert_eq!(*pixel, 0x23, "palette-2 colour 3 from plane B");
    }
}

#[test]
fn priority_bits_select_the_higher_plane() {
    let mut vdp = base_vdp();
    write_cell(&mut vdp, PLANE_A_BASE, 32, 0, 0, 0x0800).expect("A cell");
    write_cell(&mut vdp, PLANE_B_BASE, 32, 0, 0, 0x9001).expect("B cell with priority");
    write_solid_tile(&mut vdp, 0, 3).expect("tile 0");
    write_solid_tile(&mut vdp, 1, 5).expect("tile 1");

    // Plane B has the priority bit, so it wins over a non-priority plane A.
    let out = scanline(&mut vdp, 0);
    assert_eq!(out[0], 0x25, "priority plane B over plane A");

    // Setting plane A's priority bit makes A win over B.
    write_cell(&mut vdp, PLANE_A_BASE, 32, 0, 0, 0x8800).expect("A cell with priority");
    let out = scanline(&mut vdp, 0);
    assert_eq!(out[0], 0x13, "priority plane A over plane B");
}

#[test]
fn window_replaces_plane_a_in_the_selected_rectangle() {
    let mut vdp = base_vdp();
    write_cell(&mut vdp, PLANE_A_BASE, 32, 0, 0, 0x0800).expect("A cell 0");
    write_cell(&mut vdp, PLANE_A_BASE, 32, 0, 1, 0x0800).expect("A cell 1");
    write_cell(&mut vdp, PLANE_A_BASE, 32, 1, 0, 0x0800).expect("A cell at x=8");
    write_cell(&mut vdp, WINDOW_BASE, 64, 0, 0, 0x1801).expect("window cell");
    write_solid_tile(&mut vdp, 0, 3).expect("tile 0");
    write_solid_tile(&mut vdp, 1, 5).expect("tile 1");
    // Left window 8 pixels (one cell) wide, top window 8 lines high.
    write_register(&mut vdp, 0x11, 0x81).expect("window left");
    write_register(&mut vdp, 0x12, 0x81).expect("window top");

    let out = scanline(&mut vdp, 0);
    for pixel in &out[..8] {
        assert_eq!(*pixel, 0x35, "window tile 1 palette 3 colour 5");
    }
    for pixel in &out[8..16] {
        assert_eq!(*pixel, 0x13, "outside the window plane A shows through");
    }

    // Below the vertical window bound plane A is visible in the same columns.
    let out = scanline(&mut vdp, 8);
    assert_eq!(out[0], 0x13);
}

#[test]
fn column_and_line_scroll_select_their_table_entries() {
    // Column scroll: per-column words for plane A, plane B after 0x80 bytes.
    let mut vdp = base_vdp();
    write_cell(&mut vdp, PLANE_A_BASE, 32, 1, 0, 0x0801).expect("cell 1");
    write_cell(&mut vdp, PLANE_A_BASE, 32, 3, 0, 0x0803).expect("cell 3");
    write_solid_tile(&mut vdp, 1, 5).expect("tile 1");
    write_solid_tile(&mut vdp, 3, 9).expect("tile 3");
    write_register(&mut vdp, 0x0B, 0x01).expect("column scroll");
    write_vram(&mut vdp, 0x0400, &[8]).expect("column 0 scroll");
    write_vram(&mut vdp, 0x0402, &[16]).expect("column 1 scroll");

    let out = scanline(&mut vdp, 0);
    assert_eq!(out[0], 0x15, "column 0 scrolls to cell 1");
    assert_eq!(out[8], 0x19, "column 1 scrolls to cell 3");

    // Line scroll: two words per line (plane A, plane B).
    let mut vdp = base_vdp();
    write_cell(&mut vdp, PLANE_A_BASE, 32, 1, 0, 0x0801).expect("cell 1");
    write_cell(&mut vdp, PLANE_A_BASE, 32, 2, 0, 0x0802).expect("cell 2");
    write_solid_tile(&mut vdp, 1, 5).expect("tile 1");
    write_solid_tile(&mut vdp, 2, 7).expect("tile 2");
    write_register(&mut vdp, 0x0B, 0x02).expect("line scroll");
    write_vram(&mut vdp, 0x0400, &[8, 0, 16, 0]).expect("line table");

    let line_0 = scanline(&mut vdp, 0);
    assert_eq!(line_0[0], 0x15, "line 0 uses its own scroll word");
    let line_1 = scanline(&mut vdp, 1);
    assert_eq!(line_1[0], 0x17, "line 1 uses the next scroll word");
}

#[test]
fn the_undocumented_mode_falls_back_to_full_screen_scroll() {
    let mut vdp = base_vdp();
    write_cell(&mut vdp, PLANE_A_BASE, 32, 1, 0, 0x0801).expect("cell 1");
    write_cell(&mut vdp, PLANE_A_BASE, 32, 2, 0, 0x0802).expect("cell 2");
    write_solid_tile(&mut vdp, 1, 5).expect("tile 1");
    write_solid_tile(&mut vdp, 2, 7).expect("tile 2");
    write_register(&mut vdp, 0x0B, 0x03).expect("undocumented mode");
    write_vram(&mut vdp, 0x0400, &[8]).expect("full scroll word");

    // Mode 3 is documented as a full-screen-scroll fallback: both lines use
    // the single full-screen word, not the line table at +4.
    assert_eq!(scanline(&mut vdp, 0)[0], 0x15);
    assert_eq!(scanline(&mut vdp, 1)[0], 0x15);
}

#[test]
fn a_sixty_four_cell_plane_masks_the_nametable_address() {
    let mut vdp = base_vdp();
    write_register(&mut vdp, 0x10, 0x01).expect("64x32 planes");
    write_cell(&mut vdp, PLANE_A_BASE, 64, 0, 0, 0x0801).expect("cell 0");
    write_cell(&mut vdp, PLANE_A_BASE, 64, 32, 0, 0x0802).expect("cell 32");
    write_solid_tile(&mut vdp, 1, 5).expect("tile 1");
    write_solid_tile(&mut vdp, 2, 7).expect("tile 2");

    let out = scanline(&mut vdp, 0);
    assert_eq!(out[0], 0x15, "cell 0 at the left edge");
    assert_eq!(out[256], 0x17, "pixel 256 addresses cell 32, not cell 0");
}
