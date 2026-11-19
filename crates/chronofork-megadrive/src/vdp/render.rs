//! VDP scanline rendering: planes, window, sprites and frame composition.
//!
//! Rendering is a pure function of canonical device state and the line
//! position: it never reads host time, the environment or a window size, so two
//! runs from the same state produce byte-identical frames.
//!
//! # Layout
//!
//! - Plane A and plane B are nametables whose bases come from registers `0x02`
//!   and `0x04`; register `0x10` selects 32 or 64 cells in each direction.
//! - The window selects its nametable through register `0x03` and replaces
//!   plane A inside the rectangle selected by registers `0x11`/`0x12`
//!   (horizontal side and vertical side, combined as an intersection).
//! - Register `0x0B` selects full-screen, column or line scroll. The
//!   undocumented value 3 falls back to full-screen scrolling.
//! - Register `0x0C` bit 0 selects H40 (320 pixels); the video standard
//!   selects the 224- or 240-line active height.
//!
//! # Frame pixels
//!
//! A frame pixel is a palette index byte: bits `5..0` are the CRAM entry, bit 6
//! marks a shadowed pixel and bit 7 a highlighted pixel. Plane colour 0 is
//! transparent. The 6-level priority table is sprite-priority > plane-A
//! priority > plane-B priority > sprite > plane A > plane B, with the
//! register-`0x07` background colour underneath.
//!
//! **Documented approximations:** the physical byte-swapped/interleaved VRAM
//! layout and interlace are not modelled, the H40 window nametable is treated
//! as 64 cells wide (32 in H32), column-scroll VSRAM uses 20 entries per plane,
//! and line-mode vertical scrolling reuses the per-plane VSRAM words. These are
//! not claimed as dot-exact hardware behaviour.

use chronofork_core::{StateHash, hash_bytes};

use super::Vdp;

/// Maximum width of a rendered frame in pixels (H40 mode).
pub const MAX_FRAME_WIDTH: u16 = 320;
/// Maximum height of a rendered frame in lines (PAL V30).
pub const MAX_FRAME_HEIGHT: u16 = 240;
/// Mask of the CRAM entry in a frame pixel byte.
pub const PALETTE_INDEX_MASK: u8 = 0x3F;
/// Frame-pixel flag: the pixel is drawn in shadow.
pub const SHADOW_FLAG: u8 = 0x40;
/// Frame-pixel flag: the pixel is drawn highlighted.
pub const HIGHLIGHT_FLAG: u8 = 0x80;

/// A completed frame of palette-index bytes.
///
/// `indices` is row-major, one byte per pixel; see the module documentation for
/// the palette-index/operator encoding.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FrameBuffer {
    /// Frame width in pixels (256 or 320).
    pub width: u16,
    /// Frame height in lines (224 or 240).
    pub height: u16,
    /// Row-major frame pixels.
    pub indices: Vec<u8>,
}

impl FrameBuffer {
    /// Creates a frame filled with colour 0.
    #[must_use]
    pub fn new(width: u16, height: u16) -> Self {
        let length = usize::from(width) * usize::from(height);
        Self {
            width,
            height,
            indices: vec![0; length],
        }
    }

    /// Returns one pixel, or `None` when the coordinates are outside the frame.
    #[must_use]
    pub fn pixel(&self, x: u16, y: u16) -> Option<u8> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let index = usize::from(y) * usize::from(self.width) + usize::from(x);
        self.indices.get(index).copied()
    }

    /// Returns the SHA-256 digest of the frame's canonical bytes.
    ///
    /// The canonical sequence is little-endian width, little-endian height and
    /// the row-major pixel bytes, matching the media seam's
    /// [`VideoFrame`](chronofork_backend_api::media::VideoFrame) discipline
    /// without depending on it.
    #[must_use]
    pub fn checksum(&self) -> StateHash {
        let mut canonical = Vec::with_capacity(4 + self.indices.len());
        canonical.extend_from_slice(&self.width.to_le_bytes());
        canonical.extend_from_slice(&self.height.to_le_bytes());
        canonical.extend_from_slice(&self.indices);
        hash_bytes(&canonical)
    }
}

