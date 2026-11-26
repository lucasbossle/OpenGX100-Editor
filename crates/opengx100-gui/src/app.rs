//! The editor window: patch list on the left; in the middle, the selected patch's chain and
//! parameters, the setlists ([`setlists`]) or the backups ([`backups`]).

mod assigns;
mod auto_write;
mod backups;
mod chain_edit;
mod controls;
mod editing;
mod eq_graph;
mod presets;
mod setlists;
mod tuner;

use crate::store::{Backup, BackupInfo, Numbering, Setlist, Settings, Store};
use crate::worker::{self, Cmd, Event, PatchId};
use editing::Dialog;
use eframe::egui::{self, pos2, vec2, Color32, Pos2, Rect, RichText, Sense, Stroke};
use opengx100_protocol::backup::AreaData;
use opengx100_protocol::chain::Limits;
use opengx100_protocol::patch::{self, ChainLayout, PatchData, FX_SLOTS, USER_PATCHES};
use opengx100_protocol::tables::{self, FxField, FxParamDef, FxTypeDef};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Patch-level settings shown above the chain, as (parameter path, label).
const PATCH_SETTINGS: &[(&str, &str)] = &[
    ("Temp_patch.efct.MasterPatchLevel", "Level"),
    ("Temp_patch.efct.MasterBpm", "BPM"),
    ("Temp_patch.efct.MasterKey", "Key"),
    ("Temp_patch.efct.MasterInputSens", "Input sens"),
    ("Temp_patch.efct.MasterTempoHold", "Tempo hold"),
    ("Temp_patch.efct.MasterCarryOver", "Carryover"),
];

const KEYS: &[&str] = &["C", "Db", "D", "Eb", "E", "F", "F#", "G", "Ab", "A", "Bb", "B"];

/// FX types that route the signal rather than process it.
const ROUTING_TYPES: &[&str] = &["DIVIDER", "SPLITTER", "MIXER"];

/// How long a value changed on the pedal stays highlighted.
const FLASH: Duration = Duration::from_millis(800);

const AMBER: Color32 = Color32::from_rgb(240, 190, 80);
const GREEN: Color32 = Color32::from_rgb(90, 200, 120);
const RED: Color32 = Color32::from_rgb(230, 90, 80);
/// Wires the signal flows through.
const SIGNAL: Color32 = Color32::from_rgb(110, 175, 235);

// Chain diagram geometry.
const BLOCK_W: f32 = 112.0;
const ROUTING_W: f32 = 76.0;
const BLOCK_H: f32 = 46.0;
const GAP: f32 = 30.0;
const ROW_GAP: f32 = 16.0;
const TERMINAL_W: f32 = 34.0;
/// Space between the lines of a folded chain.
const LINE_GAP: f32 = 30.0;
/// Room beside the lines for the wire that turns down to the next one.
const TURN: f32 = 20.0;

