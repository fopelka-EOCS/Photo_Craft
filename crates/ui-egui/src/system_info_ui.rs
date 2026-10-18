//! Help › System Info laid out as in Photoshop: the report in a framed, scrolling text box with
//! OK and Copy stacked on its right. The report itself is `gpu_status::system_info_ui`; this
//! module draws it and reads the machine facts it adds (OS version, processor, memory).

use serde_json::{Map, Value};

use crate::PhotocraftApp;

/// The text box: wide enough for Photoshop-length lines, a dozen of them visible before scrolling.
const TEXT_WIDTH: f32 = 470.0;
const TEXT_HEIGHT: f32 = 270.0;
const BUTTON_WIDTH: f32 = 84.0;
/// The whole body: the text box, the gap and the button column.
pub const WIDTH: f32 = TEXT_WIDTH + 12.0 + 10.0 + BUTTON_WIDTH;

/// Is this dialog Help › System Info (an About dialog marked `systemInfo`)?
pub fn owns(f: &Map<String, Value>) -> bool {
    f.get("systemInfo").and_then(Value::as_bool) == Some(true)
}

/// Draw the dialog's body. `Some(false)` when OK (or ↩) closes it; Copy puts the English report
/// on the clipboard and leaves it open.
pub fn body(app: &PhotocraftApp, ui: &mut egui::Ui, interactive: bool, id: egui::Id) -> Option<bool> {
    let t = crate::theme::Tokens::get(ui.ctx());
    let text = crate::gpu_status::system_info_ui(app).join("\n");
    let mut outcome = None;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        egui::Frame::new().fill(t.field).stroke(egui::Stroke::new(1.0, t.accent_border)).corner_radius(t.radius_sm).inner_margin(6).show(ui, |ui| {
            ui.set_width(TEXT_WIDTH);
            egui::ScrollArea::vertical().id_salt(id.with("system-info-text")).max_height(TEXT_HEIGHT).min_scrolled_height(TEXT_HEIGHT).auto_shrink(false).show(
                ui,
                |ui| {
                    // A read-only text field, so a drag selects across lines as in Photoshop.
                    let mut view = text.as_str();
                    ui.add(
                        egui::TextEdit::multiline(&mut view)
                            .font(egui::FontId::proportional(12.0))
                            .frame(egui::Frame::NONE)
                            .margin(0)
                            .desired_width(f32::INFINITY),
                    );
                },
            );
        });
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            if crate::widgets::primary_button(ui, tl!("OK"), BUTTON_WIDTH).clicked() {
                outcome = Some(false);
            }
            if crate::widgets::secondary_button(ui, tl!("Copy"), BUTTON_WIDTH).clicked() {
                ui.ctx().copy_text(crate::gpu_status::system_info(app).join("\n"));
            }
        });
    });
    // ↩ is OK, from the frame after the dialog appeared (the ↩ that opened it mustn't close it).
    let seen = id.with("alert-seen");
    let shown_before = ui.ctx().data(|m| m.get_temp::<bool>(seen).unwrap_or(false));
    ui.ctx().data_mut(|m| m.insert_temp(seen, true));
    if outcome.is_none() && interactive && shown_before && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
        outcome = Some(false);
    }
    outcome
}

/// Facts about the machine that don't change while PhotoCraft runs, read once.
#[derive(Debug, Default)]
pub struct Host {
    /// Operating system name and version ("macOS 26.5.1"), or just the OS family.
    pub os: String,
    /// Processor name as the OS reports it ("Apple M5 Pro").
    pub cpu: Option<String>,
    /// Logical processors (hardware threads) available to PhotoCraft.
    pub threads: Option<usize>,
}

/// The machine facts, read on first use.
pub fn host() -> &'static Host {
    static HOST: std::sync::OnceLock<Host> = std::sync::OnceLock::new();
    HOST.get_or_init(|| Host {
        os: os_version().unwrap_or_else(|| std::env::consts::OS.to_string()),
        cpu: cpu_name().filter(|s| !s.is_empty()),
        threads: if cfg!(target_arch = "wasm32") { None } else { std::thread::available_parallelism().ok().map(|n| n.get()) },
    })
}

/// Memory PhotoCraft's process is using right now (resident set), in bytes.
pub fn process_memory() -> Option<u64> {
    // The tests compare reports made moments apart, so they leave out a number that moves.
    if cfg!(test) { None } else { sample_process_memory() }
}

#[cfg(not(target_arch = "wasm32"))]
fn sample_process_memory() -> Option<u64> {
    memory_stats::memory_stats().and_then(|s| u64::try_from(s.physical_mem).ok())
}

#[cfg(target_arch = "wasm32")]
fn sample_process_memory() -> Option<u64> {
    None
}

