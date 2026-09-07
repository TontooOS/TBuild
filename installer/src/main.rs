use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::Command;

use tontooui::prelude::*;

const MAGIC: &[u8; 8] = b"TONTINST";
const SYSTEM_INSTALL_ARG: &str = "--install-system";
const LICENSE_WRAP_CHARS: usize = 72;

#[derive(Serialize, Deserialize)]
struct Footer {
    magic: String,
    app_size: u64,
    license_size: u64,
    name: String,
    bundle_id: String,
    version: String,
}

struct Payload {
    footer: Footer,
    app: Vec<u8>,
    license: Option<String>,
}

fn read_payload() -> Result<Payload, String> {
    let exe = std::fs::read("/proc/self/exe").map_err(|e| format!("cannot read self: {}", e))?;
    if exe.len() < 12 + 8 {
        return Err("file too small to be a .tinstaller".to_string());
    }
    if &exe[exe.len() - 8..] != MAGIC {
        return Err("not a .tinstaller (missing magic)".to_string());
    }
    let footer_len =
        u32::from_le_bytes(exe[exe.len() - 12..exe.len() - 8].try_into().unwrap()) as usize;
    if exe.len() < 12 + footer_len {
        return Err("corrupt .tinstaller (bad footer length)".to_string());
    }
    let footer_bytes = &exe[exe.len() - 12 - footer_len..exe.len() - 12];
    let footer: Footer =
        serde_json::from_slice(footer_bytes).map_err(|e| format!("bad footer: {}", e))?;
    if footer.magic != "tontinstaller" {
        return Err("not a .tinstaller (bad footer magic)".to_string());
    }
    let lic_end = exe.len() - 12 - footer_len;
    let lic_start = lic_end - footer.license_size as usize;
    let app_start = lic_start - footer.app_size as usize;
    let app = exe[app_start..lic_start].to_vec();
    let license = if footer.license_size > 0 {
        Some(String::from_utf8_lossy(&exe[lic_start..lic_end]).to_string())
    } else {
        None
    };
    Ok(Payload { footer, app, license })
}

/// Installs the bundled `.app` as a directory bundle below `target`.
/// The ZIP is extracted into a hidden staging directory first and renamed
/// into place, so a crash never leaves a half-installed bundle behind.
fn install_bundle(payload: &Payload, target: &Path) -> Result<PathBuf, String> {
    std::fs::create_dir_all(target)
        .map_err(|e| format!("cannot create '{}': {}", target.display(), e))?;

    let staging = target.join(format!(
        ".{}.install-{}",
        payload.footer.name,
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging)
        .map_err(|e| format!("cannot create '{}': {}", staging.display(), e))?;

    let cursor = Cursor::new(&payload.app);
    let mut archive =
        zip::ZipArchive::new(cursor).map_err(|e| format!("bad .app payload: {}", e))?;
    archive
        .extract(&staging)
        .map_err(|e| format!("extract failed: {}", e))?;

    // TBuild zips contain a single top-level "<Name>.app" directory.
    let src = staging.join(format!("{}.app", payload.footer.name));
    let src = if src.is_dir() { src } else { staging.clone() };

    let dest = target.join(format!("{}.app", payload.footer.name));
    if dest.is_dir() {
        std::fs::remove_dir_all(&dest)
            .map_err(|e| format!("cannot replace '{}': {}", dest.display(), e))?;
    } else if dest.is_file() {
        // Clean up single-file (zipped) installs from older versions.
        std::fs::remove_file(&dest)
            .map_err(|e| format!("cannot replace '{}': {}", dest.display(), e))?;
    }

    std::fs::rename(&src, &dest)
        .map_err(|e| format!("cannot finalize '{}': {}", dest.display(), e))?;
    let _ = std::fs::remove_dir_all(&staging);
    Ok(dest)
}

fn detect_locale() -> &'static str {
    let lang = std::env::var("LANG")
        .or_else(|_| std::env::var("LC_ALL"))
        .unwrap_or_default();
    let lang = if !lang.is_empty() {
        lang
    } else {
        std::fs::read_to_string("/etc/locale.conf")
            .ok()
            .and_then(|c| {
                c.lines()
                    .find(|l| l.starts_with("LANG="))
                    .map(|l| l.trim_start_matches("LANG=").to_string())
            })
            .unwrap_or_default()
    };
    if lang.to_ascii_lowercase().starts_with("de") {
        "de_de"
    } else {
        "en_us"
    }
}

fn load_lang() -> HashMap<String, String> {
    let raw = match detect_locale() {
        "de_de" => include_str!("../lang/de_de.json"),
        _ => include_str!("../lang/en_us.json"),
    };
    serde_json::from_str(raw).unwrap_or_default()
}