/// A plane nametable: base address, cell columns and cell rows.
#[derive(Clone, Copy)]
struct Plane {
    base: u16,
    columns: u16,
    rows: u16,
}

impl Plane {
    /// Returns the fixed 64x32-cell window nametable.
    fn window(base: u16, h40: bool) -> Self {
        Self {
            base,
            columns: if h40 { 64 } else { 32 },
            rows: 32,
        }
    }
}

/// One plane or window pixel before priority selection.
#[derive(Clone, Copy, Default)]
struct LayerPixel {
    /// CRAM palette index; 0 means transparent.
    colour: u8,
    /// Nametable priority bit.
    priority: bool,
}

impl LayerPixel {
    /// Returns the transient pixel's priority key; sprite rank 2, plane A 1,
    /// plane B 0, with the nametable priority bit adding six.
    fn key(self, rank: u8) -> u8 {
        (if self.priority { 6 } else { 0 }) + rank
    }
}

/// Scroll mode selected by register `0x0B` bits 1..0.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ScrollMode {
    /// One scroll word per plane for the whole screen.
    Full,
    /// One scroll word per cell column.
    Column,
    /// One scroll pair per line.
    Line,
}

// Nametable cell bit positions.
const CELL_TILE_MASK: u16 = 0x07FF;
const CELL_PALETTE_SHIFT: u16 = 11;
const CELL_VFLIP: u16 = 0x2000;
const CELL_HFLIP: u16 = 0x4000;
const CELL_PRIORITY: u16 = 0x8000;

/// Bytes per 8x8 4bpp tile.
const TILE_BYTES: u16 = 32;
/// Pixels per tile row and column.
const TILE_PIXELS: u16 = 8;
/// VSRAM words available per plane in column-scroll mode.
const COLUMN_SCROLL_WORDS: u16 = 20;
/// Byte offset of the plane-B column-scroll table.
const COLUMN_SCROLL_PLANE_B_OFFSET: u16 = 0x0080;

/// Maximum number of sprite attribute entries.
const MAX_SPRITES: usize = 80;
/// Per-line sprite count limit in H40 mode.
const MAX_SPRITES_PER_LINE_H40: u16 = 20;
/// Per-line sprite count limit in H32 mode.
const MAX_SPRITES_PER_LINE_H32: u16 = 16;
/// Per-line sprite pixel budget.
const MAX_SPRITE_PIXELS_PER_LINE: u16 = 320;
/// Sprite attribute word bit positions.
const SPRITE_Y_MASK: u16 = 0x03FF;
const SPRITE_WIDTH_SHIFT: u16 = 10;
const SPRITE_HEIGHT_SHIFT: u16 = 12;
const SPRITE_LINK_MASK: u16 = 0x007F;
const SPRITE_HFLIP: u16 = 0x0800;
const SPRITE_VFLIP: u16 = 0x1000;
const SPRITE_PALETTE_SHIFT: u16 = 13;
const SPRITE_PRIORITY: u16 = 0x8000;
const SPRITE_TILE_MASK: u16 = 0x07FF;
const SPRITE_X_MASK: u16 = 0x03FF;
const SPRITE_X_SIGN: u16 = 0x0200;
/// Duplicate offset applied to a sprite whose X starts left of the screen.
const SPRITE_DUPLICATE_OFFSET: i32 = 512;
/// Sprite colour indices that act as shadow/highlight operators.
const SPRITE_SHADOW_COLOUR: u8 = 0x3F;
const SPRITE_HIGHLIGHT_COLOUR: u8 = 0x3E;
/// Number of pixels in the widest scanline.
const LINE_PIXELS: usize = MAX_FRAME_WIDTH as usize;

/// One decoded sprite attribute entry.
#[derive(Clone, Copy)]
struct Sprite {
    index: u8,
    x: i32,
    top: i32,
    width_cells: u16,
    width_px: i32,
    height_px: i32,
    tile: u16,
    palette: u8,
    priority: bool,
    hflip: bool,
    vflip: bool,
    hidden: bool,
}

