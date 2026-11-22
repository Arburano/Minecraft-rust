//! Menu screens (the in-game launcher, ADR-0010) and the in-game overlay.

use std::path::PathBuf;

use auth::Identity;
use client::packs::{self, ShaderPack};
use client::settings::Settings;
use client::worlds::{self, WorldInfo};
use egui::{Align, Color32, Layout, RichText, Ui};
use render::selection::{BackendKind, PowerPreference};
use render::terrain::profiles::QualityProfile;

/// Window title and menu heading.
// OPEN-QUESTION: 15 — the game has no name yet; a neutral working title.
pub const TITLE: &str = "Voxel Engine (working title)";

/// `docs/legal.md` rule 4.
pub const LEGAL_NOTICE: &str =
    "Not an official Minecraft product. Not approved by or associated with Mojang or Microsoft.";

/// Which menu screen is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    /// Title screen.
    Main,
    /// Single-player world list and creation.
    Worlds,
    /// Settings.
    Settings,
    /// About and the legal notice.
    About,
    /// Resource and texture packs (enable, order).
    ResourcePacks,
    /// Shader pack (choose, options).
    ShaderPack,
}

/// What the player asked for.
pub enum MenuAction {
    /// Nothing.
    None,
    /// Open a world.
    Play(PathBuf),
    /// Settings were changed and confirmed.
    SaveSettings(Settings),
    /// Pack choices changed: save and apply them now (also in a running world).
    ApplyPacks(Settings),
    /// Leave the game.
    Quit,
}

/// Menu state.
pub struct Menu {
    /// Open screen.
    pub screen: Screen,
    worlds_root: PathBuf,
    worlds: Vec<(PathBuf, WorldInfo)>,
    new_name: String,
    new_seed: String,
    draft: Settings,
    name: String,
    confirm_delete: Option<PathBuf>,
    /// A message shown on the next screen (errors, notices).
    pub message: Option<String>,
    /// GPU line for the settings and about screens.
    pub gpu_line: String,
    packs_root: PathBuf,
    /// Screen to return to from the pack screens (settings, or the pause menu).
    pub packs_return: Option<Screen>,
    shader_cache: Option<(String, Result<ShaderPack, String>)>,
}

impl Menu {
    /// A menu over the worlds in `worlds_root`.
    #[must_use]
    pub fn new(worlds_root: PathBuf, settings: &Settings) -> Self {
        let packs_root = worlds_root
            .parent()
            .map_or_else(PathBuf::new, PathBuf::from);
        Self {
            packs_root,
            packs_return: None,
            shader_cache: None,
            screen: Screen::Main,
            worlds: worlds::list(&worlds_root),
            worlds_root,
            new_name: "New world".into(),
            new_seed: String::new(),
            draft: settings.clone(),
            name: settings.offline_name.clone().unwrap_or_default(),
            confirm_delete: None,
            message: None,
            gpu_line: String::new(),
        }
    }

    /// The offline identity typed on the title screen, if valid.
    #[must_use]
    pub fn identity(&self) -> Option<Identity> {
        Identity::offline(self.name.trim()).ok()
    }

    /// Re-reads the world list (after playing).
    pub fn refresh(&mut self) {
        self.worlds = worlds::list(&self.worlds_root);
    }

