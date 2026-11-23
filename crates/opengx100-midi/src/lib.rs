//! MIDI I/O through `midir` (CoreMIDI on macOS, WinMM on Windows, ALSA on Linux).

use midir::{Ignore, MidiInput, MidiInputConnection, MidiOutput, MidiOutputConnection};
use opengx100_protocol::session::Transport;
use opengx100_protocol::sysex::SysexAssembler;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::Duration;

const CLIENT: &str = "opengx100";

pub struct MidiTransport {
    _input: MidiInputConnection<()>,
    output: MidiOutputConnection,
    rx: Receiver<Vec<u8>>,
    pub in_name: String,
    pub out_name: String,
}

/// (input port names, output port names)
pub fn port_names() -> Result<(Vec<String>, Vec<String>), String> {
    let mi = MidiInput::new(CLIENT).map_err(|e| e.to_string())?;
    let mo = MidiOutput::new(CLIENT).map_err(|e| e.to_string())?;
    let ins = mi.ports().iter().filter_map(|p| mi.port_name(p).ok()).collect();
    let outs = mo.ports().iter().filter_map(|p| mo.port_name(p).ok()).collect();
    Ok((ins, outs))
}

/// The pedal the installed tables describe (GX-100, GX-10, …), whose name the ports carry.
fn default_filter() -> &'static str {
    opengx100_protocol::tables::PRODUCT.name
}

/// Whether `name` carries `want` as a whole word: "GX-10" must not match a "GX-100" port.
fn mentions(name: &str, want: &str) -> bool {
    let n = name.to_lowercase();
    n.match_indices(want).any(|(i, _)| !n[i + want.len()..].chars().next().is_some_and(|c| c.is_alphanumeric()))
}

/// Picks the port whose name contains `filter` (default: the pedal's name),
/// preferring one that isn't a DAW-control port.
fn pick(names: &[String], filter: Option<&str>) -> Option<usize> {
    let want = filter.unwrap_or(default_filter()).to_lowercase();
    let hits: Vec<usize> = (0..names.len()).filter(|&i| mentions(&names[i], &want)).collect();
    hits.iter()
        .copied()
        .find(|&i| {
            let n = names[i].to_uppercase();
            !n.contains("DAW") && !n.contains("CTRL")
        })
        .or(hits.first().copied())
}

fn not_found(kind: &str, filter: Option<&str>, names: &[String]) -> String {
    format!(
        "no MIDI {kind} port matching {:?}. Available: {}. Is the {} connected by USB and its driver installed? \
         Use --port <part of the name> to choose one. Which pedal this is comes from the installed BOSS TONE STUDIO, \
         so with both installed, point --bts at the right one.",
        filter.unwrap_or(default_filter()),
        if names.is_empty() { "none".to_string() } else { names.join(", ") },
        default_filter()
    )
}

/// Watches whether MIDI ports are present, without connecting to them. Keep one for the program's
/// life and create it on the main thread: on macOS, CoreMIDI updates a client's port list through the
/// run loop of the thread that created it.
pub struct PortWatch {
    input: MidiInput,
    output: MidiOutput,
}

impl PortWatch {
    pub fn new() -> Result<Self, String> {
        let input = MidiInput::new(&format!("{CLIENT}-watch")).map_err(|e| e.to_string())?;
        let output = MidiOutput::new(&format!("{CLIENT}-watch")).map_err(|e| e.to_string())?;
        Ok(PortWatch { input, output })
    }

    /// Whether an input port named `in_name` and an output port named `out_name` both exist.
    pub fn present(&self, in_name: &str, out_name: &str) -> bool {
        let has_in = self.input.ports().iter().any(|p| self.input.port_name(p).is_ok_and(|n| n == in_name));
        has_in && self.output.ports().iter().any(|p| self.output.port_name(p).is_ok_and(|n| n == out_name))
    }
}

pub fn open(filter: Option<&str>) -> Result<MidiTransport, String> {
    open_with_waker(filter, || {})
}

/// Like [`open`], but calls `wake` (on the MIDI thread) after each complete incoming message,
/// so a caller can sleep until the pedal sends something instead of polling.
pub fn open_with_waker(filter: Option<&str>, wake: impl Fn() + Send + 'static) -> Result<MidiTransport, String> {
    let mut mi = MidiInput::new(CLIENT).map_err(|e| e.to_string())?;
    // Keep SysEx; drop clock and active sensing.
    mi.ignore(Ignore::TimeAndActiveSense);
    let mo = MidiOutput::new(CLIENT).map_err(|e| e.to_string())?;

    let in_ports = mi.ports();
    let out_ports = mo.ports();
    let in_names: Vec<String> = in_ports.iter().map(|p| mi.port_name(p).unwrap_or_default()).collect();
    let out_names: Vec<String> = out_ports.iter().map(|p| mo.port_name(p).unwrap_or_default()).collect();
    let i = pick(&in_names, filter).ok_or_else(|| not_found("input", filter, &in_names))?;
    let o = pick(&out_names, filter).ok_or_else(|| not_found("output", filter, &out_names))?;

    let (tx, rx) = mpsc::channel();
    let mut asm = SysexAssembler::new();
    let input = mi
        .connect(
            &in_ports[i],
            "gx100-in",
            move |_stamp, bytes, _| {
                for m in asm.push(bytes) {
                    let _ = tx.send(m);
                    wake();
                }
            },
            (),
        )
        .map_err(|e| e.to_string())?;
    let output = mo.connect(&out_ports[o], "gx100-out").map_err(|e| e.to_string())?;

    Ok(MidiTransport { _input: input, output, rx, in_name: in_names[i].clone(), out_name: out_names[o].clone() })
}

impl Transport for MidiTransport {
    fn send(&mut self, msg: &[u8]) -> Result<(), String> {
        self.output.send(msg).map_err(|e| e.to_string())
    }

    fn recv(&mut self, timeout: Duration) -> Result<Option<Vec<u8>>, String> {
        match self.rx.recv_timeout(timeout) {
            Ok(m) => Ok(Some(m)),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => Err("MIDI input closed".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ports(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    /// The GX-10's port filter must not settle on a GX-100: the pedals can both be connected, and the
    /// names differ by one digit.
    #[test]
    fn the_filter_is_a_whole_word() {
        let both = ports(&["GX-100", "GX-10"]);
        assert_eq!(pick(&both, Some("GX-10")).map(|i| both[i].as_str()), Some("GX-10"));
        assert_eq!(pick(&both, Some("GX-100")).map(|i| both[i].as_str()), Some("GX-100"));

        let only_100 = ports(&["GX-100 1"]);
        assert_eq!(pick(&only_100, Some("GX-10")), None);

        let with_suffix = ports(&["GX-10 MIDI 1"]);
        assert_eq!(pick(&with_suffix, Some("GX-10")), Some(0));
    }

    #[test]
    fn a_daw_port_loses_to_the_pedal() {
        let names = ports(&["GX-100 DAW CTRL", "GX-100"]);
        assert_eq!(pick(&names, Some("GX-100")), Some(1));
    }
}
