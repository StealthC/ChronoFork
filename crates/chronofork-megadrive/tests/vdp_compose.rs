#![allow(clippy::expect_used)]

//! Frame composition: priority, shadow/highlight, border and determinism.
//!
//! `finish_frame` renders the active display into a [`FrameBuffer`] of palette
//! index bytes. Bits `5..0` are the CRAM entry, bit 6 marks a shadowed pixel and
//! bit 7 a highlighted pixel; the RGBA conversion that applies the intensity
//! belongs to the media layer (phase 8.3 task 11). The register-`0x07`
//! background colour fills the H40 side borders.
//!
//! Two runs from the same state must produce byte-identical frame bytes and the
//! same checksum.

use chronofork_megadrive::VideoStandard;
use chronofork_megadrive::vdp::{
    CONTROL_PORT_OFFSET, DATA_PORT_OFFSET, HIGHLIGHT_FLAG, SHADOW_FLAG, Vdp, VdpError,
};

const PLANE_A_BASE: u16 = 0xE000;
const SAT_BASE: u16 = 0x8000;

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

fn write_solid_tile(vdp: &mut Vdp, tile: u16, colour: u8) -> Result<(), VdpError> {
    let packed = (colour << 4) | colour;
    let word = u16::from_be_bytes([packed, packed]);
    write_vram(vdp, tile.wrapping_mul(32), &[word; 16])
}

fn write_cell(
    vdp: &mut Vdp,
    columns: u16,
    cell_x: u16,
    cell_y: u16,
    word: u16,
) -> Result<(), VdpError> {
    let address = PLANE_A_BASE + (cell_y * columns + cell_x) * 2;
    write_vram(vdp, address, &[word])
}

fn fill_plane_row(vdp: &mut Vdp, line_cell: u16) -> Result<(), VdpError> {
    for column in 0..32 {
        write_cell(vdp, 32, column, line_cell, 0x0800)?;
    }
    Ok(())
}

/// Writes one 1x1 sprite entry at `index`.
fn write_sprite(
    vdp: &mut Vdp,
    index: u16,
    x: i16,
    y: i16,
    tile: u16,
    palette: u8,
    link: u8,
) -> Result<(), VdpError> {
    let y_raw = ((i32::from(y) + 128) & 0x03FF) as u16;
    let x_raw = (i32::from(x) & 0x03FF) as u16;
    let attribution = u16::from(link) | (u16::from(palette) << 13);
    let address = SAT_BASE + index * 8;
    write_vram(vdp, address, &[y_raw, attribution, tile, x_raw])
}

fn base_vdp(h40: bool, shadow_highlight: bool) -> Vdp {
    let mut vdp = Vdp::new();
    write_register(&mut vdp, 0x02, 0x38).expect("plane A base");
    write_register(&mut vdp, 0x04, 0x01).expect("plane B base");
    write_register(&mut vdp, 0x05, 0x40).expect("SAT base");
    write_register(&mut vdp, 0x10, 0x00).expect("32x32 planes");
    write_register(&mut vdp, 0x0B, 0x00).expect("full scroll");
    write_register(&mut vdp, 0x0D, 0x01).expect("hscroll table");
    let mode = u8::from(h40) | (u8::from(shadow_highlight) << 3);
    write_register(&mut vdp, 0x0C, mode).expect("mode register");
    vdp
}

#[test]
fn shadow_operator_darkens_lower_priority_pixels() {
    let mut vdp = base_vdp(false, true);
    fill_plane_row(&mut vdp, 0).expect("plane row");
    write_solid_tile(&mut vdp, 0, 3).expect("plane tile");
    // Palette-3 colour-15 is the shadow operator; it is invisible itself.
    write_solid_tile(&mut vdp, 6, 15).expect("shadow operator tile");
    write_sprite(&mut vdp, 0, 0, 0, 6, 3, 0).expect("shadow sprite");

    let frame = vdp.finish_frame();
    assert_eq!(frame.width, 256, "H32 mode");
    assert_eq!(frame.height, 224, "NTSC active lines");
    assert_eq!(
        frame.pixel(0, 0),
        Some(0x13 | SHADOW_FLAG),
        "the plane pixel under the operator is shadowed"
    );
    assert_eq!(
        frame.pixel(8, 0),
        Some(0x13),
        "outside the operator the plane is normal"
    );

    // Palette-3 colour-14 is the highlight operator.
    let mut vdp = base_vdp(false, true);
    fill_plane_row(&mut vdp, 0).expect("plane row");
    write_solid_tile(&mut vdp, 0, 3).expect("plane tile");
    write_solid_tile(&mut vdp, 7, 14).expect("highlight operator tile");
    write_sprite(&mut vdp, 0, 0, 0, 7, 3, 0).expect("highlight sprite");
    let frame = vdp.finish_frame();
    assert_eq!(frame.pixel(0, 0), Some(0x13 | HIGHLIGHT_FLAG));
}

