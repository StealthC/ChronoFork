//! Trace record taxonomy and validated kind masks.

use chronofork_core::Tick;

use crate::error::TraceError;

/// Bits reserved by the five v1 record kinds.
const KIND_MASK_BITS: u8 = 0b0001_1111;

/// One class of trace record.
///
/// The five classes are exactly the v1 taxonomy: instruction, bus write, VDP
/// port access, controller read and interrupt phase. There is deliberately no
/// generic bus-read class because no backend observes generic data reads.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TraceKind {
    /// One instruction boundary observed through the `pc` register.
    Instruction,
    /// One emulated work-RAM byte write reported by `DebugAccess`.
    BusWrite,
    /// One VDP port access reported by `DebugAccess`.
    VdpPort,
    /// One controller data-register read reported by `DebugAccess`.
    ControllerRead,
    /// One interrupt request, delivery or acknowledgement phase.
    Interrupt,
}

impl TraceKind {
    /// Every kind in canonical mask order.
    pub const ALL: [Self; 5] = [
        Self::Instruction,
        Self::BusWrite,
        Self::VdpPort,
        Self::ControllerRead,
        Self::Interrupt,
    ];

    /// Returns the stable binary kind code.
    ///
    /// The code is the record's leading byte in the canonical format, so the
    /// mapping must not change within format version 1.
    #[must_use]
    pub const fn code(self) -> u8 {
        match self {
            Self::Instruction => 0,
            Self::BusWrite => 1,
            Self::VdpPort => 2,
            Self::ControllerRead => 3,
            Self::Interrupt => 4,
        }
    }

    /// Returns the stable lowercase kind name used by summaries and TSV export.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Instruction => "instruction",
            Self::BusWrite => "bus_write",
            Self::VdpPort => "vdp_port",
            Self::ControllerRead => "controller_read",
            Self::Interrupt => "interrupt",
        }
    }

    /// Returns the kind's one-bit contribution to a [`TraceKindMask`].
    #[must_use]
    pub const fn bit(self) -> u8 {
        1 << self.code()
    }
}

/// A validated set of enabled [`TraceKind`]s.
///
/// Bits `0..=4` correspond to [`TraceKind::ALL`] order. The zero mask is valid
/// and means "capture no records"; [`TraceKindMask::default`] enables all five
/// kinds. Construction from raw bits rejects any undefined bit instead of
/// ignoring it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TraceKindMask(u8);

impl TraceKindMask {
    /// A mask enabling every record kind.
    #[must_use]
    pub const fn all() -> Self {
        Self(KIND_MASK_BITS)
    }

    /// A mask enabling no record kind.
    #[must_use]
    pub const fn none() -> Self {
        Self(0)
    }

    /// Builds a mask from raw bits, rejecting undefined bits.
    ///
    /// # Errors
    ///
    /// Returns [`TraceError::InvalidConfiguration`] when `bits` sets a bit above
    /// bit 4. Undefined bits are rejected rather than silently discarded.
    pub fn from_bits(bits: u8) -> Result<Self, TraceError> {
        if bits & !KIND_MASK_BITS != 0 {
            return Err(TraceError::InvalidConfiguration(format!(
                "trace kind mask 0b{bits:08b} sets undefined bits above bit 4"
            )));
        }
        Ok(Self(bits))
    }

    /// Returns the raw mask bits.
    #[must_use]
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// Returns whether `kind` is enabled.
    #[must_use]
    pub const fn contains(self, kind: TraceKind) -> bool {
        self.0 & kind.bit() != 0
    }

    /// Returns whether no kind is enabled.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Returns a mask with `kind` enabled.
    #[must_use]
    pub const fn with(self, kind: TraceKind) -> Self {
        Self(self.0 | kind.bit())
    }

    /// Returns a mask with `kind` disabled.
    #[must_use]
    pub const fn without(self, kind: TraceKind) -> Self {
        Self(self.0 & !kind.bit())
    }
}

impl Default for TraceKindMask {
    fn default() -> Self {
        Self::all()
    }
}

/// The phase of one observed interrupt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InterruptPhase {
    /// A scheduler source raised a request for a level.
    Requested,
    /// The CPU entered the handler for a level.
    Delivered,
    /// The scheduler acknowledged exactly the delivered level.
    Acknowledged,
}

impl InterruptPhase {
    /// Returns the stable binary phase code.
    #[must_use]
    pub const fn code(self) -> u8 {
        match self {
            Self::Requested => 0,
            Self::Delivered => 1,
            Self::Acknowledged => 2,
        }
    }

    /// Returns the stable lowercase phase name used by summaries and TSV export.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Requested => "requested",
            Self::Delivered => "delivered",
            Self::Acknowledged => "acknowledged",
        }
    }
}

