#![allow(clippy::expect_used)]

//! Sprite rendering, link order, collision and overflow coverage.
//!
//! Sprites live in the 80-entry sprite attribute table selected by register
//! `0x05`. Each 8-byte entry holds Y, size/link, tile/attribution and X. The
//! link field chains sprites in drawing order: the first entry at a pixel wins
//! and a second opaque sprite at the same pixel latches the collision status
//! bit. The per-line sprite count and pixel budget latch the overflow bit.

use chronofork_megadrive::vdp::{
    CONTROL_PORT_OFFSET, DATA_PORT_OFFSET, STATUS_FIFO_EMPTY, STATUS_SPRITE_COLLISION,
    STATUS_SPRITE_OVERFLOW, Vdp, VdpError,
};

const WIDTH_H40: usize = 320;
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
    base: u16,
    columns: u16,
    cell_x: u16,
    cell_y: u16,
    word: u16,
) -> Result<(), VdpError> {
    let address = base.wrapping_add((cell_y * columns + cell_x) * 2);
    write_vram(vdp, address, &[word])
}

struct Sprite {
    index: u16,
    x: i16,
    y: i16,
    width_cells: u8,
    height_cells: u8,
    tile: u16,
    palette: u8,
    priority: bool,
    hflip: bool,
    vflip: bool,
    link: u8,
}

impl Sprite {
    fn simple(index: u16, x: i16, y: i16, tile: u16, palette: u8) -> Self {
        Self {
            index,
            x,
            y,
            width_cells: 1,
            height_cells: 1,
            tile,
            palette,
            priority: false,
            hflip: false,
            vflip: false,
            link: 0,
        }
    }
}

fn write_sprite(vdp: &mut Vdp, sprite: &Sprite) -> Result<(), VdpError> {
    let y_raw = ((i32::from(sprite.y) + 128) & 0x03FF) as u16;
    let x_raw = (i32::from(sprite.x) & 0x03FF) as u16;
    let size_word = y_raw
        | (u16::from(sprite.width_cells - 1) << 10)
        | (u16::from(sprite.height_cells - 1) << 12);
    let attribution = u16::from(sprite.link)
        | (u16::from(sprite.hflip) << 11)
        | (u16::from(sprite.vflip) << 12)
        | (u16::from(sprite.palette) << 13)
        | (u16::from(sprite.priority) << 15);
    let address = SAT_BASE + sprite.index * 8;
    write_vram(vdp, address, &[size_word, attribution, sprite.tile, x_raw])
}

fn base_vdp() -> Vdp {
    let mut vdp = Vdp::new();
    write_register(&mut vdp, 0x02, 0x38).expect("plane A base");
    write_register(&mut vdp, 0x04, 0x01).expect("plane B base");
    write_register(&mut vdp, 0x05, 0x40).expect("SAT base");
    write_register(&mut vdp, 0x0C, 0x01).expect("H40 mode");
    write_register(&mut vdp, 0x10, 0x00).expect("32x32 planes");
    write_register(&mut vdp, 0x0B, 0x00).expect("full-screen scroll");
    write_register(&mut vdp, 0x0D, 0x01).expect("hscroll table at 0x0400");
    vdp
}

fn scanline(vdp: &mut Vdp, line: u16) -> [u8; WIDTH_H40] {
    let mut out = [0_u8; WIDTH_H40];
    vdp.render_scanline(line, &mut out);
    out
}

#[test]
fn sprite_over_plane_priority_and_collision_latch() {
    let mut vdp = base_vdp();
    // Plane A shows palette-1 colour 3 everywhere on the line.
    for column in 0..32 {
        write_cell(&mut vdp, PLANE_A_BASE, 32, column, 0, 0x0800).expect("plane cell");
    }
    write_solid_tile(&mut vdp, 0, 3).expect("plane tile");
    write_solid_tile(&mut vdp, 1, 5).expect("sprite tile");

    let mut sprite = Sprite::simple(0, 0, 0, 1, 2);
    sprite.link = 1;
    write_sprite(&mut vdp, &sprite).expect("sprite 0");
    let overlapping = Sprite::simple(1, 0, 0, 2, 3);
    write_solid_tile(&mut vdp, 2, 7).expect("second sprite tile");
    write_sprite(&mut vdp, &overlapping).expect("sprite 1");

    assert!(!vdp.sprite_collision_latched());
    let out = scanline(&mut vdp, 0);
    assert_eq!(out[0], 0x25, "sprite 0 wins in link order");
    assert_eq!(out[8], 0x13, "the plane shows where the sprite ends");
    assert!(
        vdp.sprite_collision_latched(),
        "two sprite pixels overlapped"
    );
    assert_eq!(
        vdp.status() & STATUS_SPRITE_COLLISION,
        STATUS_SPRITE_COLLISION,
        "collision is observable through the status port"
    );
    assert_eq!(
        vdp.status() & STATUS_SPRITE_OVERFLOW,
        0,
        "two sprites do not overflow"
    );
}

#[test]
fn more_than_twenty_sprites_sets_the_overflow_latch() {
    let mut vdp = base_vdp();
    write_solid_tile(&mut vdp, 1, 5).expect("sprite tile");
    for index in 0..21_u16 {
        let mut sprite = Sprite::simple(index, (index as i16) * 8, 0, 1, 2);
        sprite.link = if index == 20 { 0 } else { (index + 1) as u8 };
        write_sprite(&mut vdp, &sprite).expect("sprite");
    }

    assert!(!vdp.sprite_overflow_latched());
    let _out = scanline(&mut vdp, 0);
    assert!(
        vdp.sprite_overflow_latched(),
        "21 sprites exceed 20 per line"
    );
    assert!(vdp.sprite_limit_reached(), "the per-line limit was reached");
    assert_eq!(
        vdp.status() & STATUS_SPRITE_OVERFLOW,
        STATUS_SPRITE_OVERFLOW
    );
    assert_eq!(vdp.status() & STATUS_SPRITE_COLLISION, 0);
}

