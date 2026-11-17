//! Deterministic, one-way TSV export.

use std::fmt::Write as _;

use crate::record::{InterruptPhase, TraceRecord};
use crate::trace::Trace;

impl Trace {
    /// Renders the trace as a deterministic, tab-separated, one-way export.
    ///
    /// The output begins with the documented comment header, then the fixed
    /// table header `index`, `kind`, `tick`, `pc`, `address`, `value`, `width`,
    /// `extra` separated by tabs, and one row per retained record. Instruction
    /// `pc` is six uppercase hex
    /// digits, `address` and `value` are eight uppercase hex digits, `width` is
    /// decimal and absent fields are `-`. There is no TSV import.
    #[must_use]
    pub fn to_tsv(&self) -> String {
        let meta = self.meta();
        let mut output = String::new();
        let _ = writeln!(output, "# chronofork trace v1");
        let _ = writeln!(output, "# backend={}", meta.backend);
        let _ = writeln!(output, "# backend_version={}", meta.backend_version);
        let _ = writeln!(output, "# state_hash={}", meta.state_hash);
        let _ = writeln!(output, "# start_tick={}", meta.start_tick);
        let _ = writeln!(output, "# end_tick={}", meta.end_tick);
        let _ = writeln!(output, "# capacity={}", meta.capacity);
        let _ = writeln!(output, "# records={}", self.record_count());
        let _ = writeln!(output, "# first_index={}", meta.first_index);
        let _ = writeln!(output, "# dropped={}", meta.dropped);
        let _ = writeln!(output, "# stop_reason={}", meta.stop_reason.name());
        let _ = writeln!(output, "# triggered={}", u8::from(meta.triggered));
        let _ = writeln!(
            output,
            "# events_observed={}",
            u8::from(meta.events_observed)
        );
        output.push_str("index\tkind\ttick\tpc\taddress\tvalue\twidth\textra\n");
        for record in self.records() {
            write_row(&mut output, record);
        }
        output
    }
}

fn write_row(output: &mut String, record: &TraceRecord) {
    match record {
        TraceRecord::Instruction {
            index,
            tick,
            pc,
            opcode,
        } => {
            let _ = writeln!(
                output,
                "{index}\tinstruction\t{}\t{pc:06X}\t-\t{opcode:08X}\t-\t-",
                tick.raw()
            );
        }
        TraceRecord::BusWrite {
            index,
            tick,
            address,
            value,
            width,
        } => {
            let _ = writeln!(
                output,
                "{index}\tbus_write\t{}\t-\t{address:08X}\t{value:08X}\t{width}\t-",
                tick.raw()
            );
        }
        TraceRecord::VdpPort {
            index,
            tick,
            offset,
            value,
            width,
            write,
        } => {
            let direction = if *write { "write" } else { "read" };
            let _ = writeln!(
                output,
                "{index}\tvdp_port\t{}\t-\t{offset:08X}\t{value:08X}\t{width}\t{direction}",
                tick.raw()
            );
        }
        TraceRecord::ControllerRead {
            index,
            tick,
            port,
            value,
        } => {
            let _ = writeln!(
                output,
                "{index}\tcontroller_read\t{}\t-\t{port:08X}\t{value:08X}\t-\t-",
                tick.raw()
            );
        }
        TraceRecord::Interrupt {
            index,
            tick,
            level,
            phase,
        } => {
            let phase = match phase {
                InterruptPhase::Requested => "requested",
                InterruptPhase::Delivered => "delivered",
                InterruptPhase::Acknowledged => "acknowledged",
            };
            let _ = writeln!(
                output,
                "{index}\tinterrupt\t{}\t-\t-\t{level:08X}\t-\t{phase}",
                tick.raw()
            );
        }
    }
}