/// Per-line sprite state: the winning sprite pixel, operator flags, collision
/// and the budget observations.
struct SpriteLine {
    colour: [u8; LINE_PIXELS],
    priority: [bool; LINE_PIXELS],
    owner: [u8; LINE_PIXELS],
    shadow: [bool; LINE_PIXELS],
    highlight: [bool; LINE_PIXELS],
    collision: bool,
    overflow: bool,
    limit_reached: bool,
}

impl SpriteLine {
    fn new() -> Self {
        Self {
            colour: [0; LINE_PIXELS],
            priority: [false; LINE_PIXELS],
            owner: [0; LINE_PIXELS],
            shadow: [false; LINE_PIXELS],
            highlight: [false; LINE_PIXELS],
            collision: false,
            overflow: false,
            limit_reached: false,
        }
    }
}

impl Vdp {
    /// Renders one composited scanline into `out`.
    ///
    /// The frame line is `line`; `out` receives the first
    /// `min(out.len(), active width)` pixels of the line, so a caller normally
    /// supplies a slice exactly as wide as the active display (320 in H40, 256
    /// in H32). Rendering has no hidden host input.
    pub fn render_scanline(&mut self, line: u16, out: &mut [u8]) {
        let width = usize::from(self.active_width());
        let mut pixels = [0_u8; MAX_FRAME_WIDTH as usize];
        self.compose_scanline(line, &mut pixels[..width]);
        let count = out.len().min(width);
        out[..count].copy_from_slice(&pixels[..count]);
    }

    /// Returns the active display width in pixels.
    #[must_use]
    pub fn active_width(&self) -> u16 {
        if self.is_h40() { 320 } else { 256 }
    }

    /// Returns the active display height in lines.
    #[must_use]
    pub fn active_height(&self) -> u16 {
        match self.video {
            super::VideoStandard::Ntsc => 224,
            super::VideoStandard::Pal => 240,
        }
    }

    fn is_h40(&self) -> bool {
        self.registers[0x0C] & 0x01 != 0
    }

    fn background_colour(&self) -> u8 {
        self.registers[0x07] & PALETTE_INDEX_MASK
    }

    fn plane_a(&self) -> Plane {
        Plane {
            base: (u16::from(self.registers[0x02]) & 0x38) << 10,
            columns: self.plane_columns(),
            rows: self.plane_rows(),
        }
    }

    fn plane_b(&self) -> Plane {
        Plane {
            base: (u16::from(self.registers[0x04]) & 0x07) << 13,
            columns: self.plane_columns(),
            rows: self.plane_rows(),
        }
    }

    fn window_plane(&self) -> Plane {
        Plane::window(
            (u16::from(self.registers[0x03]) & 0x3E) << 10,
            self.is_h40(),
        )
    }

    fn plane_columns(&self) -> u16 {
        if self.registers[0x10] & 0x01 != 0 {
            64
        } else {
            32
        }
    }

    fn plane_rows(&self) -> u16 {
        if self.registers[0x10] & 0x02 != 0 {
            64
        } else {
            32
        }
    }

    fn scroll_mode(&self) -> ScrollMode {
        match self.registers[0x0B] & 0x03 {
            0 => ScrollMode::Full,
            1 => ScrollMode::Column,
            2 => ScrollMode::Line,
            // Documented fallback for the undocumented value 3.
            _ => ScrollMode::Full,
        }
    }

    fn hscroll_base(&self) -> u16 {
        (u16::from(self.registers[0x0D]) & 0x3F) << 10
    }

    /// Reads a big-endian VRAM word; the address wraps in the 64 KiB space.
    fn vram_word(&self, address: u16) -> u16 {
        let index = usize::from(address & 0xFFFE);
        u16::from_be_bytes([self.vram[index], self.vram[index + 1]])
    }

    fn tile_pixel(&self, tile: u16, pixel_x: u16, pixel_y: u16) -> u8 {
        let address = tile
            .wrapping_mul(TILE_BYTES)
            .wrapping_add(pixel_y * 4 + pixel_x / 2);
        let byte = self.vram[usize::from(address)];
        if pixel_x & 1 == 0 {
            byte >> 4
        } else {
            byte & 0x0F
        }
    }