/// One retained trace record with an absolute index and emulated tick.
///
/// Indexes are contiguous from [`crate::TraceMeta::first_index`] and ticks are
/// non-decreasing; both invariants are enforced by [`crate::Trace::new`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TraceRecord {
    /// One instruction observed before it executed.
    Instruction {
        /// Absolute record index in the capture stream.
        index: u64,
        /// Emulated tick before the instruction executed.
        tick: Tick,
        /// Address of the instruction about to execute.
        pc: u32,
        /// Big-endian 16-bit opcode word at `pc`.
        opcode: u16,
    },
    /// One emulated work-RAM byte write.
    BusWrite {
        /// Absolute record index in the capture stream.
        index: u64,
        /// Emulated tick before the step that produced the write.
        tick: Tick,
        /// Written bus address.
        address: u32,
        /// Written value.
        value: u32,
        /// Access width in bytes, `1..=4`.
        width: u8,
    },
    /// One VDP port access.
    VdpPort {
        /// Absolute record index in the capture stream.
        index: u64,
        /// Emulated tick before the step that produced the access.
        tick: Tick,
        /// Port offset relative to the VDP base address.
        offset: u8,
        /// Accessed value.
        value: u32,
        /// Access width in bytes, `1..=2`.
        width: u8,
        /// `true` for a write, `false` for a read.
        write: bool,
    },
    /// One controller data-register read.
    ControllerRead {
        /// Absolute record index in the capture stream.
        index: u64,
        /// Emulated tick before the step that produced the read.
        tick: Tick,
        /// Controller port number, `1..=2`.
        port: u8,
        /// Data byte returned by the read.
        value: u8,
    },
    /// One interrupt phase.
    Interrupt {
        /// Absolute record index in the capture stream.
        index: u64,
        /// Emulated tick before the step that produced the phase.
        tick: Tick,
        /// 68000 interrupt level, `1..=7`.
        level: u8,
        /// Phase of the observed interrupt.
        phase: InterruptPhase,
    },
}

impl TraceRecord {
    /// Returns the absolute record index.
    #[must_use]
    pub const fn index(&self) -> u64 {
        match self {
            Self::Instruction { index, .. }
            | Self::BusWrite { index, .. }
            | Self::VdpPort { index, .. }
            | Self::ControllerRead { index, .. }
            | Self::Interrupt { index, .. } => *index,
        }
    }

    /// Returns the emulated tick the record belongs to.
    #[must_use]
    pub const fn tick(&self) -> Tick {
        match self {
            Self::Instruction { tick, .. }
            | Self::BusWrite { tick, .. }
            | Self::VdpPort { tick, .. }
            | Self::ControllerRead { tick, .. }
            | Self::Interrupt { tick, .. } => *tick,
        }
    }

    /// Returns the record kind.
    #[must_use]
    pub const fn kind(&self) -> TraceKind {
        match self {
            Self::Instruction { .. } => TraceKind::Instruction,
            Self::BusWrite { .. } => TraceKind::BusWrite,
            Self::VdpPort { .. } => TraceKind::VdpPort,
            Self::ControllerRead { .. } => TraceKind::ControllerRead,
            Self::Interrupt { .. } => TraceKind::Interrupt,
        }
    }

