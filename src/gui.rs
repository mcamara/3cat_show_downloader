//! Native graphical launcher for the downloader CLI.

use std::io::{BufRead, BufReader, Read};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use eframe::egui;

/// Opens the native downloader window.
///
/// # Errors
///
/// Returns an error when the operating system cannot create the GUI window.
pub fn launch() -> anyhow::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([760.0, 720.0])
            .with_min_inner_size([620.0, 560.0]),
        ..Default::default()
    };
    eframe::run_native(
        "3Cat Show Downloader",
        options,
        Box::new(|_| Ok(Box::<DownloaderGui>::default())),
    )
    .map_err(|error| anyhow::anyhow!(error.to_string()))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum RepairMode {
    #[default]
    Download,
    Plan,
    Apply,
}

#[derive(Clone, Debug)]
struct GuiInputs {
    slug: String,
    directory: String,
    start_from_episode: i32,
    concurrent_downloads: u8,
    skip_subtitles: bool,
    strict_subtitles: bool,
    auto_naming: bool,
    season: u32,
    reencode: String,
    request_delay_ms: u64,
    plex_metadata: bool,
    tvdb_series_id: String,
    repair_mode: RepairMode,
}

impl Default for GuiInputs {
    fn default() -> Self {
        Self {
            slug: String::new(),
            directory: String::new(),
            start_from_episode: 1,
            concurrent_downloads: 2,
            skip_subtitles: false,
            strict_subtitles: false,
            auto_naming: true,
            season: 1,
            reencode: "off".into(),
            request_delay_ms: 1500,
            plex_metadata: false,
            tvdb_series_id: String::new(),
            repair_mode: RepairMode::Download,
        }
    }
}

impl GuiInputs {
    fn command_args(&self) -> Result<Vec<String>, String> {
        let slug = self.slug.trim();
        if slug.is_empty() {
            return Err("Escriu el slug del programa (per exemple: mic).".into());
        }
        let directory = self.directory.trim();
        if directory.is_empty() {
            return Err("Selecciona una carpeta de destinació.".into());
        }

        let mut args = vec![
            slug.to_owned(),
            "--directory".into(),
            directory.to_owned(),
            "--start-from-episode".into(),
            self.start_from_episode.max(1).to_string(),
            "--concurrent-downloads".into(),
            self.concurrent_downloads.clamp(1, 10).to_string(),
            "--season".into(),
            self.season.max(1).to_string(),
            "--reencode".into(),
            self.reencode.clone(),
            "--request-delay-ms".into(),
            self.request_delay_ms.to_string(),
        ];
        if self.skip_subtitles {
            args.push("--skip-subtitles".into());
        }
        if self.strict_subtitles {
            args.push("--strict-subtitles".into());
        }
        if self.auto_naming {
            args.push("--auto-naming".into());
        }
        if self.plex_metadata {
            let tvdb_id = self
                .tvdb_series_id
                .trim()
                .parse::<u32>()
                .map_err(|_| "L'ID TVDB ha de ser un número vàlid.".to_string())?;
            if tvdb_id == 0 {
                return Err("L'ID TVDB ha de ser superior a zero.".into());
            }
            args.extend([
                "--plex-metadata".into(),
                "--tvdb-series-id".into(),
                tvdb_id.to_string(),
            ]);
            match self.repair_mode {
                RepairMode::Download => {}
                RepairMode::Plan => args.extend(["--repair-existing".into(), "plan".into()]),
                RepairMode::Apply => args.extend(["--repair-existing".into(), "apply".into()]),
            }
        }
        Ok(args)
    }
}

#[derive(Debug)]
enum GuiEvent {
    Output(String),
    Finished(Result<(), String>),
}

#[derive(Default)]
struct DownloaderGui {
    inputs: GuiInputs,
    output: String,
    running: bool,
    events: Option<Receiver<GuiEvent>>,
}