    /// Draws the open screen.
    pub fn show(&mut self, ui: &mut Ui, settings: &Settings) -> MenuAction {
        let mut action = MenuAction::None;
        egui::CentralPanel::default_margins()
            .frame(egui::Frame::NONE.fill(Color32::from_rgb(24, 28, 36)))
            .show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(40.0);
                    ui.heading(RichText::new(TITLE).size(30.0).strong());
                    ui.add_space(24.0);
                    if let Some(m) = &self.message {
                        ui.colored_label(Color32::from_rgb(255, 200, 120), m);
                        ui.add_space(8.0);
                    }
                    action = match self.screen {
                        Screen::Main => self.main(ui, settings),
                        Screen::Worlds => self.worlds_screen(ui),
                        Screen::Settings => self.settings_screen(ui),
                        Screen::About => self.about(ui),
                        Screen::ResourcePacks => self.resource_packs(ui),
                        Screen::ShaderPack => self.shader_pack(ui),
                    };
                });
            });
        action
    }

    fn main(&mut self, ui: &mut Ui, settings: &Settings) -> MenuAction {
        let button = |ui: &mut Ui, text: &str| {
            ui.add_sized([260.0, 36.0], egui::Button::new(text))
                .clicked()
        };
        ui.label("Offline name (development and private tests only):");
        ui.add_sized([260.0, 24.0], egui::TextEdit::singleline(&mut self.name));
        let valid = self.identity().is_some();
        if !valid {
            ui.small("3–16 letters, digits or _");
        }
        ui.add_space(12.0);
        let mut action = MenuAction::None;
        if ui
            .add_enabled_ui(valid, |ui| button(ui, "Single-player"))
            .inner
        {
            self.refresh();
            self.message = None;
            self.screen = Screen::Worlds;
            if settings.offline_name.as_deref() != Some(self.name.trim()) {
                let mut s = settings.clone();
                s.offline_name = Some(self.name.trim().to_owned());
                self.draft = s.clone();
                action = MenuAction::SaveSettings(s);
            }
        }
        ui.add_enabled(
            false,
            egui::Button::new("Multiplayer (stage 2)").min_size([260.0, 36.0].into()),
        );
        if button(ui, "Settings") {
            self.draft = settings.clone();
            self.message = None;
            self.screen = Screen::Settings;
        }
        if button(ui, "About") {
            self.screen = Screen::About;
        }
        if button(ui, "Quit") {
            return MenuAction::Quit;
        }
        ui.add_space(30.0);
        ui.small(LEGAL_NOTICE);
        action
    }

    fn worlds_screen(&mut self, ui: &mut Ui) -> MenuAction {
        let mut action = MenuAction::None;
        ui.label(RichText::new("Single-player").size(20.0));
        ui.add_space(8.0);
        egui::ScrollArea::vertical()
            .max_height(260.0)
            .show(ui, |ui| {
                if self.worlds.is_empty() {
                    ui.label("No worlds yet.");
                }
                for (dir, info) in &self.worlds {
                    ui.horizontal(|ui| {
                        ui.add_space(ui.available_width() / 2.0 - 230.0);
                        ui.label(RichText::new(&info.name).strong());
                        ui.small(format!("seed {}", info.seed));
                        if ui.button("Play").clicked() {
                            action = MenuAction::Play(dir.clone());
                        }
                        if self.confirm_delete.as_ref() == Some(dir) {
                            if ui.button("Really delete").clicked() {
                                match worlds::delete(dir) {
                                    Ok(()) => {
                                        self.message = Some(format!("Deleted \"{}\".", info.name))
                                    }
                                    Err(e) => self.message = Some(e.to_string()),
                                }
                                self.confirm_delete = None;
                            }
                        } else if ui.button("Delete").clicked() {
                            self.confirm_delete = Some(dir.clone());
                        }
                    });
                }
            });
        if self
            .message
            .as_deref()
            .is_some_and(|m| m.starts_with("Deleted"))
        {
            self.refresh();
        }
        ui.add_space(16.0);
        ui.label(RichText::new("New world").size(18.0));
        egui::Grid::new("new_world").num_columns(2).show(ui, |ui| {
            ui.label("Name");
            ui.text_edit_singleline(&mut self.new_name);
            ui.end_row();
            ui.label("Seed (empty = random)");
            ui.text_edit_singleline(&mut self.new_seed);
            ui.end_row();
        });
        if ui.button("Create and play").clicked() {
            let seed = worlds::seed_from_text(&self.new_seed);
            match worlds::create(&self.worlds_root, &self.new_name, seed) {
                Ok(dir) => action = MenuAction::Play(dir),
                Err(e) => self.message = Some(e.to_string()),
            }
        }
        ui.add_space(16.0);
        if ui.button("Back").clicked() {
            self.message = None;
            self.confirm_delete = None;
            self.screen = Screen::Main;
        }
        action
    }

    fn settings_screen(&mut self, ui: &mut Ui) -> MenuAction {
        let mut action = MenuAction::None;
        let d = &mut self.draft;
        egui::Grid::new("settings")
            .num_columns(2)
            .spacing([16.0, 8.0])
            .show(ui, |ui| {
                ui.label("Quality");
                ui.horizontal(|ui| {
                    for (p, name) in [
                        (QualityProfile::Low, "Low (8 chunks)"),
                        (QualityProfile::Default, "Default (12)"),
                        (QualityProfile::High, "High (16)"),
                    ] {
                        ui.radio_value(&mut d.quality, p, name);
                    }
                });
                ui.end_row();
                ui.label("Field of view");
                ui.add(egui::Slider::new(&mut d.fov_degrees, 50.0..=110.0).suffix("°"));
                ui.end_row();
                ui.label("Mouse sensitivity");
                ui.add(egui::Slider::new(&mut d.mouse_sensitivity, 0.5..=6.0));
                ui.end_row();
                ui.label("Graphics API");
                egui::ComboBox::from_id_salt("backend")
                    .selected_text(
                        d.gpu
                            .backend
                            .map_or("Automatic".to_owned(), |b| format!("{b:?}")),
                    )
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut d.gpu.backend, None, "Automatic");
                        for b in [
                            BackendKind::Vulkan,
                            BackendKind::Dx12,
                            BackendKind::Metal,
                            BackendKind::Gl,
                        ] {
                            ui.selectable_value(&mut d.gpu.backend, Some(b), format!("{b:?}"));
                        }
                    });
                ui.end_row();
                ui.label("GPU");
                ui.horizontal(|ui| {
                    ui.radio_value(
                        &mut d.gpu.power,
                        PowerPreference::HighPerformance,
                        "Discrete first",
                    );
                    ui.radio_value(
                        &mut d.gpu.power,
                        PowerPreference::LowPower,
                        "Integrated first",
                    );
                });
                ui.end_row();
                ui.label("GPU name contains");
                let mut name = d.gpu.adapter_name.clone().unwrap_or_default();
                if ui.text_edit_singleline(&mut name).changed() {
                    d.gpu.adapter_name = (!name.trim().is_empty()).then(|| name.trim().to_owned());
                }
                ui.end_row();
                ui.label("Autosave every");
                ui.add(egui::Slider::new(&mut d.autosave_seconds, 10..=300).suffix(" s"));
                ui.end_row();
                ui.label("Textures");
                ui.checkbox(
                    &mut d.own_assets.enabled,
                    "Use textures from my own licensed installation (read locally, never uploaded)",
                );
                ui.end_row();
                ui.label("Installation folder");
                let mut folder = d
                    .own_assets
                    .folder
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default();
                let auto = d.own_assets.resolve_folder();
                let hint = auto
                    .as_ref()
                    .map_or("not found; type the folder path".to_owned(), |p| {
                        format!("auto: {}", p.display())
                    });
                if ui
                    .add(egui::TextEdit::singleline(&mut folder).hint_text(hint))
                    .changed()
                {
                    d.own_assets.folder =
                        (!folder.trim().is_empty()).then(|| PathBuf::from(folder.trim()));
                }
                ui.end_row();
                ui.label("Version");
                let found = auto
                    .as_deref()
                    .and_then(|f| assets::local_install::versions(f).ok())
                    .unwrap_or_default();
                egui::ComboBox::from_id_salt("own_version")
                    .selected_text(
                        d.own_assets
                            .version
                            .clone()
                            .unwrap_or_else(|| "Newest release".into()),
                    )
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut d.own_assets.version, None, "Newest release");
                        for v in &found {
                            ui.selectable_value(
                                &mut d.own_assets.version,
                                Some(v.id.clone()),
                                format!("{} ({})", v.id, v.kind),
                            );
                        }
                    });
                ui.end_row();
            });
        ui.horizontal(|ui| {
            ui.add_space(ui.available_width() / 2.0 - 150.0);
            if ui.button("Resource packs…").clicked() {
                self.open_packs(Screen::ResourcePacks, Screen::Settings);
            }
            if ui.button("Shader pack…").clicked() {
                self.open_packs(Screen::ShaderPack, Screen::Settings);
            }
        });
        ui.small("Texture changes apply when a world is opened.");
        ui.small("GPU changes apply after a restart.");
        ui.small(&self.gpu_line);
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.add_space(ui.available_width() / 2.0 - 80.0);
            if ui.button("Save").clicked() {
                action = MenuAction::SaveSettings(self.draft.clone());
                self.screen = Screen::Main;
            }
            if ui.button("Cancel").clicked() {
                self.screen = Screen::Main;
            }
        });
        action
    }

    /// Opens a pack screen; `back` is where Done/Cancel return (`None` for
    /// the pause menu in a running world).
    pub fn open_packs_from_game(&mut self, screen: Screen, settings: &Settings) {
        self.draft = settings.clone();
        self.screen = screen;
        self.packs_return = None;
        self.shader_cache = None;
    }

    fn open_packs(&mut self, screen: Screen, back: Screen) {
        self.screen = screen;
        self.packs_return = Some(back);
        self.shader_cache = None;
    }

    /// Whether a pack screen is open.
    #[must_use]
    pub fn in_pack_screen(&self) -> bool {
        matches!(self.screen, Screen::ResourcePacks | Screen::ShaderPack)
    }

    fn leave_packs(&mut self) {
        self.screen = self.packs_return.take().unwrap_or(Screen::Main);
    }

    fn resource_packs(&mut self, ui: &mut Ui) -> MenuAction {
        let mut action = MenuAction::None;
        let dir = self.packs_root.join("resourcepacks");
        let _ = std::fs::create_dir_all(&dir);
        let all = packs::list_resource_packs(&dir);
        ui.label(RichText::new("Resource and texture packs").size(20.0));
        ui.small(format!(
            "Put pack folders or .zip files into {}",
            dir.display()
        ));
        ui.add_space(8.0);
        let d = &mut self.draft;
        d.resource_packs
            .retain(|id| all.iter().any(|p| &p.id == id));
        ui.label(RichText::new("Enabled — the top pack wins").strong());
        let mut swap = None;
        let mut remove = None;
        egui::Grid::new("enabled_packs")
            .num_columns(2)
            .show(ui, |ui| {
                for (i, id) in d.resource_packs.iter().enumerate() {
                    let p = all.iter().find(|p| &p.id == id);
                    ui.label(p.map_or(id.as_str(), |p| p.name.as_str()))
                        .on_hover_text(p.map_or("", |p| p.description.as_str()));
                    ui.horizontal(|ui| {
                        if ui.add_enabled(i > 0, egui::Button::new("↑")).clicked() {
                            swap = Some((i, i - 1));
                        }
                        if ui
                            .add_enabled(i + 1 < d.resource_packs.len(), egui::Button::new("↓"))
                            .clicked()
                        {
                            swap = Some((i, i + 1));
                        }
                        if ui.button("Disable").clicked() {
                            remove = Some(i);
                        }
                    });
                    ui.end_row();
                }
            });
        if let Some((a, b)) = swap {
            d.resource_packs.swap(a, b);
        }
        if let Some(i) = remove {
            d.resource_packs.remove(i);
        }
        ui.add_space(8.0);
        ui.label(RichText::new("Available").strong());
        let mut enable = None;
        for p in all.iter().filter(|p| !d.resource_packs.contains(&p.id)) {
            ui.horizontal(|ui| {
                ui.add_space(ui.available_width() / 2.0 - 160.0);
                ui.label(&p.name).on_hover_text(&p.description);
                if ui.button("Enable").clicked() {
                    enable = Some(p.id.clone());
                }
            });
        }
        if let Some(id) = enable {
            d.resource_packs.insert(0, id);
        }
        if all.is_empty() {
            ui.label("No packs found.");
        }
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.add_space(ui.available_width() / 2.0 - 80.0);
            if ui.button("Done").clicked() {
                action = MenuAction::ApplyPacks(self.draft.clone());
                self.leave_packs();
            }
            if ui.button("Cancel").clicked() {
                self.leave_packs();
            }
        });
        action
    }

    fn shader_pack(&mut self, ui: &mut Ui) -> MenuAction {
        let mut action = MenuAction::None;
        let dir = self.packs_root.join("shaderpacks");
        let _ = std::fs::create_dir_all(&dir);
        let all = packs::list_shader_packs(&dir);
        ui.label(RichText::new("Shader pack").size(20.0));
        ui.small(format!(
            "Put shader pack folders or .zip files into {}",
            dir.display()
        ));
        ui.small("Shader packs run code on your GPU: use packs from people you trust. Start with --safe-mode to turn packs off.");
        ui.add_space(8.0);
        let d = &mut self.draft;
        ui.radio_value(&mut d.shader_pack, None, "None (built-in look)");
        for p in &all {
            ui.radio_value(&mut d.shader_pack, Some(p.id.clone()), &p.name);
        }
        if let Some(id) = d.shader_pack.clone() {
            if self.shader_cache.as_ref().is_none_or(|(c, _)| c != &id) {
                let loaded = all
                    .iter()
                    .find(|p| p.id == id)
                    .ok_or_else(|| format!("{id} is missing"))
                    .and_then(packs::load_shader_pack);
                self.shader_cache = Some((id.clone(), loaded));
            }
            match self.shader_cache.as_ref().map(|(_, l)| l) {
                Some(Ok(pack)) => {
                    if !pack.description.is_empty() {
                        ui.label(&pack.description);
                    }
                    let values = d.shader_options.entry(id).or_default();
                    egui::Grid::new("shader_options")
                        .num_columns(2)
                        .show(ui, |ui| {
                            for o in &pack.options {
                                let v = values.entry(o.id.clone()).or_insert(o.default);
                                ui.label(if o.label.is_empty() { &o.id } else { &o.label });
                                ui.add(egui::Slider::new(v, o.min..=o.max));
                                ui.end_row();
                            }
                        });
                }
                Some(Err(e)) => {
                    ui.colored_label(Color32::from_rgb(255, 160, 120), e);
                }
                None => {}
            }
        }
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.add_space(ui.available_width() / 2.0 - 80.0);
            if ui.button("Done").clicked() {
                action = MenuAction::ApplyPacks(self.draft.clone());
                self.leave_packs();
            }
            if ui.button("Cancel").clicked() {
                self.leave_packs();
            }
        });
        action
    }

    fn about(&mut self, ui: &mut Ui) -> MenuAction {
        ui.label(RichText::new(LEGAL_NOTICE).strong());
        ui.add_space(8.0);
        ui.label("An independent voxel engine written in Rust. All game content is a mod on its public API.");
        ui.label("Textures shown are the engine's own generated fallback textures.");
        ui.label(
            "Offline mode is for development and private tests only; nothing is run publicly.",
        );
        ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
        ui.small(&self.gpu_line);
        ui.add_space(12.0);
        if ui.button("Back").clicked() {
            self.screen = Screen::Main;
        }
        MenuAction::None
    }
}