#[test]
fn shadow_wins_over_highlight_on_the_same_pixel() {
    let mut vdp = base_vdp(false, true);
    fill_plane_row(&mut vdp, 0).expect("plane row");
    write_solid_tile(&mut vdp, 0, 3).expect("plane tile");
    write_solid_tile(&mut vdp, 6, 15).expect("shadow tile");
    write_solid_tile(&mut vdp, 7, 14).expect("highlight tile");
    // Highlight first in link order, shadow below it.
    write_sprite(&mut vdp, 0, 0, 0, 7, 3, 1).expect("highlight sprite");
    write_sprite(&mut vdp, 1, 0, 0, 6, 3, 0).expect("shadow sprite");

    let frame = vdp.finish_frame();
    assert_eq!(
        frame.pixel(0, 0),
        Some(0x13 | SHADOW_FLAG),
        "shadow takes precedence over highlight"
    );
}

#[test]
fn operator_colours_are_normal_pixels_when_shadow_is_disabled() {
    let mut vdp = base_vdp(false, false);
    fill_plane_row(&mut vdp, 0).expect("plane row");
    write_solid_tile(&mut vdp, 0, 3).expect("plane tile");
    write_solid_tile(&mut vdp, 6, 15).expect("palette-3 colour-15 tile");
    write_sprite(&mut vdp, 0, 0, 0, 6, 3, 0).expect("sprite");

    let frame = vdp.finish_frame();
    assert_eq!(
        frame.pixel(0, 0),
        Some(0x3F),
        "without shadow/highlight the colour is a normal sprite pixel"
    );
}

#[test]
fn border_fills_the_h40_side_columns() {
    let mut vdp = base_vdp(true, false);
    // Background colour palette 2, colour 7.
    write_register(&mut vdp, 0x07, 0x27).expect("background colour");
    fill_plane_row(&mut vdp, 0).expect("plane row");
    write_solid_tile(&mut vdp, 0, 3).expect("plane tile");

    let frame = vdp.finish_frame();
    assert_eq!(frame.width, 320);
    assert_eq!(frame.height, 224);
    for x in 0..8 {
        assert_eq!(frame.pixel(x, 0), Some(0x27), "left border at x={x}");
    }
    assert_eq!(
        frame.pixel(8, 0),
        Some(0x13),
        "plane starts after the border"
    );
    assert_eq!(frame.pixel(311, 0), Some(0x13), "plane reaches the border");
    for x in 312..320 {
        assert_eq!(frame.pixel(x, 0), Some(0x27), "right border at x={x}");
    }

    // H32 has no side border inside the captured 256-pixel frame.
    let mut vdp = base_vdp(false, false);
    write_register(&mut vdp, 0x07, 0x27).expect("background colour");
    fill_plane_row(&mut vdp, 0).expect("plane row");
    write_solid_tile(&mut vdp, 0, 3).expect("plane tile");
    let frame = vdp.finish_frame();
    assert_eq!(frame.width, 256);
    assert_eq!(frame.pixel(0, 0), Some(0x13));
}

