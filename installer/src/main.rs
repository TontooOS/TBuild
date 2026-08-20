use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Cursor;
use std::path::Path;
use std::process::Command;

use uikit::prelude::*;
use uikit::widget::WidgetId;

use gtk::prelude::*;

const MAGIC: &[u8; 8] = b"TONTINST";
const SYSTEM_INSTALL_ARG: &str = "--install-system";

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

fn extract_to(payload: &Payload, target: &Path) -> Result<(), String> {
    std::fs::create_dir_all(target).map_err(|e| format!("cannot create '{}': {}", target.display(), e))?;
    let cursor = Cursor::new(&payload.app);
    let mut archive = zip::ZipArchive::new(cursor).map_err(|e| format!("bad .app payload: {}", e))?;
    archive.extract(target).map_err(|e| format!("extract failed: {}", e))?;
    Ok(())
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

    fn option_button(&self, key: &str, selected: bool) -> Button {
        let label = t(&self.lang, key, &[("user", &username())]);
        let mut btn = Button::new(label).corner_radius(10.0);
        if selected {
            btn = btn.background(Color::from_hex("#FF6B2B").unwrap()).text_color(Color::WHITE);
        } else {
            btn = btn
                .background(Color::new(0.25, 0.25, 0.27, 1.0))
                .text_color(Color::new(0.92, 0.92, 0.94, 1.0));
        }
        btn
    }

    fn build_view(&self) -> Box<dyn Widget> {
        match self.screen {
            Screen::Destination => self.build_destination(),
            Screen::License => self.build_license(),
            Screen::Install => self.build_install(),
            Screen::Done => self.build_done(),
        }
    }

    fn build_destination(&self) -> Box<dyn Widget> {
        let title = Text::new(t(&self.lang, "where.title", &[]))
            .font_size(22.0)
            .bold()
            .color(Color::new(0.95, 0.95, 0.96, 1.0));

        let all_btn = self
            .option_button("option.all", self.all_users)
            .on_custom("select_all");
        let self_btn = self
            .option_button("option.self", !self.all_users)
            .on_custom("select_self");

        let next_btn = Button::new(self.next())
            .background(Color::from_hex("#FF6B2B").unwrap())
            .text_color(Color::WHITE)
            .corner_radius(10.0)
            .padding(28.0, 10.0)
            .on_custom("next");

        Box::new(
            VStack::new()
                .spacing(14.0)
                .alignment(HAlignment::Center)
                .child(title)
                .child(Text::new("").height(8.0))
                .child(all_btn)
                .child(self_btn)
                .child(Text::new("").height(12.0))
                .child(next_btn),
        )
    }

    fn build_license(&self) -> Box<dyn Widget> {
        let title = Text::new(t(&self.lang, "license.title", &[]))
            .font_size(22.0)
            .bold()
            .color(Color::new(0.95, 0.95, 0.96, 1.0));

        let license_text = self
            .payload
            .license
            .clone()
            .unwrap_or_default();

        let accept_btn = Button::new(t(&self.lang, "button.accept", &[]))
            .background(Color::from_hex("#FF6B2B").unwrap())
            .text_color(Color::WHITE)
            .corner_radius(10.0)
            .padding(28.0, 10.0)
            .on_custom("next");
        let back_btn = Button::new(self.back())
            .background(Color::new(0.25, 0.25, 0.27, 1.0))
            .text_color(Color::new(0.92, 0.92, 0.94, 1.0))
            .corner_radius(10.0)
            .padding(28.0, 10.0)
            .on_custom("back");

        Box::new(
            VStack::new()
                .spacing(14.0)
                .alignment(HAlignment::Center)
                .child(title)
                .child(LicenseScroll(license_text))
                .child(
                    HStack::new()
                        .spacing(12.0)
                        .child(back_btn)
                        .child(accept_btn),
                ),
        )
    }

    fn build_install(&self) -> Box<dyn Widget> {
        let title = Text::new(t(
            &self.lang,
            "install.title",
            &[("name", &self.payload.footer.name)],
        ))
        .font_size(22.0)
        .bold()
        .color(Color::new(0.95, 0.95, 0.96, 1.0));

        let install_btn = Button::new(t(&self.lang, "button.install", &[]))
            .background(Color::from_hex("#FF6B2B").unwrap())
            .text_color(Color::WHITE)
            .corner_radius(12.0)
            .padding(48.0, 16.0)
            .on_custom("install");
        let back_btn = Button::new(self.back())
            .background(Color::new(0.25, 0.25, 0.27, 1.0))
            .text_color(Color::new(0.92, 0.92, 0.94, 1.0))
            .corner_radius(10.0)
            .padding(28.0, 10.0)
            .on_custom("back");

        Box::new(
            VStack::new()
                .spacing(24.0)
                .alignment(HAlignment::Center)
                .child(title)
                .child(install_btn)
                .child(back_btn),
        )
    }

    fn build_done(&self) -> Box<dyn Widget> {
        let message = match &self.result {
            Ok(target) => t(
                &self.lang,
                "done.success",
                &[("name", &self.payload.footer.name), ("target", target)],
            ),
            Err(err) => t(&self.lang, "done.error", &[("error", err)]),
        };
        let text = Text::new(message)
            .font_size(16.0)
            .max_width(440.0)
            .color(Color::new(0.92, 0.92, 0.94, 1.0));

        let close_btn = Button::new(t(&self.lang, "button.close", &[]))
            .background(Color::from_hex("#FF6B2B").unwrap())
            .text_color(Color::WHITE)
            .corner_radius(10.0)
            .padding(28.0, 10.0)
            .on_custom("__close");

        Box::new(
            VStack::new()
                .spacing(20.0)
                .alignment(HAlignment::Center)
                .child(text)
                .child(close_btn),
        )
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
                Ok("/Applications".to_string())
            } else {
                Err("pkexec failed".to_string())
            }
        } else {
            let target = format!("/Users/{}/Applications", username());
            extract_to(&self.payload, Path::new(&target))?;
            Ok(target)
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

struct LicenseScroll(String);

impl Widget for LicenseScroll {
    fn id(&self) -> WidgetId {
        0
    }

    fn to_gtk(&self) -> gtk::Widget {
        let scrolled = gtk::ScrolledWindow::new();
        scrolled.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Automatic);
        scrolled.set_width_request(460);
        scrolled.set_height_request(320);
        scrolled.set_hexpand(true);

        let label = gtk::Label::new(Some(&self.0));
        label.set_wrap(true);
        label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        label.set_xalign(0.0);
        label.set_yalign(0.0);
        label.set_halign(gtk::Align::Fill);
        label.set_valign(gtk::Align::Start);
        label.set_margin_start(8);
        label.set_margin_end(8);
        scrolled.set_child(Some(&label));

        scrolled.upcast()
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == SYSTEM_INSTALL_ARG) {
        match read_payload().and_then(|payload| extract_to(&payload, Path::new("/Applications"))) {
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

    let mut app = App::with_delegate(title, 540, 560, delegate);
    app.auto_color_scheme();
    app.set_glass(0.25, 0.65, 20.0);
    app.run();
}