    fn plane_pixel(&self, plane: Plane, x: u16, y: u16) -> LayerPixel {
        let plane_width = plane.columns * TILE_PIXELS;
        let plane_height = plane.rows * TILE_PIXELS;
        let pixel_x = x % plane_width;
        let pixel_y = y % plane_height;
        let cell_x = pixel_x / TILE_PIXELS;
        let cell_y = pixel_y / TILE_PIXELS;
        let cell_address = plane
            .base
            .wrapping_add((cell_y * plane.columns + cell_x) * 2);
        let cell = self.vram_word(cell_address);
        let palette = ((cell >> CELL_PALETTE_SHIFT) & 0x03) as u8;
        let mut tile_x = pixel_x % TILE_PIXELS;
        if cell & CELL_HFLIP != 0 {
            tile_x = TILE_PIXELS - 1 - tile_x;
        }
        let mut tile_y = pixel_y % TILE_PIXELS;
        if cell & CELL_VFLIP != 0 {
            tile_y = TILE_PIXELS - 1 - tile_y;
        }
        let colour = self.tile_pixel(cell & CELL_TILE_MASK, tile_x, tile_y);
        LayerPixel {
            colour: if colour == 0 {
                0
            } else {
                palette * 16 + colour
            },
            priority: cell & CELL_PRIORITY != 0,
        }
    }

    fn plane_hscroll(&self, plane_a: bool, x: u16, line: u16) -> u16 {
        let offset = match self.scroll_mode() {
            ScrollMode::Full => {
                if plane_a {
                    0
                } else {
                    2
                }
            }
            ScrollMode::Column => {
                let column = x / TILE_PIXELS;
                let plane_offset = if plane_a {
                    0
                } else {
                    COLUMN_SCROLL_PLANE_B_OFFSET
                };
                plane_offset + (column % COLUMN_SCROLL_WORDS) * 2
            }
            ScrollMode::Line => line * 4 + if plane_a { 0 } else { 2 },
        };
        self.vram_word(self.hscroll_base().wrapping_add(offset)) & 0x03FF
    }

    fn plane_vscroll(&self, plane_a: bool, column: u16) -> u16 {
        let value = match self.scroll_mode() {
            ScrollMode::Column => {
                let index = usize::from(column % COLUMN_SCROLL_WORDS);
                if plane_a {
                    self.vsram[index]
                } else {
                    self.vsram[usize::from(COLUMN_SCROLL_WORDS) + index]
                }
            }
            // Full and line modes use the per-plane vertical scroll words.
            ScrollMode::Full | ScrollMode::Line => {
                if plane_a {
                    self.vsram[0]
                } else {
                    self.vsram[1]
                }
            }
        };
        value & 0x03FF
    }

    fn scrolled_plane_pixel(&self, plane_a: bool, x: u16, y: u16) -> LayerPixel {
        let plane = if plane_a {
            self.plane_a()
        } else {
            self.plane_b()
        };
        let horizontal = self.plane_hscroll(plane_a, x, y);
        let vertical = self.plane_vscroll(plane_a, x / TILE_PIXELS);
        self.plane_pixel(plane, x.wrapping_add(horizontal), y.wrapping_add(vertical))
    }

    fn window_covers(&self, x: u16, y: u16) -> bool {
        let horizontal = self.registers[0x11];
        let vertical = self.registers[0x12];
        // A zero position disables the window on that axis, which is what the
        // reset state and programs that never use the window expect.
        let horizontal_cells = horizontal & 0x1F;
        let vertical_cells = vertical & 0x1F;
        let inside_horizontal = horizontal_cells != 0
            && if horizontal & 0x80 != 0 {
                x < u16::from(horizontal_cells) * TILE_PIXELS
            } else {
                x >= u16::from(horizontal_cells) * TILE_PIXELS
            };
        let inside_vertical = vertical_cells != 0
            && if vertical & 0x80 != 0 {
                y < u16::from(vertical_cells) * TILE_PIXELS
            } else {
                y >= u16::from(vertical_cells) * TILE_PIXELS
            };
        inside_horizontal && inside_vertical
    }