#[test]
fn more_than_320_sprite_pixels_sets_the_overflow_latch() {
    let mut vdp = base_vdp();
    write_solid_tile(&mut vdp, 1, 5).expect("sprite tile");
    // Eleven 4-cell-wide sprites cover 352 pixels, exceeding the 320 budget.
    for index in 0..11_u16 {
        let mut sprite = Sprite::simple(index, (index as i16) * 32, 0, 1, 2);
        sprite.width_cells = 4;
        sprite.link = if index == 10 { 0 } else { (index + 1) as u8 };
        write_sprite(&mut vdp, &sprite).expect("sprite");
    }

    let _out = scanline(&mut vdp, 0);
    assert!(
        vdp.sprite_overflow_latched(),
        "352 sprite pixels exceed the 320-pixel budget"
    );
    assert!(vdp.sprite_limit_reached());
}

#[test]
fn sprite_size_and_flip_render_expected_cells() {
    let mut vdp = base_vdp();
    for (tile, colour) in [(1_u16, 5_u8), (2, 7), (3, 9), (4, 11)] {
        write_solid_tile(&mut vdp, tile, colour).expect("sprite tile");
    }

    // 4x4-cell sprite at the top-left: each cell shows its own tile.
    let mut sprite = Sprite::simple(0, 0, 0, 1, 2);
    sprite.width_cells = 4;
    sprite.height_cells = 4;
    write_sprite(&mut vdp, &sprite).expect("4x4 sprite");

    let out = scanline(&mut vdp, 0);
    assert_eq!(&out[..8], &[0x25; 8], "cell 0");
    assert_eq!(&out[8..16], &[0x27; 8], "cell 1");
    assert_eq!(&out[16..24], &[0x29; 8], "cell 2");
    assert_eq!(&out[24..32], &[0x2B; 8], "cell 3");

    // HFlip mirrors the 4 cells horizontally.
    sprite.hflip = true;
    write_sprite(&mut vdp, &sprite).expect("flipped sprite");
    let out = scanline(&mut vdp, 0);
    assert_eq!(&out[..8], &[0x2B; 8], "mirrored cell 3");
    assert_eq!(&out[24..32], &[0x25; 8], "mirrored cell 0");

    // VFlip mirrors a 1x2-cell sprite vertically: line 0 uses tile 2.
    let mut tall = Sprite::simple(0, 0, 0, 1, 2);
    tall.height_cells = 2;
    tall.vflip = true;
    write_sprite(&mut vdp, &tall).expect("vflipped sprite");
    let out = scanline(&mut vdp, 0);
    assert_eq!(&out[..8], &[0x27; 8], "top line shows the mirrored tile");
    let out = scanline(&mut vdp, 8);
    assert_eq!(&out[..8], &[0x25; 8], "second line shows the first tile");
}

#[test]
fn a_sprite_starting_left_of_the_screen_is_duplicated() {
    let mut vdp = base_vdp();
    write_solid_tile(&mut vdp, 1, 5).expect("sprite tile");
    // X = -200: the first copy is off-screen; the hardware duplicates it at
    // X + 512 = 312, which is visible at the right edge.
    let sprite = Sprite::simple(0, -200, 0, 1, 2);
    write_sprite(&mut vdp, &sprite).expect("sprite");

    let out = scanline(&mut vdp, 0);
    assert_eq!(&out[312..320], &[0x25; 8], "duplicated copy is visible");
    assert_eq!(out[0], 0x00, "the off-screen copy draws nothing at x=0");
}

#[test]
fn a_sprite_with_y_zero_is_hidden() {
    let mut vdp = base_vdp();
    write_solid_tile(&mut vdp, 1, 5).expect("sprite tile");
    let sprite = Sprite::simple(0, 0, -128, 1, 2);
    write_sprite(&mut vdp, &sprite).expect("hidden sprite");
    let out = scanline(&mut vdp, 0);
    assert_eq!(out[0], 0x00);
    assert!(!vdp.sprite_collision_latched());
}

#[test]
fn a_sprite_priority_bit_wins_over_a_priority_plane() {
    let mut vdp = base_vdp();
    write_cell(&mut vdp, PLANE_A_BASE, 32, 0, 0, 0x8800).expect("priority plane");
    write_solid_tile(&mut vdp, 0, 3).expect("plane tile");
    write_solid_tile(&mut vdp, 1, 5).expect("sprite tile");
    let mut sprite = Sprite::simple(0, 0, 0, 1, 2);
    sprite.priority = true;
    write_sprite(&mut vdp, &sprite).expect("priority sprite");
    assert_eq!(scanline(&mut vdp, 0)[0], 0x25);

    // A non-priority sprite loses to the priority plane.
    let mut sprite = Sprite::simple(0, 0, 0, 1, 2);
    sprite.priority = false;
    write_sprite(&mut vdp, &sprite).expect("plain sprite");
    assert_eq!(scanline(&mut vdp, 0)[0], 0x13);
}

#[test]
fn status_starts_without_sprite_latches() {
    let vdp = Vdp::new();
    assert_eq!(vdp.status(), STATUS_FIFO_EMPTY);
    assert!(!vdp.sprite_collision_latched());
    assert!(!vdp.sprite_overflow_latched());
    assert!(!vdp.sprite_limit_reached());
}