    /// Returns the deterministic one-line summary used by CLI query/compare.
    ///
    /// The form is `<kind> index=<n> tick=<n> <fields>` where `<fields>`
    /// appears in the record's declared order and absent fields never appear:
    ///
    /// - `instruction ... pc=<6 hex> opcode=<4 hex>`
    /// - `bus_write ... address=<8 hex> value=<8 hex> width=<n>`
    /// - `vdp_port ... offset=<2 hex> value=<8 hex> width=<n> write=<0|1>`
    /// - `controller_read ... port=<n> value=<2 hex>`
    /// - `interrupt ... level=<n> phase=<requested|delivered|acknowledged>`
    ///
    /// This is not the TSV row form (see [`crate::Trace::to_tsv`]).
    #[must_use]
    pub fn summary(&self) -> String {
        match self {
            Self::Instruction {
                index,
                tick,
                pc,
                opcode,
            } => format!(
                "instruction index={index} tick={} pc={pc:06X} opcode={opcode:04X}",
                tick.raw()
            ),
            Self::BusWrite {
                index,
                tick,
                address,
                value,
                width,
            } => format!(
                "bus_write index={index} tick={} address={address:08X} value={value:08X} \
                 width={width}",
                tick.raw()
            ),
            Self::VdpPort {
                index,
                tick,
                offset,
                value,
                width,
                write,
            } => format!(
                "vdp_port index={index} tick={} offset={offset:02X} value={value:08X} \
                 width={width} write={}",
                tick.raw(),
                u8::from(*write)
            ),
            Self::ControllerRead {
                index,
                tick,
                port,
                value,
            } => format!(
                "controller_read index={index} tick={} port={port} value={value:02X}",
                tick.raw()
            ),
            Self::Interrupt {
                index,
                tick,
                level,
                phase,
            } => format!(
                "interrupt index={index} tick={} level={level} phase={}",
                tick.raw(),
                phase.name()
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use chronofork_core::Tick;

    use super::{InterruptPhase, TraceKind, TraceKindMask, TraceRecord};
    use crate::error::TraceError;

    fn records() -> Vec<TraceRecord> {
        vec![
            TraceRecord::Instruction {
                index: 3,
                tick: Tick::from_raw(3),
                pc: 0x000200,
                opcode: 0x7001,
            },
            TraceRecord::BusWrite {
                index: 4,
                tick: Tick::from_raw(3),
                address: 0x00FF_0000,
                value: 0x2A,
                width: 1,
            },
            TraceRecord::VdpPort {
                index: 5,
                tick: Tick::from_raw(4),
                offset: 4,
                value: 0x8F02,
                width: 2,
                write: true,
            },
            TraceRecord::ControllerRead {
                index: 6,
                tick: Tick::from_raw(5),
                port: 1,
                value: 0x7F,
            },
            TraceRecord::Interrupt {
                index: 7,
                tick: Tick::from_raw(5),
                level: 4,
                phase: InterruptPhase::Requested,
            },
        ]
    }

    #[test]
    fn kind_codes_bits_and_names_are_stable() {
        for (position, kind) in TraceKind::ALL.iter().enumerate() {
            assert_eq!(usize::from(kind.code()), position);
            assert_eq!(kind.bit(), 1 << position);
            assert!(TraceKindMask::all().contains(*kind));
            assert!(!TraceKindMask::none().contains(*kind));
        }
        let expected = [
            "instruction",
            "bus_write",
            "vdp_port",
            "controller_read",
            "interrupt",
        ];
        for (kind, name) in TraceKind::ALL.iter().zip(expected) {
            assert_eq!(kind.name(), name);
        }
        assert_eq!(InterruptPhase::Requested.code(), 0);
        assert_eq!(InterruptPhase::Delivered.code(), 1);
        assert_eq!(InterruptPhase::Acknowledged.code(), 2);
        assert_eq!(InterruptPhase::Requested.name(), "requested");
        assert_eq!(InterruptPhase::Delivered.name(), "delivered");
        assert_eq!(InterruptPhase::Acknowledged.name(), "acknowledged");
    }

    #[test]
    fn mask_rejects_undefined_bits_and_supports_composition() {
        assert_eq!(TraceKindMask::default(), TraceKindMask::all());
        assert_eq!(TraceKindMask::all().bits(), 0b0001_1111);
        assert_eq!(TraceKindMask::none().bits(), 0);
        assert!(TraceKindMask::none().is_empty());
        assert!(!TraceKindMask::all().is_empty());

        assert_eq!(
            TraceKindMask::from_bits(0b0010_0000),
            Err(TraceError::InvalidConfiguration(
                "trace kind mask 0b00100000 sets undefined bits above bit 4".to_owned()
            ))
        );
        for bits in 0..=0b0001_1111 {
            assert_eq!(TraceKindMask::from_bits(bits), Ok(TraceKindMask(bits)));
        }

        let bus = TraceKindMask::none()
            .with(TraceKind::BusWrite)
            .with(TraceKind::ControllerRead);
        assert!(bus.contains(TraceKind::BusWrite));
        assert!(bus.contains(TraceKind::ControllerRead));
        assert!(!bus.contains(TraceKind::Instruction));
        assert_eq!(bus.without(TraceKind::BusWrite).bits(), 1 << 3);
        assert_eq!(bus.with(TraceKind::Instruction).bits(), 0b0000_1011);
    }

    #[test]
    fn accessors_and_summaries_match_the_documented_form() {
        let expected = [
            "instruction index=3 tick=3 pc=000200 opcode=7001",
            "bus_write index=4 tick=3 address=00FF0000 value=0000002A width=1",
            "vdp_port index=5 tick=4 offset=04 value=00008F02 width=2 write=1",
            "controller_read index=6 tick=5 port=1 value=7F",
            "interrupt index=7 tick=5 level=4 phase=requested",
        ];
        let records = records();
        for (record, summary) in records.iter().zip(expected) {
            assert_eq!(record.summary(), summary);
            assert_eq!(
                record.kind().name(),
                summary.split(' ').next().unwrap_or_default()
            );
        }
        assert_eq!(records[0].index(), 3);
        assert_eq!(records[0].tick(), Tick::from_raw(3));
        assert_eq!(records[1].index(), 4);
        assert_eq!(records[1].tick(), Tick::from_raw(3));
        assert_eq!(records[2].tick(), Tick::from_raw(4));
    }
}