impl DownloaderGui {
    fn start(&mut self) {
        let args = match self.inputs.command_args() {
            Ok(args) => args,
            Err(error) => {
                self.output = format!("ERROR: {error}\n");
                return;
            }
        };
        let executable = match std::env::current_exe() {
            Ok(path) => path,
            Err(error) => {
                self.output = format!("ERROR: no es pot localitzar l'executable: {error}\n");
                return;
            }
        };
        let (sender, receiver) = mpsc::channel();
        self.events = Some(receiver);
        self.running = true;
        self.output.clear();
        self.output.push_str(&format!(
            "> {} {}\n\n",
            executable.display(),
            args.join(" ")
        ));

        std::thread::spawn(move || run_cli(executable, args, sender));
    }

    fn receive_events(&mut self) {
        let Some(receiver) = &self.events else {
            return;
        };
        while let Ok(event) = receiver.try_recv() {
            match event {
                GuiEvent::Output(line) => {
                    self.output.push_str(&line);
                    self.output.push('\n');
                }
                GuiEvent::Finished(result) => {
                    self.running = false;
                    match result {
                        Ok(()) => self.output.push_str("\nCompletat correctament.\n"),
                        Err(error) => self.output.push_str(&format!("\nERROR: {error}\n")),
                    }
                }
            }
        }
    }
}

impl eframe::App for DownloaderGui {
    fn ui(&mut self, root_ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.receive_events();
        if self.running {
            root_ui
                .ctx()
                .request_repaint_after(Duration::from_millis(100));
        }

        egui::CentralPanel::default().show(root_ui, |ui| {
            ui.heading("3Cat Show Downloader");
            ui.label("Descarrega programes de 3Cat i prepara'ls per a Plex.");
            ui.separator();

            egui::Grid::new("download_options")
                .num_columns(2)
                .spacing([16.0, 8.0])
                .show(ui, |ui| {
                    ui.label("Programa (slug)");
                    ui.text_edit_singleline(&mut self.inputs.slug)
                        .on_hover_text("Part final de l'URL de 3Cat, per exemple: mic");
                    ui.end_row();

                    ui.label("Carpeta de destinació");
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.inputs.directory)
                                .desired_width(390.0),
                        );
                        if ui.button("Selecciona...").clicked()
                            && let Some(path) = rfd::FileDialog::new().pick_folder()
                        {
                            self.inputs.directory = path.display().to_string();
                        }
                    });
                    ui.end_row();

                    ui.label("Comença pel capítol");
                    ui.add(
                        egui::DragValue::new(&mut self.inputs.start_from_episode).range(1..=99999),
                    );
                    ui.end_row();

                    ui.label("Descàrregues simultànies");
                    ui.add(egui::Slider::new(
                        &mut self.inputs.concurrent_downloads,
                        1..=10,
                    ));
                    ui.end_row();

                    ui.label("Temporada");
                    ui.add(egui::DragValue::new(&mut self.inputs.season).range(1..=999));
                    ui.end_row();

                    ui.label("Recompressió");
                    egui::ComboBox::from_id_salt("reencode")
                        .selected_text(&self.inputs.reencode)
                        .show_ui(ui, |ui| {
                            for preset in ["off", "light", "balanced", "max"] {
                                ui.selectable_value(
                                    &mut self.inputs.reencode,
                                    preset.to_owned(),
                                    preset,
                                );
                            }
                        });
                    ui.end_row();

                    ui.label("Retard entre peticions (ms)");
                    ui.add(
                        egui::DragValue::new(&mut self.inputs.request_delay_ms).range(0..=60_000),
                    );
                    ui.end_row();
                });

            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut self.inputs.auto_naming, "Noms i carpetes automàtics");
                ui.checkbox(&mut self.inputs.skip_subtitles, "Sense subtítols");
                ui.checkbox(&mut self.inputs.strict_subtitles, "Subtítols estrictes");
            });

            ui.separator();
            ui.checkbox(
                &mut self.inputs.plex_metadata,
                "Preparar metadades per a Plex",
            );
            ui.add_enabled_ui(self.inputs.plex_metadata, |ui| {
                ui.horizontal(|ui| {
                    ui.label("ID de sèrie TVDB");
                    ui.text_edit_singleline(&mut self.inputs.tvdb_series_id);
                    ui.label("(actualment obligatori)");
                });
                ui.horizontal(|ui| {
                    ui.label("Operació");
                    ui.selectable_value(
                        &mut self.inputs.repair_mode,
                        RepairMode::Download,
                        "Descarrega",
                    );
                    ui.selectable_value(
                        &mut self.inputs.repair_mode,
                        RepairMode::Plan,
                        "Planifica reparació",
                    );
                    ui.selectable_value(
                        &mut self.inputs.repair_mode,
                        RepairMode::Apply,
                        "Aplica reparació",
                    );
                });
            });

            ui.separator();
            let button = ui.add_enabled(
                !self.running,
                egui::Button::new(if self.running {
                    "Treballant..."
                } else {
                    "Inicia"
                }),
            );
            if button.clicked() {
                self.start();
            }

            ui.label("Sortida");
            egui::ScrollArea::vertical()
                .stick_to_bottom(true)
                .max_height(260.0)
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut self.output)
                            .font(egui::TextStyle::Monospace)
                            .desired_width(f32::INFINITY)
                            .desired_rows(11)
                            .interactive(false),
                    );
                });
        });
    }
}