/// What the pause screen returns.
pub enum PauseAction {
    /// Keep the pause screen.
    None,
    /// Back to the game.
    Resume,
    /// Save, close the world and go to the title screen.
    SaveAndQuit,
    /// Open a pack screen over the paused game.
    Packs(Screen),
}

/// The pause screen over the game.
pub fn pause(ui: &mut Ui) -> PauseAction {
    let mut action = PauseAction::None;
    egui::CentralPanel::no_frame().show(ui, |ui| {
        let rect = ui.max_rect();
        ui.painter()
            .rect_filled(rect, 0.0, Color32::from_black_alpha(140));
        ui.with_layout(Layout::top_down(Align::Center), |ui| {
            ui.add_space(rect.height() / 3.0);
            ui.heading("Paused");
            ui.add_space(12.0);
            if ui
                .add_sized([240.0, 34.0], egui::Button::new("Back to game"))
                .clicked()
            {
                action = PauseAction::Resume;
            }
            if ui
                .add_sized([240.0, 34.0], egui::Button::new("Resource packs…"))
                .clicked()
            {
                action = PauseAction::Packs(Screen::ResourcePacks);
            }
            if ui
                .add_sized([240.0, 34.0], egui::Button::new("Shader pack…"))
                .clicked()
            {
                action = PauseAction::Packs(Screen::ShaderPack);
            }
            if ui
                .add_sized([240.0, 34.0], egui::Button::new("Save and quit to title"))
                .clicked()
            {
                action = PauseAction::SaveAndQuit;
            }
        });
    });
    action
}