/// Scrolling by scroll bar and wheel, but not by dragging content (dragging moves rows instead).
const SCROLL_NOT_DRAG: egui::scroll_area::ScrollSource = egui::scroll_area::ScrollSource {
    drag: egui::scroll_area::DragScroll::Never,
    ..egui::scroll_area::ScrollSource::ALL
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum View {
    /// One patch: chain and parameters.
    Patch,
    /// Saved lists of patches in a new order, and what writing them into a range would do.
    Setlists,
    /// Full backups of the pedal.
    Backups,
    /// What the footswitches, CTLs and expression pedals do.
    CtlExp,
    /// Input/output settings and global EQ.
    InOut,
    /// System settings.
    System,
    /// The pedal's tuner (shown on the pedal while this is open).
    Tuner,
}

/// The two pages of the CTL/EXP tab, as in the official editor.
#[derive(Clone, Copy, PartialEq, Eq)]
enum CtlExpPage {
    /// What each footswitch, CTL and expression pedal does.
    Functions,
    /// The live patch's 20 assigns.
    Assigns,
}

enum Conn {
    Connecting,
    Connected(String),
    Failed(String),
    /// The pedal's MIDI ports went away (unplugged, switched off); reconnects once they're back.
    Lost,
}

/// How often to look whether the pedal's MIDI ports are still (or again) there.
const PORT_CHECK: Duration = Duration::from_secs(1);

pub struct EditorApp {
    tx: Sender<Cmd>,
    rx: Receiver<Event>,
    worker: Option<JoinHandle<()>>,
    conn: Conn,
    /// Sees the MIDI ports come and go (`None` if CoreMIDI refused a client).
    port_watch: Option<opengx100_midi::PortWatch>,
    /// The (input, output) ports of the pedal we're (or were last) connected to; `None` for the simulator.
    ports: Option<(String, String)>,
    last_port_check: Instant,
    /// The current connection attempt was started by the port watch: if it fails, keep waiting.
    auto_reconnect: bool,
    /// The pedal pushes changes (editor mode is on).
    live: bool,
    current: Option<u16>,
    names: Vec<Option<String>>,
    patches: HashMap<PatchId, PatchData>,
    /// Patches requested but not yet received.
    loading: Vec<PatchId>,
    /// Live-patch byte ranges the pedal changed recently, for highlighting.
    recent: Vec<(u32, u32, Instant)>,
    /// The system settings, as read at connect and kept current.
    system: Vec<AreaData>,
    selected_slot: Option<u8>,
    show_unused: bool,
    /// What the pedal says still fits into the live chain (read again with every live-patch read).
    chain_limits: Option<Limits>,
    /// A chain edit is on its way to the pedal.
    chain_busy: bool,
    /// The chain block being dragged (its slot).
    chain_drag: Option<u8>,
    /// The FX type list, open to add or replace an effect.
    picker: Option<chain_edit::Picker>,
    tuner: tuner::TunerState,
    filter: String,
    last_error: Option<String>,
    /// Good news to show once (e.g. where a backup was saved).
    notice: Option<String>,
    view: View,
    ctl_exp_page: CtlExpPage,
    /// The assign being edited (1-based).
    selected_assign: u8,
    /// Where setlists and backups live; `None` if the data folder can't be used.
    store: Option<Store>,
    /// The app's preferences (auto-write), saved with the store.
    settings: Settings,
    /// When the current auto-write period started (`None` while auto-write is off).
    auto_write_from: Option<Instant>,
    /// The patch an auto-write is on its way to, to say so when it's done.
    auto_writing: Option<u16>,
    setlists: Vec<Setlist>,
    /// Index in `setlists` of the one being edited.
    active: Option<usize>,
    /// When the active setlist was last changed without being saved yet.
    unsaved_since: Option<Instant>,
    confirm_delete_setlist: bool,
    /// Index of the setlist row being dragged.
    dragging: Option<usize>,
    backups: Vec<BackupInfo>,
    /// (done, total) while a backup runs.
    backup_progress: Option<(usize, usize)>,
    open_backup: Option<(PathBuf, Backup)>,
    confirm_delete_backup: Option<PathBuf>,
    dialog: Option<Dialog>,
    /// The FX presets menu's list, read from disk when it opens.
    preset_cache: presets::PresetCache,
    /// (done, total) while a setlist is being written.
    setlist_write: Option<(usize, usize)>,
    /// The current patch and its new data, when a setlist write rewrites it.
    setlist_rewrites_current: Option<(u16, PatchData)>,
}

impl EditorApp {
    pub fn new(egui_ctx: &egui::Context, sim: bool) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (ev_tx, ev_rx) = mpsc::channel();
        let ctx = egui_ctx.clone();
        let wake = cmd_tx.clone();
        let worker = std::thread::Builder::new()
            .name("gx100-worker".into())
            .spawn(move || worker::run(cmd_rx, wake, ev_tx, move || ctx.request_repaint()))
            .expect("spawning the MIDI worker thread");
        let mut app = EditorApp {
            tx: cmd_tx,
            rx: ev_rx,
            worker: Some(worker),
            conn: Conn::Connecting,
            port_watch: opengx100_midi::PortWatch::new().ok(),
            ports: None,
            last_port_check: Instant::now(),
            auto_reconnect: false,
            live: false,
            current: None,
            names: vec![None; USER_PATCHES as usize],
            patches: HashMap::new(),
            loading: Vec::new(),
            recent: Vec::new(),
            system: Vec::new(),
            selected_slot: None,
            show_unused: false,
            chain_limits: None,
            chain_busy: false,
            chain_drag: None,
            picker: None,
            tuner: Default::default(),
            filter: String::new(),
            last_error: None,
            notice: None,
            view: View::Patch,
            ctl_exp_page: CtlExpPage::Functions,
            selected_assign: 1,
            store: None,
            settings: Settings::default(),
            auto_write_from: None,
            auto_writing: None,
            setlists: Vec::new(),
            active: None,
            unsaved_since: None,
            confirm_delete_setlist: false,
            dragging: None,
            backups: Vec::new(),
            backup_progress: None,
            open_backup: None,
            confirm_delete_backup: None,
            dialog: None,
            preset_cache: Default::default(),
            setlist_write: None,
            setlist_rewrites_current: None,
        };
        app.open_store();
        app.connect(sim);
        app
    }

    /// Loads the saved setlists (opening the most recently changed one) and the backup list.
    fn open_store(&mut self) {
        match Store::open_default() {
            Ok(store) => {
                let (lists, errors) = store.load_setlists();
                if !errors.is_empty() {
                    self.last_error = Some(format!("couldn't read some setlists: {}", errors.join("; ")));
                }
                self.setlists = lists;
                self.active = (!self.setlists.is_empty()).then_some(0);
                self.backups = store.list_backups();
                self.settings = store.load_settings();
                self.store = Some(store);
            }
            Err(e) => {
                self.last_error = Some(format!("can't use the data folder, setlists and backups won't be saved: {e}"))
            }
        }
    }

    fn connect(&mut self, sim: bool) {
        self.forget_pedal();
        self.conn = Conn::Connecting;
        self.auto_reconnect = false;
        self.last_error = None;
        let _ = self.tx.send(Cmd::Connect { sim });
    }

    /// Drops everything read from the pedal and whatever was on its way to it.
    fn forget_pedal(&mut self) {
        self.live = false;
        self.current = None;
        self.names.iter_mut().for_each(|n| *n = None);
        self.patches.clear();
        self.loading.clear();
        self.recent.clear();
        self.system.clear();
        self.chain_limits = None;
        self.chain_busy = false;
        self.chain_drag = None;
        self.picker = None;
        self.tuner = Default::default();
        self.dialog = None;
        self.setlist_write = None;
    }

    /// Notices the pedal going away (USB unplugged, switched off) and coming back. Without this the app
    /// would keep talking to ports that are gone, and a pedal switched on again would be out of editor mode.
    fn watch_ports(&mut self, ctx: &egui::Context) {
        let Some((input, output)) = self.ports.clone() else { return };
        if !matches!(self.conn, Conn::Connected(_) | Conn::Lost) {
            return;
        }
        ctx.request_repaint_after(PORT_CHECK);
        if self.last_port_check.elapsed() < PORT_CHECK {
            return;
        }
        self.last_port_check = Instant::now();
        let Some(watch) = &self.port_watch else { return };
        let present = watch.present(&input, &output);
        match self.conn {
            Conn::Connected(_) if !present => {
                let _ = self.tx.send(Cmd::Disconnected);
                self.forget_pedal();
                self.conn = Conn::Lost;
                self.last_error = None;
            }
            Conn::Lost if present => {
                self.connect(false);
                self.auto_reconnect = true;
            }
            _ => {}
        }
    }

    /// Reads a patch unless it's cached or on its way. Stored patches don't change behind our back;
    /// the live one is kept current by the pedal, but is re-read anyway, in case live updates are off.
    fn request(&mut self, id: PatchId) {
        if (id == PatchId::Temp || !self.patches.contains_key(&id)) && !self.loading.contains(&id) {
            self.loading.push(id);
            let _ = self.tx.send(Cmd::LoadPatch(id));
        }
    }

    fn reload(&mut self) {
        self.patches.clear();
        self.names.iter_mut().for_each(|n| *n = None);
        let _ = self.tx.send(Cmd::LoadNames);
        let _ = self.tx.send(Cmd::ReadSystem);
        self.request(PatchId::Temp);
        if let Some(n) = self.current {
            self.request(PatchId::User(n));
        }
        self.request_setlist_sources();
    }

    fn handle_events(&mut self) {
        while let Ok(ev) = self.rx.try_recv() {
            match ev {
                Event::Connected { desc, current, live, ports } => {
                    self.conn = Conn::Connected(desc);
                    self.ports = ports;
                    self.auto_reconnect = false;
                    self.last_port_check = Instant::now();
                    self.current = current;
                    self.live = live;
                    self.backup_progress = None;
                    self.request(PatchId::Temp);
                    if let Some(n) = current {
                        self.request(PatchId::User(n)); // the stored copy, to spot unsaved changes
                    }
                    self.request_setlist_sources();
                }
                Event::Name(n, name) => {
                    if let Some(slot) = self.names.get_mut(n as usize - 1) {
                        *slot = Some(name);
                    }
                }
                Event::Patch(id, data) => {
                    self.loading.retain(|&l| l != id);
                    if let PatchId::User(n) = id {
                        self.names[n as usize - 1] = Some(data.name());
                    }
                    if id == PatchId::Temp && self.patches.get(&id).is_some_and(|old| old.chain() != data.chain()) {
                        self.selected_slot = None;
                    }
                    if let PatchId::User(n) = id {
                        self.fill_setlist_entries(n, &data);
                    }
                    if id == PatchId::Temp && !self.chain_busy {
                        let _ = self.tx.send(Cmd::ReadChainLimits);
                    }
                    self.patches.insert(id, data);
                }
                Event::TempChanged { offset, data } => {
                    if let Some(p) = self.patches.get_mut(&PatchId::Temp) {
                        p.apply(offset, &data);
                    }
                    self.recent.push((offset, offset + data.len() as u32, Instant::now()));
                }
                Event::CurrentPatch(n) => {
                    self.current = Some(n);
                    self.selected_slot = None;
                    self.request(PatchId::User(n));
                    // The worker re-reads the live patch now; show that it's coming.
                    if !self.loading.contains(&PatchId::Temp) {
                        self.loading.push(PatchId::Temp);
                    }
                }
                Event::BackupProgress { done, total } => self.backup_progress = Some((done, total)),
                Event::BackupCancelled => self.backup_progress = None,
                Event::BackupDone(b) => {
                    self.backup_progress = None;
                    self.backup_finished(*b);
                }
                Event::System(areas) => self.system = areas,
                Event::SystemChanged { addr, data } => self.apply_system(addr, &data),
                Event::Saved(n) => {
                    self.auto_write_restart();
                    if self.auto_writing.take() == Some(n) {
                        let at = chrono::Local::now().format("%H:%M");
                        self.notice = Some(format!("Patch {} auto-written at {at}.", self.settings.numbering.label(n)));
                    } else if self.current == Some(n) || self.view == View::Patch {
                        self.notice = Some(format!("Patch {} written.", self.settings.numbering.label(n)));
                    }
                }
                Event::ChainLimits(l) => self.chain_limits = Some(l),
                Event::Tuner(r) => self.tuner.update(r),
                Event::RunningMode(m) => self.pedal_running_mode(m),
                Event::ChainEdited { select } => {
                    self.chain_busy = false;
                    if select.is_some() {
                        self.selected_slot = select;
                    }
                }
                Event::SetlistProgress { done, total } => self.setlist_write = Some((done, total)),
                Event::SetlistDone { written, total } => self.setlist_write_finished(written, total),
                Event::Error(e) => {
                    self.loading.clear();
                    self.auto_writing = None;
                    if matches!(self.conn, Conn::Connecting) && self.auto_reconnect {
                        // The ports are back but the pedal isn't answering yet (still starting up): try again.
                        self.conn = Conn::Lost;
                        self.last_port_check = Instant::now();
                    } else if matches!(self.conn, Conn::Connecting) {
                        self.conn = Conn::Failed(e);
                    } else if !matches!(self.conn, Conn::Lost) {
                        // (Once the pedal is gone, the timeouts of what was still on its way say nothing new.)
                        self.last_error = Some(e);
                    }
                }
            }
        }
        self.recent.retain(|&(_, _, t)| t.elapsed() < FLASH);
    }

    /// Whether the pedal changed any of `len` bytes at `offset` of the live patch just now.
    fn flashing(&self, offset: u32, len: u32) -> bool {
        self.recent.iter().any(|&(s, e, _)| s < offset + len && offset < e)
    }

    // ------------------------------------------------------------------ panels

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("OpenGX100 Editor");
            ui.separator();
            ui.selectable_value(&mut self.view, View::Patch, "Patch").on_hover_text("The patch active on the pedal");
            ui.selectable_value(&mut self.view, View::CtlExp, "CTL/EXP");
            ui.selectable_value(&mut self.view, View::InOut, "IN/OUT");
            ui.selectable_value(&mut self.view, View::System, "System");
            ui.selectable_value(&mut self.view, View::Setlists, "Setlists")
                .on_hover_text("Saved lists of patches in a new order, to write into a range of the pedal");
            let backups = if self.backup_progress.is_some() { "Backups ⟳" } else { "Backups" };
            ui.selectable_value(&mut self.view, View::Backups, backups)
                .on_hover_text("Back up everything on the pedal");
            ui.separator();
            ui.selectable_value(&mut self.view, View::Tuner, "Tuner")
                .on_hover_text("Show the pedal's tuner (it stays on while this page is open)");
            ui.separator();
            match &self.conn {
                Conn::Connecting => {
                    ui.spinner();
                    ui.label("Connecting…");
                }
                Conn::Connected(desc) => {
                    ui.colored_label(GREEN, "●");
                    ui.label(desc);
                    if self.live {
                        ui.label(RichText::new("LIVE").color(GREEN).strong()).on_hover_text(
                            "The pedal sends every change as it happens (footswitches, expression pedal, knobs, \
                             patch changes)",
                        );
                    }
                }
                Conn::Failed(e) => {
                    ui.colored_label(RED, "●");
                    ui.label(e).on_hover_text(e);
                }
                Conn::Lost => {
                    ui.colored_label(RED, "●");
                    ui.label("GX-100 disconnected, waiting for it…")
                        .on_hover_text("Plug it in or switch it on: the editor reconnects by itself");
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Use simulator").clicked() {
                    self.connect(true);
                }
                if ui.button("Connect pedal").clicked() {
                    self.connect(false);
                }
                let connected = matches!(self.conn, Conn::Connected(_));
                if ui.add_enabled(connected, egui::Button::new("Reload")).on_hover_text("Re-read everything").clicked()
                {
                    self.reload();
                }
            });
        });
        if let Some(e) = self.last_error.clone() {
            ui.horizontal(|ui| {
                ui.colored_label(RED, format!("Error: {e}"));
                if ui.small_button("dismiss").clicked() {
                    self.last_error = None;
                }
            });
        }
        if let Some(n) = self.notice.clone() {
            ui.horizontal(|ui| {
                ui.colored_label(GREEN, n);
                if ui.small_button("dismiss").clicked() {
                    self.notice = None;
                }
            });
        }
    }

    fn patch_list(&mut self, ui: &mut egui::Ui) {
        let connected = matches!(self.conn, Conn::Connected(_));
        ui.add_enabled_ui(connected, |ui| {
            if self.view != View::Setlists {
                ui.label(RichText::new("click a patch to switch the pedal to it").weak());
            }
            ui.horizontal(|ui| {
                ui.label("Filter");
                ui.add(egui::TextEdit::singleline(&mut self.filter).desired_width(f32::INFINITY));
            });
            ui.horizontal(|ui| {
                ui.label("Numbers");
                let before = self.settings.numbering;
                ui.selectable_value(&mut self.settings.numbering, Numbering::Sequential, "001")
                    .on_hover_text("Running number, 001 to 200");
                ui.selectable_value(&mut self.settings.numbering, Numbering::Bank, "U01-1")
                    .on_hover_text("As on the pedal: bank U01 to U50, 4 patches each");
                if self.settings.numbering != before {
                    if let Some(store) = &self.store {
                        if let Err(e) = store.save_settings(&self.settings) {
                            self.last_error = Some(format!("couldn't save the settings: {e}"));
                        }
                    }
                }
            });
            let loaded = self.names.iter().filter(|n| n.is_some()).count();
            if connected && loaded < self.names.len() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(format!("Reading names… {loaded}/{}", self.names.len()));
                });
            }
            ui.separator();

            let filter = self.filter.to_lowercase();
            if self.view == View::Setlists {
                let n = self.active_setlist().map(|s| s.entries.len()).unwrap_or(0);
                ui.label(RichText::new(format!("tick patches to add them to the setlist ({n} in it)")).weak());
            }
            let rows: Vec<u16> = (1..=USER_PATCHES)
                .filter(|&n| {
                    filter.is_empty()
                        || self.names[n as usize - 1].as_deref().is_some_and(|s| s.to_lowercase().contains(&filter))
                        || format!("{n:03}").contains(&filter)
                        || self.settings.numbering.label(n).to_lowercase().contains(&filter)
                })
                .collect();
            let row_h = ui.text_style_height(&egui::TextStyle::Body) + 4.0;
            egui::ScrollArea::vertical().auto_shrink(false).show_rows(ui, row_h, rows.len(), |ui, range| {
                for &n in &rows[range] {
                    let name = self.names[n as usize - 1].as_deref().unwrap_or("…");
                    let mut text = RichText::new(format!("{}  {name}", self.settings.numbering.label(n))).monospace();
                    if self.current == Some(n) {
                        text = text.color(AMBER);
                    }
                    if self.view == View::Setlists {
                        let mut on = self.in_setlist(n);
                        if ui.checkbox(&mut on, text).changed() {
                            self.toggle_pick(n);
                        }
                        continue;
                    }
                    let r = ui.selectable_label(self.current == Some(n), text);
                    let r = if self.current == Some(n) { r.on_hover_text("The pedal is on this patch") } else { r };
                    if r.clicked() {
                        self.switch_to(n);
                    }
                }
            });
        });
    }

    /// The live editor: always the patch active on the pedal, as it is now (including changes made on
    /// the pedal). Edits go to the pedal right away; WRITE stores them.
    fn patch_view(&mut self, ui: &mut egui::Ui) {
        let id = PatchId::Temp;
        let Some(data) = self.patches.get(&id).cloned() else {
            ui.heading("Live patch");
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Reading the active patch…");
            });
            return;
        };

        ui.horizontal(|ui| {
            let number = self.current.map(|n| format!("{}  ", self.settings.numbering.label(n))).unwrap_or_default();
            ui.heading(RichText::new(format!("{number}{}", data.name())).strong());
            ui.label(RichText::new("active on the pedal").weak());
            if self.loading.contains(&id) {
                ui.spinner();
            }
            let unsaved = self.unsaved_changes();
            match unsaved {
                Some(true) => {
                    ui.label(RichText::new("● edited").color(AMBER).strong())
                        .on_hover_text("The live patch differs from the stored one. WRITE stores the changes.");
                }
                Some(false) => {
                    ui.label(RichText::new("saved").color(GREEN));
                }
                None => {}
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let connected = matches!(self.conn, Conn::Connected(_));
                if ui
                    .add_enabled(connected, egui::Button::new("Write…"))
                    .on_hover_text("Store the live patch")
                    .clicked()
                {
                    self.open_save_dialog();
                }
                ui.add_enabled_ui(connected, |ui| self.auto_write_controls(ui));
                if ui
                    .add_enabled(connected && unsaved == Some(true), egui::Button::new("Revert"))
                    .on_hover_text("Drop the changes: reload the stored patch")
                    .clicked()
                {
                    self.revert();
                }
            });
        });
        self.patch_settings_editor(ui, PATCH_SETTINGS);
        ui.separator();

        ui.horizontal(|ui| {
            ui.label(RichText::new("Signal chain").strong());
            ui.checkbox(&mut self.show_unused, "show unused slots");
            if self.chain_busy {
                ui.spinner();
                ui.label("changing the chain…");
            } else if self.connected() {
                ui.label(RichText::new("drag an effect to move it · click between two to add one").weak());
            }
        });
        let layout = data.chain_layout();
        let chain = data.chain();
        if self.selected_slot.is_none() {
            self.selected_slot = chain.iter().copied().find(|&s| !is_routing(&data, s));
        }
        // Folds to the width there is; scrolls only if one piece (a wide parallel section) doesn't fit.
        let width = ui.available_width();
        egui::ScrollArea::horizontal()
            .id_salt("chain")
            .scroll_source(SCROLL_NOT_DRAG)
            .show(ui, |ui| self.chain_diagram(ui, &data, &layout, width));
        if self.show_unused {
            let unused: Vec<u8> = (1..=FX_SLOTS).filter(|s| !chain.contains(s)).collect();
            if !unused.is_empty() {
                ui.label(RichText::new("Unused slots (not in the chain; the pedal keeps old settings here)").weak());
                ui.horizontal_wrapped(|ui| {
                    for s in unused {
                        let (rect, _) = ui.allocate_exact_size(vec2(BLOCK_W, BLOCK_H), Sense::hover());
                        if self.fx_block(ui, &data, s, rect, false).clicked() {
                            self.selected_slot = Some(s);
                        }
                    }
                });
            }
        }
        ui.separator();

        if let Some(slot) = self.selected_slot {
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| self.fx_params(ui, &data, slot));
        }
    }

    /// The chain as boxes and wires. A parallel section is drawn as two rows that leave the
    /// DIVIDER and meet again at the MIXER; the SPLITTER (only a marker) isn't drawn.
    /// The path the signal takes is drawn bright; a branch the DIVIDER doesn't feed is dashed and dimmed.
    /// Every gap between two blocks is a place an effect can be added or moved to.
    ///
    /// When the chain is wider than `width` it folds like a snake: the first line runs left to right,
    /// the next right to left below it, and so on. A parallel section always stays on one line.
    fn chain_diagram(&mut self, ui: &mut egui::Ui, data: &PatchData, layout: &ChainLayout, width: f32) {
        let chain = data.chain();
        let pos_of = |s: u8| chain.iter().position(|&c| c == s).unwrap_or(chain.len());
        // (centre, position in the chain an effect put there would get)
        let mut gaps: Vec<(Pos2, usize)> = Vec::new();
        let width_of = |s: u8| if is_routing(data, s) { ROUTING_W } else { BLOCK_W };
        let row_w = |slots: &[u8]| slots.iter().map(|&s| width_of(s) + GAP).sum::<f32>();
        let par_w = layout.branch.as_ref().map(|b| row_w(&b.a).max(row_w(&b.b)).max(GAP)).unwrap_or(0.0);
        let branch_w =
            layout.branch.as_ref().map(|b| width_of(b.divider) + GAP + par_w + width_of(b.mixer)).unwrap_or(0.0);

        let mut units = vec![Unit::In];
        units.extend(layout.pre.iter().map(|&s| Unit::Block(s)));
        if layout.branch.is_some() {
            units.push(Unit::Branch);
        }
        units.extend(layout.post.iter().map(|&s| Unit::Block(s)));
        units.push(Unit::Out);
        let unit_w = |u: Unit| match u {
            Unit::In | Unit::Out => TERMINAL_W,
            Unit::Block(s) => width_of(s),
            Unit::Branch => branch_w,
        };

        // Fill each line as far as the width allows (room is kept on both sides for the wire that turns).
        let widest = units.iter().map(|&u| unit_w(u)).fold(0.0, f32::max);
        let line_w = (width - 2.0 * TURN).max(widest);
        let mut lines: Vec<Vec<Unit>> = vec![Vec::new()];
        let mut used = 0.0;
        for &u in &units {
            let line = lines.last_mut().expect("at least one line");
            let need = if line.is_empty() { unit_w(u) } else { used + GAP + unit_w(u) };
            if need > line_w && !line.is_empty() {
                lines.push(vec![u]);
                used = unit_w(u);
            } else {
                line.push(u);
                used = need;
            }
        }
        let line_h = |l: &[Unit]| if l.contains(&Unit::Branch) { 2.0 * BLOCK_H + ROW_GAP } else { BLOCK_H };
        let total_h = lines.iter().map(|l| line_h(l)).sum::<f32>() + LINE_GAP * (lines.len() - 1) as f32 + 8.0;

        let (canvas, _) = ui.allocate_exact_size(vec2(line_w + 2.0 * TURN, total_h), Sense::hover());
        let painter = ui.painter_at(canvas.expand(2.0));
        let live = Wire { stroke: Stroke::new(2.0, SIGNAL), dashed: false };
        let dead = Wire { stroke: Stroke::new(1.0, Color32::from_gray(85)), dashed: true };

        // Wires the previous output to the next input and returns the middle of that wire (a gap).
        let link = |from: Pos2, to: Pos2, w: Wire| -> Pos2 {
            if (from.y - to.y).abs() < 0.5 {
                connect(&painter, w, from, to);
                return pos2((from.x + to.x) / 2.0, to.y);
            }
            // Down to the next line, turning on the side where the previous line ended.
            let right = to.x >= canvas.center().x;
            let x = if right { from.x.max(to.x) + TURN / 2.0 } else { from.x.min(to.x) - TURN / 2.0 };
            let corners = [from, pos2(x, from.y), pos2(x, to.y), to];
            w.segment(&painter, &corners);
            arrowhead(&painter, w, to, if to.x < x { -1.0 } else { 1.0 });
            pos2(x, (from.y + to.y) / 2.0)
        };
        let block = |this: &mut Self, ui: &mut egui::Ui, s: u8, rect: Rect, dimmed: bool| {
            let r = this.fx_block(ui, data, s, rect, dimmed);
            if r.clicked() {
                this.selected_slot = Some(s);
            }
            if r.drag_started() && this.can_edit_chain() {
                this.chain_drag = Some(s);
                this.selected_slot = Some(s);
            }
        };
        let terminal = |rect: Rect, text: &str| {
            painter.rect_stroke(rect, 10.0, live.stroke, egui::StrokeKind::Inside);
            painter.text(rect.center(), egui::Align2::CENTER_CENTER, text, egui::FontId::monospace(10.0), SIGNAL);
        };

        let mut prev: Option<Pos2> = None;
        let mut top = canvas.top() + 4.0;
        for (i, units) in lines.iter().enumerate() {
            let h = line_h(units);
            let line = Line { left: canvas.left() + TURN, width: line_w, rtl: i % 2 == 1, y: top + h / 2.0 };
            top += h + LINE_GAP;
            let mut x = 0.0;
            for &u in units {
                let w = unit_w(u);
                match u {
                    Unit::In => {
                        let r = line.rect(x, w, line.y, 20.0);
                        terminal(r, "IN");
                        prev = Some(line.output(r));
                    }
                    Unit::Out => {
                        let r = line.rect(x, w, line.y, 20.0);
                        if let Some(from) = prev {
                            gaps.push((link(from, line.input(r), live), chain.len()));
                        }
                        terminal(r, "OUT");
                    }
                    Unit::Block(s) => {
                        let r = line.rect(x, w, line.y, BLOCK_H);
                        if let Some(from) = prev {
                            gaps.push((link(from, line.input(r), live), pos_of(s)));
                        }
                        block(self, ui, s, r, false);
                        prev = Some(line.output(r));
                    }
                    Unit::Branch => {
                        let Some(b) = &layout.branch else { continue };
                        let route = divider_route(data, b.divider);
                        let div = line.rect(x, width_of(b.divider), line.y, BLOCK_H);
                        if let Some(from) = prev {
                            gaps.push((link(from, line.input(div), live), pos_of(b.divider)));
                        }
                        block(self, ui, b.divider, div, false);
                        let split = line.output(div);
                        let start = x + width_of(b.divider) + GAP;
                        let par_end = start + par_w - GAP;
                        let (row_a, row_b) = (line.y - (BLOCK_H + ROW_GAP) / 2.0, line.y + (BLOCK_H + ROW_GAP) / 2.0);
                        let mut ends = Vec::new();
                        // The end of branch A is just before the SPLITTER; the end of B just before the MIXER.
                        for (slots, y, active, end) in
                            [(&b.a, row_a, route.a, pos_of(b.splitter)), (&b.b, row_b, route.b, pos_of(b.mixer))]
                        {
                            let wire = if active { live } else { dead };
                            let mut from = split;
                            let mut bx = start;
                            for &s in slots.iter() {
                                let r = line.rect(bx, width_of(s), y, BLOCK_H);
                                connect(&painter, wire, from, line.input(r));
                                gaps.push((line.at((line.logical(from.x) + bx) / 2.0, y), pos_of(s)));
                                block(self, ui, s, r, wire.dashed);
                                from = line.output(r);
                                bx += width_of(s) + GAP;
                            }
                            // Run the wire along the row to the end of the parallel section, so both rows meet together.
                            let row_end = line.at(par_end.max(bx - GAP), y);
                            if slots.is_empty() {
                                connect(&painter, wire, split, row_end);
                                gaps.push((pos2((split.x + row_end.x) / 2.0, y), end));
                            } else {
                                wire.segment(&painter, &[from, row_end]);
                                gaps.push((line.ahead(from, GAP / 2.0), end));
                            }
                            ends.push((row_end, wire));
                        }
                        let mixer = line.rect(start + par_w, width_of(b.mixer), line.y, BLOCK_H);
                        // Inactive wires first, so the live one is drawn on top where they meet.
                        ends.sort_by_key(|(_, w)| !w.dashed);
                        for (end, wire) in ends {
                            connect(&painter, wire, end, line.input(mixer));
                        }
                        block(self, ui, b.mixer, mixer, false);
                        prev = Some(line.output(mixer));
                        for (label, y, active) in [("A", row_a, route.a), ("B", row_b, route.b)] {
                            let color = if active { SIGNAL } else { dead.stroke.color };
                            let align = if line.rtl { egui::Align2::RIGHT_TOP } else { egui::Align2::LEFT_TOP };
                            let at = line.ahead(pos2(split.x, y - BLOCK_H / 2.0 + 2.0), 4.0);
                            painter.text(at, align, label, egui::FontId::proportional(11.0), color);
                        }
                    }
                }
                x += w + GAP;
            }
        }
        self.chain_gaps(ui, &painter, data, &gaps);
    }

    /// One clickable box: FX type name, lit when on, flashing when the pedal just changed it,
    /// with a bar for the expression-pedal position on pedal effects. `dimmed`: no signal reaches it.
    fn fx_block(&self, ui: &mut egui::Ui, data: &PatchData, slot: u8, rect: Rect, dimmed: bool) -> egui::Response {
        let resp = ui.interact(rect, egui::Id::unique(("fx-block", slot)), Sense::click_and_drag());
        let Some(s) = data.fx_slot(slot) else { return resp };
        let ty = s.fx_type();
        let name = ty.map(|t| t.name).unwrap_or("UNKNOWN");
        let routing = ROUTING_TYPES.contains(&name);
        let selected = self.selected_slot == Some(slot);
        let flash = slot_offset(slot).is_some_and(|o| self.flashing(o, 2));

        // As the official editor draws it: an effect that is on is filled with its type's colour (black text);
        // one that is off is only outlined in it (white text). DIVIDER and MIXER can't be off.
        let lit = s.on || routing;
        let (fill, edge, text_color) = match ty.and_then(|t| t.color) {
            Some([r, g, b]) => {
                let color = Color32::from_rgb(r, g, b);
                if lit {
                    (color, color, Color32::BLACK)
                } else {
                    (Color32::from_gray(24), color, Color32::WHITE)
                }
            }
            // Tables without colours: green when on.
            None if routing => (Color32::from_gray(70), Color32::from_gray(90), Color32::WHITE),
            None if s.on => (Color32::from_rgb(40, 110, 70), Color32::from_gray(90), Color32::WHITE),
            None => (Color32::from_gray(45), Color32::from_gray(90), Color32::from_gray(150)),
        };
        let fill = if resp.hovered() { fill.gamma_multiply(1.15) } else { fill };
        let (fill, edge) = if dimmed { (fill.gamma_multiply(0.45), edge.gamma_multiply(0.45)) } else { (fill, edge) };
        let text_color = if dimmed { text_color.gamma_multiply(0.6) } else { text_color };
        let painter = ui.painter();
        painter.rect(rect, 5.0, fill, Stroke::new(2.5, edge), egui::StrokeKind::Inside);
        // Selection and pedal changes are marked around the block, so its colour stays visible.
        if selected {
            painter.rect_stroke(rect, 5.0, Stroke::new(2.5, AMBER), egui::StrokeKind::Outside);
        } else if flash {
            painter.rect_stroke(rect, 5.0, Stroke::new(2.0, Color32::WHITE), egui::StrokeKind::Outside);
        }
        // The DIVIDER says where it sends the signal.
        let caption = (name == "DIVIDER").then(|| divider_route(data, slot).label());
        if let Some(c) = &caption {
            let color = if fill.intensity() > 0.5 { Color32::from_rgb(20, 60, 130) } else { SIGNAL };
            painter.text(
                pos2(rect.center().x, rect.bottom() - 9.0),
                egui::Align2::CENTER_CENTER,
                c,
                egui::FontId::proportional(10.0),
                color,
            );
        }
        let galley =
            painter.layout(name.to_string(), egui::FontId::proportional(11.5), text_color, rect.width() - 10.0);
        let lift = if caption.is_some() { 7.0 } else { 3.0 };
        let text_pos = rect.center() - galley.size() / 2.0 - vec2(0.0, lift);
        painter.galley(text_pos, galley, text_color);

        if let Some((p, v)) = ty.and_then(|t| pedal_position(data, slot, t)) {
            let frac = ((v - p.min) as f32 / (p.max - p.min).max(1) as f32).clamp(0.0, 1.0);
            let track = Rect::from_min_max(
                pos2(rect.left() + 6.0, rect.bottom() - 8.0),
                pos2(rect.right() - 6.0, rect.bottom() - 4.0),
            );
            painter.rect_filled(track, 2.0, Color32::from_gray(30));
            painter.rect_filled(Rect::from_min_size(track.min, vec2(track.width() * frac, track.height())), 2.0, AMBER);
        }
        let about = ty.and_then(|t| tables::fx_doc(t.name)).and_then(|d| d.text.lines().next()).unwrap_or("");
        let drag = if self.can_edit_chain() && name != "SPLITTER" { " · drag to move" } else { "" };
        let state = if routing {
            ""
        } else if s.on {
            " · ON"
        } else {
            " · OFF"
        };
        resp.on_hover_text(format!("slot {slot}{state}{drag}\n{about}").trim_end().to_string())
    }

    /// Every parameter of the effect in `slot`, editable. Values the pedal just changed are highlighted.
    fn fx_params(&mut self, ui: &mut egui::Ui, data: &PatchData, slot: u8) {
        let Some(s) = data.fx_slot(slot) else { return };
        let Some(t) = s.fx_type() else {
            ui.label(format!("Slot {slot} holds unknown FX type {}", s.type_id));
            return;
        };
        let doc = tables::fx_doc(t.name);
        ui.horizontal(|ui| {
            ui.heading(t.name);
            if let Some(on) = controls::switch(ui, s.on) {
                self.set_fx_switch(slot, on);
            }
            ui.label(RichText::new(format!("slot {slot}")).weak());
            self.presets_menu(ui, data, slot, t);
            self.chain_buttons(ui, data, slot);
        });
        // What the effect does (and what it models), from the parameter guide.
        if let Some(d) = doc.filter(|d| !d.text.is_empty()) {
            ui.add(egui::Label::new(RichText::new(d.text).weak()).wrap());
        }
        ui.add_space(4.0);
        // PARAMETRIC EQUALIZER: its frequency response, with draggable bands.
        let eq = eq_graph::Eq::read(s.on, |k| t.param(k.fx_name()).and_then(|p| data.fx_param(slot, p)));
        if let Some(eq) = eq {
            for (k, v) in eq_graph::graph(ui, ("fx-eq", slot), &eq) {
                if let Some(p) = t.param(k.fx_name()) {
                    self.set_fx_param(slot, p, v);
                }
            }
            ui.add_space(6.0);
        }
        let params: Vec<&'static FxParamDef> =
            t.params.iter().filter(|p| matches!(p.field, FxField::Param(_))).collect();
        if t.name == "PARAMETRIC EQUALIZER" && self.peq_controls(ui, data, slot, t) {
            return;
        }
        // Numbered sets (a dual CHORUS's "1: RATE", "2: RATE") get a row of their own.
        let mut groups: Vec<(Option<&str>, Vec<&'static FxParamDef>)> = Vec::new();
        for p in params {
            let g = controls::short_name(p.name).0;
            match groups.iter_mut().find(|(k, _)| *k == g) {
                Some((_, ps)) => ps.push(p),
                None => groups.push((g, vec![p])),
            }
        }
        for (g, ps) in groups {
            if let Some(g) = g {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(g).strong());
                    ui.add(egui::Separator::default().horizontal());
                });
            }
            let kind = |p: &FxParamDef| controls::control_for(t, p);
            // Knobs side by side, then the faders, then switches and lists.
            for row in [controls::Control::Knob, controls::Control::Fader] {
                if ps.iter().any(|p| kind(p) == row) {
                    controls::row(ui, |ui| {
                        for p in ps.iter().filter(|p| kind(p) == row) {
                            self.fx_scale(ui, data, slot, t, p, controls::short_name(p.name).1, row);
                        }
                    });
                    ui.add_space(8.0);
                }
            }
            let rest: Vec<_> =
                ps.iter().filter(|p| !matches!(kind(p), controls::Control::Knob | controls::Control::Fader)).collect();
            if rest.is_empty() {
                continue;
            }
            egui::Grid::new(("fx-params", slot, g)).num_columns(2).striped(true).spacing([16.0, 8.0]).show(ui, |ui| {
                for p in rest {
                    let pdoc = doc.and_then(|d| d.param(p.unique_name));
                    let color = if self.fx_flashing(slot, p) { AMBER } else { ui.visuals().text_color() };
                    editing::doc_label(ui, RichText::new(controls::short_name(p.name).1).color(color), pdoc);
                    let Some(v) = data.fx_param(slot, p) else {
                        ui.label("—");
                        ui.end_row();
                        continue;
                    };
                    ui.vertical(|ui| {
                        let new = match kind(p) {
                            controls::Control::Switch => ui
                                .horizontal(|ui| controls::switch(ui, v == p.max))
                                .inner
                                .map(|on| if on { p.max } else { p.min }),
                            controls::Control::Buttons => {
                                let mut picked = None;
                                ui.horizontal_wrapped(|ui| {
                                    for (o, label) in editing::fx_options(p) {
                                        let r = ui.radio(o == v, label);
                                        let r = match pdoc.and_then(|d| d.option(label)) {
                                            Some(t) => r.on_hover_text(t),
                                            None => r,
                                        };
                                        if r.clicked() && o != v {
                                            picked = Some(o);
                                        }
                                    }
                                });
                                picked
                            }
                            _ => editing::fx_param_widget(ui, slot, p, v, pdoc),
                        };
                        if let Some(n) = new {
                            self.set_fx_param(slot, p, n);
                        }
                        if !p.labels().is_empty() {
                            editing::option_note(ui, pdoc, &p.display(v));
                        }
                    });
                    ui.end_row();
                }
            });
        }
    }

    /// Whether the pedal just changed parameter `p` of the effect in `slot`.
    fn fx_flashing(&self, slot: u8, p: &FxParamDef) -> bool {
        slot_offset(slot).is_some_and(|b| self.flashing(b + p.offset.linear(), p.encoding.size()))
    }

    /// A knob or fader for parameter `p` of the effect in `slot`, labelled `name`; writes changes.
    #[allow(clippy::too_many_arguments)]
    fn fx_scale(
        &mut self,
        ui: &mut egui::Ui,
        data: &PatchData,
        slot: u8,
        t: &FxTypeDef,
        p: &FxParamDef,
        name: &str,
        kind: controls::Control,
    ) {
        let Some(value) = data.fx_param(slot, p) else { return };
        let pdoc = tables::fx_doc(t.name).and_then(|d| d.param(p.unique_name));
        let name_color = if self.fx_flashing(slot, p) { AMBER } else { ui.visuals().text_color() };
        let display = |v: i32| p.display(v);
        let scale = controls::Scale {
            name,
            name_color,
            value,
            min: p.min,
            max: p.max,
            default: controls::default_value(p.init, (p.min, p.max)),
            display: &display,
            hover: pdoc.map(|d| d.text).filter(|t| !t.is_empty()),
        };
        let new = ui.push_id(("fx", slot, p.unique_name), |ui| controls::knob_or_fader(ui, kind, &scale)).inner;
        if let Some(n) = new {
            self.set_fx_param(slot, p, n);
        }
    }

    /// PARAMETRIC EQUALIZER as an EQ unit ([`controls::eq_sections`]). False if the type lacks a setting.
    fn peq_controls(&mut self, ui: &mut egui::Ui, data: &PatchData, slot: u8, t: &FxTypeDef) -> bool {
        if eq_graph::Setting::ALL.iter().any(|k| t.param(k.fx_name()).is_none()) {
            return false;
        }
        controls::eq_sections(ui, |ui, k, label, kind| {
            let p = t.param(k.fx_name()).expect("checked above");
            self.fx_scale(ui, data, slot, t, p, label, kind);
        });
        true
    }
}