fn run_cli(executable: std::path::PathBuf, args: Vec<String>, sender: Sender<GuiEvent>) {
    let result = (|| -> Result<(), String> {
        let mut child = Command::new(executable)
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("no es pot iniciar la descàrrega: {error}"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "no es pot capturar stdout".to_string())?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| "no es pot capturar stderr".to_string())?;
        let stdout_sender = sender.clone();
        let stderr_sender = sender.clone();
        let stdout_thread = std::thread::spawn(move || forward_lines(stdout, stdout_sender));
        let stderr_thread = std::thread::spawn(move || forward_lines(stderr, stderr_sender));
        let status = child
            .wait()
            .map_err(|error| format!("error esperant el procés: {error}"))?;
        let _ = stdout_thread.join();
        let _ = stderr_thread.join();
        if status.success() {
            Ok(())
        } else {
            Err(format!("el procés ha acabat amb {status}"))
        }
    })();
    let _ = sender.send(GuiEvent::Finished(result));
}

fn forward_lines(reader: impl Read, sender: Sender<GuiEvent>) {
    for line in BufReader::new(reader).lines().map_while(Result::ok) {
        let _ = sender.send(GuiEvent::Output(line.trim_end_matches('\r').to_owned()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_standard_download_arguments() {
        let inputs = GuiInputs {
            slug: "mic".into(),
            directory: "C:\\Videos".into(),
            ..Default::default()
        };
        let args = inputs.command_args().unwrap();
        assert_eq!(args[0], "mic");
        assert!(
            args.windows(2)
                .any(|pair| pair == ["--directory", "C:\\Videos"])
        );
        assert!(args.iter().any(|arg| arg == "--auto-naming"));
        assert!(!args.iter().any(|arg| arg == "--plex-metadata"));
    }

    #[test]
    fn plex_download_requires_and_forwards_tvdb_id() {
        let inputs = GuiInputs {
            slug: "mic".into(),
            directory: "C:\\Videos".into(),
            plex_metadata: true,
            tvdb_series_id: "280190".into(),
            repair_mode: RepairMode::Plan,
            ..Default::default()
        };
        let args = inputs.command_args().unwrap();
        assert!(
            args.windows(2)
                .any(|pair| pair == ["--tvdb-series-id", "280190"])
        );
        assert!(
            args.windows(2)
                .any(|pair| pair == ["--repair-existing", "plan"])
        );
    }

    #[test]
    fn rejects_missing_required_fields() {
        assert!(GuiInputs::default().command_args().is_err());
    }
}