/// The first line of a command's output, trimmed; `None` when it can't run or prints nothing.
#[cfg(any(target_os = "macos", target_os = "freebsd"))]
fn command_line(program: &str, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new(program).args(args).output().ok()?;
    let text = String::from_utf8(out.stdout).ok()?;
    Some(text.lines().next()?.trim().to_string()).filter(|s| !s.is_empty())
}

#[cfg(target_os = "macos")]
fn os_version() -> Option<String> {
    command_line("/usr/sbin/sysctl", &["-n", "kern.osproductversion"]).map(|v| format!("macOS {v}"))
}

#[cfg(target_os = "macos")]
fn cpu_name() -> Option<String> {
    command_line("/usr/sbin/sysctl", &["-n", "machdep.cpu.brand_string"])
}

#[cfg(target_os = "linux")]
fn os_version() -> Option<String> {
    let release = std::fs::read_to_string("/etc/os-release").ok()?;
    os_release_name(&release)
}

/// `PRETTY_NAME` from an os-release file ("Ubuntu 24.04.1 LTS"), quotes removed.
#[cfg(any(target_os = "linux", test))]
fn os_release_name(release: &str) -> Option<String> {
    let value = release.lines().find_map(|l| l.strip_prefix("PRETTY_NAME="))?.trim();
    Some(value.trim_matches(|c| c == '"' || c == '\'').to_string()).filter(|s| !s.is_empty())
}

#[cfg(target_os = "linux")]
fn cpu_name() -> Option<String> {
    let info = std::fs::read_to_string("/proc/cpuinfo").ok()?;
    cpuinfo_name(&info)
}

/// The processor name in /proc/cpuinfo: x86's `model name`, else ARM's `Model` (Raspberry Pi).
#[cfg(any(target_os = "linux", test))]
fn cpuinfo_name(info: &str) -> Option<String> {
    let field = |key: &str| {
        info.lines().find_map(|l| {
            let (k, v) = l.split_once(':')?;
            (k.trim() == key).then(|| v.trim().to_string()).filter(|v| !v.is_empty())
        })
    };
    field("model name").or_else(|| field("Model"))
}

#[cfg(target_os = "freebsd")]
fn os_version() -> Option<String> {
    command_line("/usr/bin/uname", &["-r"]).map(|v| format!("FreeBSD {v}"))
}

#[cfg(target_os = "freebsd")]
fn cpu_name() -> Option<String> {
    command_line("/sbin/sysctl", &["-n", "hw.model"])
}

#[cfg(windows)]
fn os_version() -> Option<String> {
    Some("Windows".into())
}

#[cfg(windows)]
fn cpu_name() -> Option<String> {
    std::env::var("PROCESSOR_IDENTIFIER").ok().map(|s| s.trim().to_string())
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "freebsd", windows)))]
fn os_version() -> Option<String> {
    None
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "freebsd", windows)))]
fn cpu_name() -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn os_release_pretty_name_without_quotes() {
        let release = "NAME=\"Ubuntu\"\nPRETTY_NAME=\"Ubuntu 24.04.1 LTS\"\nID=ubuntu\n";
        assert_eq!(os_release_name(release).as_deref(), Some("Ubuntu 24.04.1 LTS"));
        assert_eq!(os_release_name("PRETTY_NAME='Arch Linux'").as_deref(), Some("Arch Linux"));
        assert_eq!(os_release_name("PRETTY_NAME=\"\"\n"), None);
        assert_eq!(os_release_name("NAME=Debian\n"), None);
    }

    #[test]
    fn cpuinfo_name_on_x86_and_arm() {
        let x86 = "processor\t: 0\nvendor_id\t: GenuineIntel\nmodel name\t: Intel(R) Core(TM) i7-8700 CPU @ 3.20GHz\n";
        assert_eq!(cpuinfo_name(x86).as_deref(), Some("Intel(R) Core(TM) i7-8700 CPU @ 3.20GHz"));
        let pi = "processor\t: 0\nBogoMIPS\t: 108.00\nModel\t\t: Raspberry Pi 5 Model B Rev 1.0\n";
        assert_eq!(cpuinfo_name(pi).as_deref(), Some("Raspberry Pi 5 Model B Rev 1.0"));
        assert_eq!(cpuinfo_name("garbage without fields"), None);
        assert_eq!(cpuinfo_name("model name\t:\n"), None);
    }

    #[test]
    fn host_names_an_os_and_counts_processors() {
        let h = host();
        assert!(!h.os.is_empty());
        if cfg!(not(target_arch = "wasm32")) {
            assert!(h.threads.is_some_and(|n| n >= 1));
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn samples_this_process_memory() {
        if cfg!(any(target_os = "macos", target_os = "linux", windows)) {
            assert!(sample_process_memory().is_some_and(|b| b > 1 << 20), "a running test uses more than 1 MB");
        }
    }
}