impl eframe::App for EditorApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_events();
        self.watch_ports(ctx);
        self.sync_tuner();
        self.autosave(ctx);
        self.auto_write(ctx);
        if !self.recent.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(100)); // let highlights fade
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::top("top").show(ui, |ui| {
            ui.add_space(4.0);
            self.top_bar(ui);
            ui.add_space(2.0);
        });
        egui::Panel::left("patches").resizable(true).default_size(240.0).show(ui, |ui| {
            ui.add_space(4.0);
            self.patch_list(ui);
        });
        egui::CentralPanel::default().show(ui, |ui| match &self.conn {
            Conn::Failed(e) if self.patches.is_empty() => {
                let e = e.clone();
                ui.vertical_centered(|ui| {
                    ui.add_space(60.0);
                    ui.heading("Couldn't reach the pedal");
                    ui.label(e);
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("Try again").clicked() {
                            self.connect(false);
                        }
                        if ui.button("Use simulator").clicked() {
                            self.connect(true);
                        }
                    });
                });
            }
            Conn::Lost if !matches!(self.view, View::Setlists | View::Backups) => {
                ui.vertical_centered(|ui| {
                    ui.add_space(60.0);
                    ui.heading("The GX-100 was disconnected");
                    ui.label("Plug it in or switch it on: the editor reconnects by itself.");
                    ui.add_space(8.0);
                    ui.spinner();
                });
            }
            _ => match self.view {
                View::Setlists => self.setlists_view(ui),
                View::Backups => self.backups_view(ui),
                View::CtlExp => self.ctl_exp_view(ui),
                View::InOut => self.in_out_view(ui),
                View::System => self.system_view(ui),
                View::Patch => self.patch_view(ui),
                View::Tuner => self.tuner_view(ui),
            },
        });
        let ctx = ui.ctx().clone();
        self.fx_picker(&ctx);
        self.dialogs(&ctx);
    }
}

