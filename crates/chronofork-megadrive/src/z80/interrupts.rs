//! Z80 maskable and non-maskable interrupt acceptance.
//!
//! Requests are latched by [`Z80::request_interrupt`] and [`Z80::request_nmi`].
//! [`Z80::step`] checks them at an instruction boundary and executes the
//! acknowledge sequence instead of an instruction, so `HALT` is woken and the
//! documented T-state costs are charged to the canonical timeline:
//!
//! | source | accepted when | vector | T-states |
//! |---|---|---|---|
//! | NMI | always (not maskable) | `0x0066` | 11 |
//! | IM 0 | `IFF1` set, `EI` delay elapsed | device-supplied byte, executed | 13 for `RST` |
//! | IM 1 | `IFF1` set, `EI` delay elapsed | `0x0038` | 13 |
//! | IM 2 | `IFF1` set, `EI` delay elapsed | table word at `I << 8 \| vector` | 19 |
//!
//! NMI clears `IFF1` and leaves `IFF2` untouched, so `RETN`/`RETI` restore the
//! previous mask. An accepted maskable interrupt leaves `IFF1`/`IFF2` unchanged
//! (the handler's `DI` normally clears them). `HALT` is left and the pushed
//! return address is the byte after the `HALT`.
//!
//! **Limits.** The Mega Drive wires the Z80 interrupts in IM 1 in practice; the
//! other modes are modelled from documented Z80 behaviour. IM 0 executes the
//! device-supplied byte through the main table, so `RST` and `CALL nn` work
//! while a multi-byte supplied instruction reads its immediates from the
//! interrupted program's memory. The interrupt acknowledge is modelled without a
//! bus cycle of its own; it increments `R` once as the M1 refresh does.

use super::{Z80, Z80Bus, Z80Error, ops};

/// IM 1 vector address (`RST 38h`).
pub const IM1_VECTOR_ADDRESS: u16 = 0x0038;
/// NMI vector address.
pub const NMI_VECTOR_ADDRESS: u16 = 0x0066;
/// T-states charged for accepting an IM 1 interrupt.
pub const IM1_ACKNOWLEDGE_T_STATES: u32 = 13;
/// T-states charged for accepting an IM 2 interrupt (acknowledge plus vector read).
pub const IM2_ACKNOWLEDGE_T_STATES: u32 = 19;
/// T-states charged for accepting an NMI.
pub const NMI_ACKNOWLEDGE_T_STATES: u32 = 11;
/// Extra T-states an IM 0 acknowledge adds to the supplied instruction's cost.
const IM0_ACKNOWLEDGE_T_STATES: u32 = 2;

impl Z80 {
    /// Latches a maskable interrupt request with its device-supplied vector byte.
    ///
    /// Multiple requests coalesce into one latch; the last vector wins. The
    /// request stays latched while `IFF1` is clear or the `EI` delay is pending,
    /// and [`Z80::step`] accepts it at the first permitted instruction boundary.
    pub fn request_interrupt(&mut self, vector: u8) {
        self.interrupt_pending = true;
        self.interrupt_vector = vector;
    }

    /// Latches a non-maskable interrupt request.
    ///
    /// NMI is accepted at the next [`Z80::step`] boundary regardless of `IFF1`
    /// and the `EI` delay, and wakes a halted core.
    pub fn request_nmi(&mut self) {
        self.nmi_pending = true;
    }

    /// Takes a pending maskable interrupt when it may be accepted.
    ///
    /// Returns `true` and clears the request latch when a request is pending,
    /// `IFF1` is set and the `EI` one-instruction delay has elapsed. A masked or
    /// delayed request stays latched and returns `false`. NMI has its own latch
    /// and is serviced first by [`Z80::step`].
    pub fn take_interrupt_pending(&mut self) -> bool {
        if self.interrupt_pending && self.iff1 && !self.ei_delay {
            self.interrupt_pending = false;
            true
        } else {
            false
        }
    }
}

/// Accepts one pending interrupt at an instruction boundary, if permitted.
///
/// Returns the acknowledge sequence's T-state cost, or `None` when nothing is
/// pending or the maskable request is blocked by `IFF1` or the `EI` delay.
pub(crate) fn accept(z80: &mut Z80, bus: &mut impl Z80Bus) -> Result<Option<u32>, Z80Error> {
    if z80.nmi_pending {
        z80.nmi_pending = false;
        z80.halted = false;
        z80.push16(bus, z80.pc)?;
        z80.pc = NMI_VECTOR_ADDRESS;
        // NMI saves the mask in IFF2 and disables maskable interrupts.
        z80.iff1 = false;
        z80.increment_r();
        return Ok(Some(NMI_ACKNOWLEDGE_T_STATES));
    }
    if !z80.take_interrupt_pending() {
        return Ok(None);
    }
    z80.halted = false;
    z80.increment_r();
    let t_states = match z80.im {
        0 => {
            // The interrupting device supplies one instruction byte; `RST` and
            // `CALL nn` are the forms real hardware uses.
            let opcode = z80.interrupt_vector;
            IM0_ACKNOWLEDGE_T_STATES + ops::execute(z80, bus, opcode)?
        }
        2 => {
            z80.push16(bus, z80.pc)?;
            let address = (u16::from(z80.i) << 8) | u16::from(z80.interrupt_vector);
            let low = bus.read8(address)?;
            let high = bus.read8(address.wrapping_add(1))?;
            z80.pc = u16::from_le_bytes([low, high]);
            IM2_ACKNOWLEDGE_T_STATES
        }
        _ => {
            z80.push16(bus, z80.pc)?;
            z80.pc = IM1_VECTOR_ADDRESS;
            IM1_ACKNOWLEDGE_T_STATES
        }
    };
    Ok(Some(t_states))
}