    /// Returns the per-line sprite count limit for the active mode.
    fn sprites_per_line(&self) -> u16 {
        if self.is_h40() {
            MAX_SPRITES_PER_LINE_H40
        } else {
            MAX_SPRITES_PER_LINE_H32
        }
    }

    fn sat_base(&self) -> u16 {
        (u16::from(self.registers[0x05]) & 0x7F) << 9
    }

    fn sprite_entry(&self, index: u16) -> Sprite {
        let address = self.sat_base().wrapping_add(index * 8);
        let size = self.vram_word(address);
        let attribution = self.vram_word(address.wrapping_add(2));
        let tile = self.vram_word(address.wrapping_add(4)) & SPRITE_TILE_MASK;
        let x_raw = self.vram_word(address.wrapping_add(6)) & SPRITE_X_MASK;
        let y_raw = size & SPRITE_Y_MASK;
        let width_cells = ((size >> SPRITE_WIDTH_SHIFT) & 0x03) + 1;
        let height_cells = ((size >> SPRITE_HEIGHT_SHIFT) & 0x03) + 1;
        // Sprite X is a signed ten-bit screen coordinate; Y is biased by 128
        // and the value 0 hides the sprite.
        let x = if x_raw & SPRITE_X_SIGN != 0 {
            i32::from(x_raw) - 0x400
        } else {
            i32::from(x_raw)
        };
        Sprite {
            index: index as u8,
            x,
            top: i32::from(y_raw) - 128,
            width_cells,
            width_px: i32::from(width_cells) * 8,
            height_px: i32::from(height_cells) * 8,
            tile,
            palette: ((attribution >> SPRITE_PALETTE_SHIFT) & 0x03) as u8,
            priority: attribution & SPRITE_PRIORITY != 0,
            hflip: attribution & SPRITE_HFLIP != 0,
            vflip: attribution & SPRITE_VFLIP != 0,
            hidden: y_raw == 0,
        }
    }

    fn shadow_highlight_enabled(&self) -> bool {
        self.registers[0x0C] & 0x08 != 0
    }

    fn evaluate_sprites(&mut self, line: u16) -> SpriteLine {
        let mut state = SpriteLine::new();
        let width = self.active_width();
        let mut on_line = 0_u16;
        let mut pixel_budget = 0_u32;
        let mut index = 0_u16;
        let mut processed = 0_u16;
        loop {
            // A link chain longer than the table, or a cycle, cannot be drawn:
            // report overflow and stop deterministically.
            if usize::from(index) >= MAX_SPRITES || processed >= MAX_SPRITES as u16 {
                state.overflow = true;
                break;
            }
            let sprite = self.sprite_entry(index);
            processed += 1;
            let link =
                self.vram_word(self.sat_base().wrapping_add(index * 8 + 2)) & SPRITE_LINK_MASK;
            let line_position = i32::from(line);
            if !sprite.hidden
                && line_position >= sprite.top
                && line_position < sprite.top + sprite.height_px
            {
                on_line += 1;
                pixel_budget += sprite.width_px as u32;
                self.draw_sprite(&sprite, line, width, &mut state);
            }
            if link == 0 {
                break;
            }
            index = link;
        }
        let per_line = self.sprites_per_line();
        state.overflow |=
            on_line > per_line || pixel_budget > u32::from(MAX_SPRITE_PIXELS_PER_LINE);
        state.limit_reached = state.overflow
            || on_line >= per_line
            || pixel_budget >= u32::from(MAX_SPRITE_PIXELS_PER_LINE);
        state
    }