fn t(lang: &HashMap<String, String>, key: &str, vars: &[(&str, &str)]) -> String {
    let template = lang.get(key).map(String::as_str).unwrap_or(key);
    let mut s = template.to_string();
    for (k, v) in vars {
        s = s.replace(&format!("{{{}}}", k), v);
    }
    s
}

fn username() -> String {
    std::env::var("USER").unwrap_or_else(|_| "user".to_string())
}

fn detect_scheme() -> ColorScheme {
    let config_dir = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_default();
            PathBuf::from(home).join(".config")
        });
    let theme_file = config_dir.join("tontoo").join("theme.conf");
    if let Ok(content) = std::fs::read_to_string(&theme_file) {
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((key, value)) = line.split_once('=') {
                if key.trim() == "color-scheme" && value.trim() == "light" {
                    return ColorScheme::Light;
                }
            }
        }
    }
    ColorScheme::Dark
}

/// Greedy word wrap so the plain-text license fits the scroll view width.
fn wrap_text(text: &str, max_chars: usize) -> String {
    let mut result = String::new();
    for line in text.lines() {
        let mut current = String::new();
        for word in line.split_whitespace() {
            loop {
                let fits = current.chars().count() + 1 + word.chars().count() <= max_chars;
                if fits || current.is_empty() {
                    break;
                }
                result.push_str(&current);
                result.push('\n');
                current.clear();
            }
            if word.chars().count() > max_chars {
                if !current.is_empty() {
                    result.push_str(&current);
                    result.push('\n');
                    current.clear();
                }
                let chars: Vec<char> = word.chars().collect();
                for chunk in chars.chunks(max_chars) {
                    result.extend(chunk);
                    result.push('\n');
                }
                continue;
            }
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(word);
        }
        result.push_str(&current);
        result.push('\n');
    }
    result
}

enum Screen {
    Destination,
    License,
    Install,
    Done,
}

struct InstallerDelegate {
    payload: Payload,
    lang: HashMap<String, String>,
    screen: Screen,
    all_users: bool,
    result: Result<String, String>,
}

impl InstallerDelegate {
    fn next(&self) -> String {
        t(&self.lang, "button.next", &[])
    }

    fn back(&self) -> String {
        t(&self.lang, "button.back", &[])
    }

    fn accent(&self) -> Color {
        Color::from_hex("#FF6B2B").unwrap_or(Color::ACCENT)
    }

    fn secondary(&self) -> Color {
        Color::new(0.25, 0.25, 0.27, 1.0)
    }

    fn foreground(&self) -> Color {
        Color::new(0.92, 0.92, 0.94, 1.0)
    }

    fn title_text(&self, content: String) -> Text {
        Text::new(content)
            .font_size(22.0)
            .bold()
            .color(Color::new(0.95, 0.95, 0.96, 1.0))
    }

    fn primary_button(&self, label: String, action: &str) -> Button {
        Button::new(label)
            .background(self.accent())
            .text_color(Color::WHITE)
            .corner_radius(10.0)
            .padding(28.0, 10.0)
            .on_custom(action)
    }

    fn back_button(&self) -> Button {
        Button::new(self.back())
            .background(self.secondary())
            .text_color(self.foreground())
            .corner_radius(10.0)
            .padding(28.0, 10.0)
            .on_custom("back")
    }

    fn option_button(&self, label: String, selected: bool, action: &str) -> Button {
        Button::new(label)
            .background(if selected {
                self.accent()
            } else {
                self.secondary()
            })
            .text_color(if selected {
                Color::WHITE
            } else {
                self.foreground()
            })
            .corner_radius(10.0)
            .padding(24.0, 12.0)
            .width(320.0)
            .on_custom(action)
    }

    fn gap(&self, height: f32) -> Frame {
        Frame::new().height(height).child(Text::new(""))
    }

    fn build_view(&self) -> Box<dyn Widget> {
        let screen = match self.screen {
            Screen::Destination => self.build_destination(),
            Screen::License => self.build_license(),
            Screen::Install => self.build_install(),
            Screen::Done => self.build_done(),
        };
        Box::new(
            PaddingWrap::new(Padding::new(36.0, 0.0, 0.0, 0.0))
                .child(Frame::new().size(720.0, 680.0).child(screen)),
        )
    }

    fn build_destination(&self) -> VStack {
        VStack::new()
            .spacing(16.0)
            .alignment(HAlignment::Center)
            .child(self.title_text(t(&self.lang, "where.title", &[])))
            .child(self.gap(12.0))
            .child(self.option_button(
                t(&self.lang, "option.all", &[]),
                self.all_users,
                "select_all",
            ))
            .child(self.option_button(
                t(&self.lang, "option.self", &[("user", &username())]),
                !self.all_users,
                "select_self",
            ))
            .child(self.gap(24.0))
            .child(self.primary_button(self.next(), "next"))
    }

