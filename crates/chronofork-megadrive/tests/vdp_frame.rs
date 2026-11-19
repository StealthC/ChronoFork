#![allow(clippy::expect_used)]

//! Frame production through the media seam.

use chronofork_backend_api::{Backend, MediaSource};
use chronofork_core::{InputFrame, StateHash};
use chronofork_megadrive::vdp::{
    CONTROL_PORT_OFFSET, DATA_PORT_OFFSET, HIGHLIGHT_FLAG, SHADOW_FLAG, Vdp, VdpError,
};
use chronofork_megadrive::{MegadriveFactory, demo_rom};

fn machine() -> chronofork_megadrive::MegadriveBackend {
    MegadriveFactory::new(demo_rom())
        .expect("factory")
        .create_machine()
        .expect("machine")
}

fn first_frame() -> (u16, u16, Vec<u8>, StateHash) {
    let mut machine = machine();
    assert!(machine.media_mut().is_some(), "media seam is advertised");
    let mut found = None;
    for _ in 0..200_000 {
        machine.step(InputFrame::NEUTRAL).expect("step");
        found = machine.take_video_frame();
        if found.is_some() {
            break;
        }
    }
    let frame = found.expect("a frame is produced");
    let checksum = frame.checksum();
    (frame.width, frame.height, frame.rgba, checksum)
}

#[test]
fn frame_is_produced_once_per_frame_with_canonical_rgba() {
    let (width, height, rgba, _) = first_frame();
    assert!(matches!(width, 256 | 320), "H32/H40 width");
    assert_eq!(height, 224, "NTSC active height");
    assert_eq!(rgba.len(), usize::from(width) * usize::from(height) * 4);
    assert!(rgba.chunks_exact(4).all(|pixel| pixel[3] == 0xFF));
}

#[test]
fn a_second_take_before_a_new_frame_returns_none_and_audio_is_absent() {
    let mut machine = machine();
    let mut first = None;
    for _ in 0..200_000 {
        machine.step(InputFrame::NEUTRAL).expect("step");
        if let Some(frame) = machine.take_video_frame() {
            first = Some(frame.checksum());
            break;
        }
    }
    assert!(first.is_some(), "a frame was produced");
    assert!(machine.take_video_frame().is_none(), "one-frame queue");
    assert!(
        machine.take_audio_chunk().is_none(),
        "audio is not implemented"
    );
}

#[test]
fn frame_hashes_are_deterministic_across_identical_runs() {
    let (_, _, _, first) = first_frame();
    let (_, _, _, second) = first_frame();
    assert_eq!(first, second, "identical state produces the same frame");
}

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

fn fill_plane_row(vdp: &mut Vdp, line_cell: u16) -> Result<(), VdpError> {
    for column in 0..32 {
        let address = PLANE_A_BASE + (line_cell * 32 + column) * 2;
        write_vram(vdp, address, &[0x0800])?;
    }
    Ok(())
}

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
    write_vram(
        vdp,
        SAT_BASE + index * 8,
        &[y_raw, attribution, tile, x_raw],
    )
}

/// Deterministic scene: a plane-A row, a normal tile and one shadow and one
/// highlight operator sprite. The frame checksum pins the renderer output.
fn fixture_scene(h40: bool) -> Vdp {
    let mut vdp = Vdp::new();
    write_register(&mut vdp, 0x02, 0x38).expect("plane A base");
    write_register(&mut vdp, 0x04, 0x01).expect("plane B base");
    write_register(&mut vdp, 0x05, 0x40).expect("SAT base");
    write_register(&mut vdp, 0x10, 0x00).expect("32x32 planes");
    write_register(&mut vdp, 0x0B, 0x00).expect("full scroll");
    write_register(&mut vdp, 0x0D, 0x01).expect("hscroll table");
    write_register(&mut vdp, 0x0C, u8::from(h40) | 0x08).expect("H32/H40 plus shadow");
    fill_plane_row(&mut vdp, 0).expect("plane row");
    write_solid_tile(&mut vdp, 0, 3).expect("plane tile");
    write_solid_tile(&mut vdp, 6, 15).expect("shadow operator");
    write_solid_tile(&mut vdp, 7, 14).expect("highlight operator");
    write_sprite(&mut vdp, 0, 0, 0, 6, 3, 1).expect("shadow sprite");
    write_sprite(&mut vdp, 1, 24, 0, 7, 3, 0).expect("highlight sprite");
    vdp
}

/// Pins the renderer's full-frame output, not only determinism across runs. The
/// fixture includes plane pixels, a shadow operator and a highlight operator in
/// both H32 and H40, so a composition change that is internally consistent still
/// fails this fixture.
#[test]
fn fixed_frame_sha256_fixtures_pin_h32_and_h40_output() {
    let h32 = fixture_scene(false).finish_frame();
    let h40 = fixture_scene(true).finish_frame();
    assert_eq!((h32.width, h32.height), (256, 224));
    assert_eq!((h40.width, h40.height), (320, 224));
    assert_eq!(
        h32.pixel(0, 0),
        Some(0x13 | SHADOW_FLAG),
        "the shadow operator covers the first plane pixel"
    );
    assert_eq!(
        h32.pixel(24, 0),
        Some(0x13 | HIGHLIGHT_FLAG),
        "the highlight operator covers the second sprite position"
    );
    assert_eq!(
        h32.checksum().to_hex(),
        "43e111026c99e6fe3b30693aed707bf3cb5ce1143d2bfffd507c2d03e49d638f",
        "H32 frame checksum fixture"
    );
    assert_eq!(
        h40.checksum().to_hex(),
        "3fb74d04b24dd9b355365b375da8edaadef5cddd97624e687587513da3517056",
        "H40 frame checksum fixture"
    );
}