    fn draw_sprite(&self, sprite: &Sprite, line: u16, width: u16, state: &mut SpriteLine) {
        let copies = if sprite.x < 0 { 2 } else { 1 };
        for copy in 0..copies {
            let left = sprite.x + copy * SPRITE_DUPLICATE_OFFSET;
            let start = left.max(0);
            let end = (left + sprite.width_px).min(i32::from(width));
            let mut screen_x = start;
            while screen_x < end {
                let mut local_x = screen_x - left;
                if sprite.hflip {
                    local_x = sprite.width_px - 1 - local_x;
                }
                let mut local_y = i32::from(line) - sprite.top;
                if sprite.vflip {
                    local_y = sprite.height_px - 1 - local_y;
                }
                let tile_x = (local_x / 8) as u16;
                let tile_y = (local_y / 8) as u16;
                let tile = sprite
                    .tile
                    .wrapping_add(tile_y * sprite.width_cells + tile_x);
                let colour = self.tile_pixel(tile, (local_x % 8) as u16, (local_y % 8) as u16);
                let column = screen_x as usize;
                if colour != 0 {
                    let palette_colour = sprite.palette * 16 + colour;
                    let operator = self.shadow_highlight_enabled()
                        && (palette_colour == SPRITE_SHADOW_COLOUR
                            || palette_colour == SPRITE_HIGHLIGHT_COLOUR);
                    if operator {
                        // Operators are invisible; they affect the layers below
                        // only when no visible sprite already covers the pixel.
                        if state.owner[column] == 0 {
                            if palette_colour == SPRITE_SHADOW_COLOUR {
                                state.shadow[column] = true;
                                state.highlight[column] = false;
                            } else if !state.shadow[column] {
                                state.highlight[column] = true;
                            }
                        }
                    } else if state.owner[column] == 0 {
                        state.owner[column] = sprite.index + 1;
                        state.colour[column] = palette_colour;
                        state.priority[column] = sprite.priority;
                    } else if state.owner[column] != sprite.index + 1 {
                        state.collision = true;
                    }
                }
                screen_x += 1;
            }
        }
    }

    fn compose_scanline(&mut self, line: u16, out: &mut [u8]) {
        let sprites = self.evaluate_sprites(line);
        for (x, pixel) in out.iter_mut().enumerate() {
            *pixel = self.compose_pixel(x as u16, line, &sprites);
        }
        self.status_latches.sprite_collision |= sprites.collision;
        self.status_latches.sprite_overflow |= sprites.overflow;
        self.sprite_limit_reached = sprites.limit_reached;
    }

    fn compose_pixel(&self, x: u16, y: u16, sprites: &SpriteLine) -> u8 {
        let plane_a = if self.window_covers(x, y) {
            self.plane_pixel(self.window_plane(), x, y)
        } else {
            self.scrolled_plane_pixel(true, x, y)
        };
        let plane_b = self.scrolled_plane_pixel(false, x, y);

        let mut colour = self.background_colour();
        let mut key = 0_u8;
        if plane_b.colour != 0 {
            colour = plane_b.colour;
            key = plane_b.key(0);
        }
        if plane_a.colour != 0 {
            let plane_a_key = plane_a.key(1);
            if plane_a_key > key {
                colour = plane_a.colour;
                key = plane_a_key;
            }
        }
        let column = usize::from(x);
        if sprites.colour[column] != 0 {
            let sprite_key = if sprites.priority[column] { 8 } else { 2 };
            if sprite_key > key {
                colour = sprites.colour[column];
            }
        }
        let mut pixel = colour;
        if sprites.shadow[column] {
            pixel |= SHADOW_FLAG;
        } else if sprites.highlight[column] {
            pixel |= HIGHLIGHT_FLAG;
        }
        pixel
    }

    /// Returns whether the most recently rendered line reached the per-line
    /// sprite count or pixel budget.
    ///
    /// This is a render observation derived from the last scanline, not
    /// canonical state: it is not snapshotted and is reset by a restore.
    #[must_use]
    pub const fn sprite_limit_reached(&self) -> bool {
        self.sprite_limit_reached
    }

    /// Returns the latched sprite-collision status bit (bit 5).
    #[must_use]
    pub const fn sprite_collision_latched(&self) -> bool {
        self.status_latches.sprite_collision
    }

    /// Returns the latched sprite-overflow status bit (bit 6).
    #[must_use]
    pub const fn sprite_overflow_latched(&self) -> bool {
        self.status_latches.sprite_overflow
    }
}