impl Drop for EditorApp {
    /// Takes the pedal out of editor mode before quitting, as the official editor does.
    /// Also saves a setlist changed in the last moments.
    fn drop(&mut self) {
        self.save_active_setlist();
        if self.tuner.on {
            let _ = self.tx.send(Cmd::TunerOff);
        }
        let _ = self.tx.send(Cmd::Shutdown);
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}

// ------------------------------------------------------------------ helpers

fn slot_type(data: &PatchData, slot: u8) -> Option<&'static FxTypeDef> {
    data.fx_slot(slot).and_then(|s| s.fx_type())
}

fn is_routing(data: &PatchData, slot: u8) -> bool {
    slot_type(data, slot).is_some_and(|t| ROUTING_TYPES.contains(&t.name))
}

/// Offset of FX slot `slot` from the start of a patch.
fn slot_offset(slot: u8) -> Option<u32> {
    patch::fx_slot_base(patch::temp_patch(), slot).and_then(patch::temp_offset)
}

/// The expression-pedal position of a pedal effect (FOOT VOLUME, WAH, PEDAL BEND…).
fn pedal_position(data: &PatchData, slot: u8, t: &FxTypeDef) -> Option<(&'static FxParamDef, i32)> {
    let p = t.param("pedal-position")?;
    Some((p, data.fx_param(slot, p)?))
}