/// Crosshair, hotbar and the optional debug text.
pub fn hud(
    ui: &mut Ui,
    hotbar: &[&str],
    selected: usize,
    debug: Option<&str>,
    status: Option<&str>,
    loading: bool,
) {
    egui::CentralPanel::no_frame().show(ui, |ui| {
        let rect = ui.max_rect();
        let c = rect.center();
        let p = ui.painter();
        let stroke = egui::Stroke::new(2.0, Color32::WHITE);
        p.line_segment([c - egui::vec2(8.0, 0.0), c + egui::vec2(8.0, 0.0)], stroke);
        p.line_segment([c - egui::vec2(0.0, 8.0), c + egui::vec2(0.0, 8.0)], stroke);
        let text: Vec<String> = hotbar
            .iter()
            .enumerate()
            .map(|(i, n)| {
                if i == selected {
                    format!("[{} {n}]", i + 1)
                } else {
                    format!("{} {n}", i + 1)
                }
            })
            .collect();
        p.text(
            rect.center_bottom() - egui::vec2(0.0, 16.0),
            egui::Align2::CENTER_BOTTOM,
            text.join("   "),
            egui::FontId::proportional(15.0),
            Color32::WHITE,
        );
        if let Some(s) = status {
            p.text(
                rect.center_bottom() - egui::vec2(0.0, 40.0),
                egui::Align2::CENTER_BOTTOM,
                s,
                egui::FontId::proportional(15.0),
                Color32::from_rgb(255, 240, 160),
            );
        }
        if loading {
            p.text(
                c + egui::vec2(0.0, 40.0),
                egui::Align2::CENTER_CENTER,
                "Loading world…",
                egui::FontId::proportional(22.0),
                Color32::WHITE,
            );
        }
        if let Some(d) = debug {
            p.text(
                rect.left_top() + egui::vec2(8.0, 8.0),
                egui::Align2::LEFT_TOP,
                d,
                egui::FontId::monospace(13.0),
                Color32::WHITE,
            );
        }
    });
}