#[test]
fn priority_table_orders_sprites_planes_and_layers() {
    // Plane B with priority beats a non-priority sprite.
    let mut vdp = base_vdp(false, false);
    write_cell(&mut vdp, 32, 0, 0, 0x1002).expect("plane A cell");
    write_vram(&mut vdp, 0x2000, &[0x9002]).expect("plane B priority cell");
    write_solid_tile(&mut vdp, 0, 3).expect("plane tile");
    write_solid_tile(&mut vdp, 2, 7).expect("plane B tile");
    write_solid_tile(&mut vdp, 1, 5).expect("sprite tile");
    write_sprite(&mut vdp, 0, 0, 0, 1, 2, 0).expect("normal sprite");
    let frame = vdp.finish_frame();
    assert_eq!(
        frame.pixel(0, 0),
        Some(0x27),
        "priority plane B beats a normal sprite"
    );

    // A priority sprite beats a priority plane.
    let mut vdp = base_vdp(false, false);
    write_cell(&mut vdp, 32, 0, 0, 0x8800).expect("plane A priority cell");
    write_solid_tile(&mut vdp, 0, 3).expect("plane tile");
    write_solid_tile(&mut vdp, 1, 5).expect("sprite tile");
    let y_raw = 128_u16;
    let attribution = 0x8000_u16 | (2 << 13);
    write_vram(&mut vdp, SAT_BASE, &[y_raw, attribution, 1, 0]).expect("priority sprite");
    let frame = vdp.finish_frame();
    assert_eq!(
        frame.pixel(0, 0),
        Some(0x25),
        "priority sprite beats a priority plane"
    );
}

#[test]
fn frame_is_deterministic_and_sized_by_mode() {
    let build = || {
        let mut vdp = base_vdp(true, true);
        fill_plane_row(&mut vdp, 0).expect("plane row");
        write_solid_tile(&mut vdp, 0, 3).expect("plane tile");
        write_solid_tile(&mut vdp, 6, 15).expect("shadow tile");
        write_sprite(&mut vdp, 0, 16, 0, 6, 3, 0).expect("sprite");
        vdp
    };

    let mut first = build();
    let frame = first.finish_frame();
    assert_eq!(frame.width, 320);
    assert_eq!(frame.height, 224);
    assert_eq!(frame.indices.len(), 320 * 224);

    let mut second = build();
    let other = second.finish_frame();
    assert_eq!(frame.indices, other.indices, "byte-identical frames");
    assert_eq!(frame.checksum(), other.checksum());

    // Rendering the same state twice is deterministic.
    let again = first.finish_frame();
    assert_eq!(frame.indices, again.indices);
    assert_eq!(frame.checksum(), again.checksum());

    // PAL raises the active height to 240 lines.
    let mut pal = build();
    pal.advance_master(0, VideoStandard::Pal);
    let frame = pal.finish_frame();
    assert_eq!(frame.height, 240);
    assert_eq!(frame.indices.len(), 320 * 240);

    // H32 narrows the frame to 256 pixels.
    let mut h32 = base_vdp(false, false);
    let frame = h32.finish_frame();
    assert_eq!(frame.width, 256);
    assert_eq!(frame.height, 224);
}

#[test]
fn frame_start_clears_and_relatches_sprite_status() {
    let mut vdp = base_vdp(false, false);
    write_solid_tile(&mut vdp, 1, 5).expect("sprite tile");
    write_sprite(&mut vdp, 0, 0, 0, 1, 2, 1).expect("sprite 0");
    write_sprite(&mut vdp, 1, 0, 0, 1, 2, 0).expect("sprite 1");

    let _ = vdp.finish_frame();
    assert!(vdp.sprite_collision_latched());
    assert_eq!(
        vdp.status() & chronofork_megadrive::vdp::STATUS_SPRITE_COLLISION,
        chronofork_megadrive::vdp::STATUS_SPRITE_COLLISION
    );

    // Hiding both sprites clears the latches on the next completed frame.
    write_vram(&mut vdp, SAT_BASE, &[0_u16; 16]).expect("hide sprites");
    let _ = vdp.finish_frame();
    assert!(!vdp.sprite_collision_latched());
    assert!(!vdp.sprite_overflow_latched());
    assert!(!vdp.sprite_limit_reached());
}

#[test]
fn frame_pixels_expose_the_documented_operator_flags() {
    assert_eq!(SHADOW_FLAG, 0x40);
    assert_eq!(HIGHLIGHT_FLAG, 0x80);
    let frame = chronofork_megadrive::vdp::FrameBuffer::new(2, 2);
    assert_eq!(frame.pixel(0, 0), Some(0));
    assert_eq!(frame.pixel(2, 0), None);
    assert_eq!(frame.pixel(0, 2), None);
    assert_ne!(frame.checksum().as_bytes(), &[0; 32]);
}