/// How a wire is drawn: solid where the signal flows, dashed where it doesn't.
#[derive(Clone, Copy)]
struct Wire {
    stroke: Stroke,
    dashed: bool,
}

impl Wire {
    fn segment(self, painter: &egui::Painter, points: &[Pos2]) {
        if self.dashed {
            painter.extend(egui::Shape::dashed_line(points, self.stroke, 4.0, 4.0));
        } else {
            painter.add(egui::Shape::line(points.to_vec(), self.stroke));
        }
    }
}

/// Which branches a DIVIDER feeds: both in DUAL mode, else the one `ch-select` picks.
#[derive(Clone, Copy)]
struct Route {
    a: bool,
    b: bool,
}

impl Route {
    fn label(self) -> &'static str {
        match (self.a, self.b) {
            (true, true) => "A + B",
            (true, false) => "A only",
            (false, true) => "B only",
            (false, false) => "",
        }
    }
}

fn divider_route(data: &PatchData, slot: u8) -> Route {
    let get = |name: &str| slot_type(data, slot).and_then(|t| t.param(name)).and_then(|p| data.fx_param(slot, p));
    match (get("mode"), get("ch-select")) {
        (Some(1), _) | (None, _) => Route { a: true, b: true }, // DUAL (or unreadable: assume both)
        (Some(_), Some(1)) => Route { a: false, b: true },
        (Some(_), _) => Route { a: true, b: false },
    }
}