    fn build_license(&self) -> VStack {
        let license_text = self.payload.license.clone().unwrap_or_default();
        let scroll = ScrollView::new().content(
            View::new(Label::new(wrap_text(&license_text, LICENSE_WRAP_CHARS)).font_size(13.0)),
        );

        VStack::new()
            .spacing(16.0)
            .alignment(HAlignment::Center)
            .child(self.title_text(t(&self.lang, "license.title", &[])))
            .child(Frame::new().size(640.0, 400.0).child(scroll))
            .child(self.back_button())
            .child(self.primary_button(t(&self.lang, "button.accept", &[]), "next"))
    }

    fn build_install(&self) -> VStack {
        VStack::new()
            .spacing(16.0)
            .alignment(HAlignment::Center)
            .child(self.title_text(t(
                &self.lang,
                "install.title",
                &[("name", &self.payload.footer.name)],
            )))
            .child(Text::new(format!(
                "{} {}",
                self.payload.footer.name, self.payload.footer.version
            ))
            .font_size(15.0)
            .color(self.foreground()))
            .child(self.gap(24.0))
            .child(self.primary_button(t(&self.lang, "button.install", &[]), "install"))
            .child(self.back_button())
    }

    fn build_done(&self) -> VStack {
        let message = match &self.result {
            Ok(target) => t(
                &self.lang,
                "done.success",
                &[("name", &self.payload.footer.name), ("target", target)],
            ),
            Err(err) => t(&self.lang, "done.error", &[("error", err)]),
        };

        VStack::new()
            .spacing(16.0)
            .alignment(HAlignment::Center)
            .child(
                Text::new(message)
                    .font_size(16.0)
                    .max_width(640.0)
                    .color(self.foreground()),
            )
            .child(self.gap(24.0))
            .child(self.primary_button(t(&self.lang, "button.close", &[]), "__close"))
    }

    fn do_install(&self) -> Result<String, String> {
        if self.all_users {
            let exe = std::fs::read_link("/proc/self/exe")
                .map_err(|e| format!("cannot resolve self: {}", e))?;
            let status = Command::new("pkexec")
                .arg(&exe)
                .arg(SYSTEM_INSTALL_ARG)
                .status()
                .map_err(|e| format!("cannot run pkexec: {}", e))?;
            if status.success() {
                Ok(format!("/Applications/{}.app", self.payload.footer.name))
            } else {
                Err("pkexec failed".to_string())
            }
        } else {
            let target = format!("/Users/{}/Applications", username());
            let dest = install_bundle(&self.payload, Path::new(&target))?;
            Ok(dest.to_string_lossy().to_string())
        }
    }
}

impl AppDelegate for InstallerDelegate {
    fn view(&self) -> Box<dyn Widget> {
        self.build_view()
    }

    fn handle_custom(&mut self, action: &str) {
        match action {
            "select_all" => self.all_users = true,
            "select_self" => self.all_users = false,
            "next" => match self.screen {
                Screen::Destination => {
                    self.screen = if self.payload.license.is_some() {
                        Screen::License
                    } else {
                        Screen::Install
                    };
                }
                Screen::License => self.screen = Screen::Install,
                _ => {}
            },
            "back" => match self.screen {
                Screen::License => self.screen = Screen::Destination,
                Screen::Install => {
                    self.screen = if self.payload.license.is_some() {
                        Screen::License
                    } else {
                        Screen::Destination
                    };
                }
                _ => {}
            },
            "install" => {
                self.result = self.do_install();
                self.screen = Screen::Done;
            }
            _ => {}
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == SYSTEM_INSTALL_ARG) {
        match read_payload().and_then(|payload| {
            install_bundle(&payload, Path::new("/Applications")).map(|_| ())
        }) {
            Ok(()) => std::process::exit(0),
            Err(err) => {
                eprintln!("error: {}", err);
                std::process::exit(1);
            }
        }
    }

    let payload = match read_payload() {
        Ok(payload) => payload,
        Err(err) => {
            eprintln!("error: {}", err);
            std::process::exit(1);
        }
    };
    let lang = load_lang();
    let title = t(&lang, "window.title", &[]);

    let delegate = InstallerDelegate {
        payload,
        lang,
        screen: Screen::Destination,
        all_users: false,
        result: Err("".to_string()),
    };

    let mut app = App::with_delegate(title, 720, 680, delegate);
    app.set_color_scheme(detect_scheme());
    app.set_glass(0.25, 0.65, 20.0);
    app.run();
}