/// A wire from an output to an input, either way across: straight on one row, an S-curve between rows.
fn connect(painter: &egui::Painter, wire: Wire, from: Pos2, to: Pos2) {
    let dir = if to.x < from.x { -1.0 } else { 1.0 };
    if (from.y - to.y).abs() < 0.5 {
        wire.segment(painter, &[from, to]);
    } else {
        let dx = ((to.x - from.x).abs() / 2.0).max(8.0) * dir;
        let curve = egui::epaint::CubicBezierShape::from_points_stroke(
            [from, pos2(from.x + dx, from.y), pos2(to.x - dx, to.y), to],
            false,
            Color32::TRANSPARENT,
            wire.stroke,
        );
        wire.segment(painter, &curve.flatten(Some(0.5)));
    }
    arrowhead(painter, wire, to, dir);
}

/// An arrowhead at `to`, pointing right (`dir` 1) or left (-1). None on dashed (dead) wires.
fn arrowhead(painter: &egui::Painter, wire: Wire, to: Pos2, dir: f32) {
    if !wire.dashed {
        painter.add(egui::Shape::convex_polygon(
            vec![to, pos2(to.x - 6.0 * dir, to.y - 3.5), pos2(to.x - 6.0 * dir, to.y + 3.5)],
            wire.stroke.color,
            Stroke::NONE,
        ));
    }
}

/// One piece of the chain diagram that stays on one line: a terminal, an effect, or the whole parallel section.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Unit {
    In,
    Out,
    Block(u8),
    Branch,
}

/// A line of the folded chain diagram. Positions along it (`x`) count from where the signal enters it:
/// the left edge, or the right edge on lines that run right to left.
#[derive(Clone, Copy)]
struct Line {
    left: f32,
    width: f32,
    rtl: bool,
    y: f32,
}

impl Line {
    /// The point `x` along the line, at height `y`.
    fn at(self, x: f32, y: f32) -> Pos2 {
        pos2(if self.rtl { self.left + self.width - x } else { self.left + x }, y)
    }

    /// How far along the line the screen position `px` is.
    fn logical(self, px: f32) -> f32 {
        if self.rtl {
            self.left + self.width - px
        } else {
            px - self.left
        }
    }

    /// Something `w` wide starting `x` along the line, `h` high, centred on `y`.
    fn rect(self, x: f32, w: f32, y: f32, h: f32) -> Rect {
        let x0 = if self.rtl { self.left + self.width - x - w } else { self.left + x };
        Rect::from_min_size(pos2(x0, y - h / 2.0), vec2(w, h))
    }

    fn input(self, r: Rect) -> Pos2 {
        if self.rtl {
            r.right_center()
        } else {
            r.left_center()
        }
    }

    fn output(self, r: Rect) -> Pos2 {
        if self.rtl {
            r.left_center()
        } else {
            r.right_center()
        }
    }

    /// `p` moved `d` further along the line.
    fn ahead(self, p: Pos2, d: f32) -> Pos2 {
        pos2(if self.rtl { p.x - d } else { p.x + d }, p.y)
    }
}
