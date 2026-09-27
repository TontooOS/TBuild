//! Tontoo Installer: UI for `.tinstaller` self-extracting bundles.
//!
//! The running binary carries `[.app TAPP container][license][footer JSON]`
//! appended after its own ELF. Screens (destination, license, install,
//! done) are TontooUI on Vello/WGPU; the payload is extracted with
//! ArchiveKit (`AppReader::extract_to`) into a staging dir and renamed
//! into place, so a crash never leaves a half-installed bundle behind.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;

sdk::preinclude!();

use TontooUI::elements::{
  Align, BasicText, Button, ButtonStyle, FormattedText, ScrollView, Spacer,
  Span, TextAlignment, TextStyle, Titlebar, TrafficAction, View, VStack,
};
use TontooUI::renderer::window::{App, Key, Viewport, WindowCommand, run};
use TontooUI::renderer::{FontSystem, ImageLoader};
use TontooUI::theme::{ThemeMode, ThemeWatcher};
use vello::Scene;
use vello::peniko::Color;

use crate::ArchiveKit::AppReader;
use crate::Foundation::serialization::{JSONSerialization, JsonDocument};

const MAGIC: &[u8; 8] = b"TONTINST";
const SYSTEM_INSTALL_ARG: &str = "--install-system";
const LICENSE_WRAP_CHARS: usize = 72;
const ACCENT: Color = Color::from_rgb8(0xff, 0x6b, 0x2b);

struct Footer {
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
  let exe = std::fs::read("/proc/self/exe").map_err(|e| format!("cannot read self: {e}"))?;
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
  let footer_text =
    std::str::from_utf8(footer_bytes).map_err(|_| "bad footer (not utf-8)".to_string())?;
  let footer_doc =
    JsonDocument::parse(footer_text).map_err(|e| format!("bad footer: {e}"))?;
  let field = |key: &str| {
    footer_doc
      .str_field(key)
      .map_err(|e| format!("bad footer: {e}"))?
      .filter(|s| !s.is_empty())
      .ok_or_else(|| format!("bad footer (missing '{key}')"))
  };
  if field("magic")? != "tontinstaller" {
    return Err("not a .tinstaller (bad footer magic)".to_string());
  }
  let num = |key: &str| {
    footer_doc
      .u64_field(key)
      .map_err(|e| format!("bad footer: {e}"))?
      .ok_or_else(|| format!("bad footer (missing '{key}')"))
  };
  let footer = Footer {
    app_size: num("app_size")?,
    license_size: num("license_size")?,
    name: field("name")?,
    bundle_id: field("bundle_id")?,
    version: field("version")?,
  };
  let lic_end = exe
    .len()
    .checked_sub(12 + footer_len)
    .ok_or_else(|| "corrupt .tinstaller (bad footer length)".to_string())?;
  let lic_start = lic_end
    .checked_sub(footer.license_size as usize)
    .ok_or_else(|| "corrupt .tinstaller (bad license size)".to_string())?;
  let app_start = lic_start
    .checked_sub(footer.app_size as usize)
    .ok_or_else(|| "corrupt .tinstaller (bad app size)".to_string())?;
  let app = exe[app_start..lic_start].to_vec();
  let license = if footer.license_size > 0 {
    Some(String::from_utf8_lossy(&exe[lic_start..lic_end]).to_string())
  } else {
    None
  };
  Ok(Payload { footer, app, license })
}

/// Installs the bundled `.app` container below `target`.
///
/// The TAPP container is extracted into a hidden staging directory first
/// and the `<Name>.app` dir inside is renamed into place.
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

  let mut reader = AppReader::from_bytes(&payload.app)
    .map_err(|e| format!("bad .app payload: {e}"))?;
  reader
    .extract_to(&staging)
    .map_err(|e| format!("extract failed: {e}"))?;

  // TAPP containers carry a single top-level "<Name>.app" directory.
  let src = staging.join(format!("{}.app", payload.footer.name));
  let src = if src.is_dir() { src } else { staging.clone() };

  let dest = target.join(format!("{}.app", payload.footer.name));
  if dest.is_dir() {
    std::fs::remove_dir_all(&dest)
      .map_err(|e| format!("cannot replace '{}': {}", dest.display(), e))?;
  } else if dest.is_file() {
    // Clean up single-file installs from older versions.
    std::fs::remove_file(&dest)
      .map_err(|e| format!("cannot replace '{}': {}", dest.display(), e))?;
  }

  std::fs::rename(&src, &dest)
    .map_err(|e| format!("cannot finalize '{}': {}", dest.display(), e))?;
  let _ = std::fs::remove_dir_all(&staging);
  Ok(dest)
}

fn detect_locale() -> &'static str {
  for key in ["LANGUAGE", "LANG", "LC_ALL"] {
    if let Ok(lang) = std::env::var(key) {
      if !lang.is_empty() {
        if lang.to_ascii_lowercase().starts_with("de") {
          return "de_de";
        }
        return "en_us";
      }
    }
  }
  let conf = std::fs::read_to_string("/etc/locale.conf").unwrap_or_default();
  for line in conf.lines() {
    if let Some(lang) = line.strip_prefix("LANG=") {
      if lang.to_ascii_lowercase().starts_with("de") {
        return "de_de";
      }
      return "en_us";
    }
  }
  "en_us"
}

fn load_lang() -> HashMap<String, String> {
  let raw = match detect_locale() {
    "de_de" => include_str!("../lang/de_de.json"),
    _ => include_str!("../lang/en_us.json"),
  };
  JSONSerialization::parse_lang_file(raw)
    .map(|(_, map)| map)
    .unwrap_or_default()
}

fn t(lang: &HashMap<String, String>, key: &str, vars: &[(&str, &str)]) -> String {
  let template = lang.get(key).map(String::as_str).unwrap_or(key);
  let mut s = template.to_string();
  for (k, v) in vars {
    s = s.replace(&format!("{{{k}}}"), v);
  }
  s
}

fn username() -> String {
  std::env::var("USER").unwrap_or_else(|_| "user".to_string())
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Screen {
  Destination,
  License,
  Install,
  Done,
}

#[derive(Debug)]
struct Shared {
  screen: Screen,
  all_users: bool,
  install_requested: bool,
  close_requested: bool,
}

fn title_text(content: String) -> FormattedText {
  FormattedText::spans(vec![Span::new(content).bold()])
    .style(TextStyle::Title2)
    .alignment(TextAlignment::Center)
    .width(640.0)
}

fn primary_button(label: String, callback: impl FnMut() + 'static) -> Button {
  Button::new(label)
    .style(ButtonStyle::BorderedProminent)
    .accent(ACCENT)
    .on_press(callback)
}

fn plain_button(label: String, callback: impl FnMut() + 'static) -> Button {
  Button::new(label).on_press(callback)
}

/// Theme every child of a screen stack. Buttons get the installer accent,
/// texts follow the theme mode; everything follows window focus.
fn theme_stack(stack: &mut VStack, mode: ThemeMode, focused: bool) {
  let dark = mode == ThemeMode::Dark;
  for index in 0..stack.len() {
    if let Some(button) = stack.child_mut::<Button>(index) {
      button.set_theme(ACCENT, dark);
      button.set_focused(focused);
      continue;
    }
    if let Some(text) = stack.child_mut::<BasicText>(index) {
      text.set_theme(mode);
      text.set_focused(focused);
      continue;
    }
    if let Some(text) = stack.child_mut::<FormattedText>(index) {
      text.set_theme(mode);
      text.set_focused(focused);
      continue;
    }
    if let Some(scroll) = stack.child_mut::<ScrollView>(index) {
      scroll.set_theme(ACCENT, dark);
      scroll.set_focused(focused);
    }
  }
}

struct InstallerApp {
  bar: Titlebar,
  dest: VStack,
  license: VStack,
  install: VStack,
  done: VStack,
  payload: Payload,
  lang: HashMap<String, String>,
  shared: Rc<RefCell<Shared>>,
  result: Option<Result<String, String>>,
  watcher: ThemeWatcher,
  focused: bool,
  bg: Color,
  command: Option<WindowCommand>,
}

impl InstallerApp {
  fn new(title: String, payload: Payload, lang: HashMap<String, String>) -> Self {
    let shared = Rc::new(RefCell::new(Shared {
      screen: Screen::Destination,
      all_users: false,
      install_requested: false,
      close_requested: false,
    }));
    let has_license = payload.license.is_some();

    // Destination screen children: title, spacer, all, self, spacer, next.
    let dest = {
      let next_shared = shared.clone();
      let next_license = has_license;
      let all_shared = shared.clone();
      let self_shared = shared.clone();
      VStack::new()
        .spacing(16.0)
        .align(Align::Center)
        .child(title_text(t(&lang, "where.title", &[])))
        .child(Spacer::new().min_size(12.0))
        .child(
          Button::new(t(&lang, "option.all", &[]))
            .style(ButtonStyle::BorderedProminent)
            .accent(ACCENT)
            .on_press(move || all_shared.borrow_mut().all_users = true),
        )
        .child(
          Button::new(t(&lang, "option.self", &[("user", &username())])).on_press(
            move || self_shared.borrow_mut().all_users = false,
          ),
        )
        .child(Spacer::new().min_size(24.0))
        .child(primary_button(t(&lang, "button.next", &[]), move || {
          next_shared.borrow_mut().screen = if next_license {
            Screen::License
          } else {
            Screen::Install
          };
        }))
    };

    let license_body = wrap_text(
      payload.license.as_deref().unwrap_or_default(),
      LICENSE_WRAP_CHARS,
    );
    let license = {
      let back_shared = shared.clone();
      let accept_shared = shared.clone();
      VStack::new()
        .spacing(16.0)
        .align(Align::Center)
        .child(title_text(t(&lang, "license.title", &[])))
        .child(ScrollView::new(VStack::new().spacing(0.0).child(
          BasicText::new(license_body)
            .style(TextStyle::Footnote)
            .width(600.0),
        )))
        .child(plain_button(t(&lang, "button.back", &[]), move || {
          back_shared.borrow_mut().screen = Screen::Destination;
        }))
        .child(primary_button(
          t(&lang, "button.accept", &[]),
          move || accept_shared.borrow_mut().screen = Screen::Install,
        ))
    };

    let install = {
      let install_shared = shared.clone();
      let back_shared = shared.clone();
      let back_license = has_license;
      VStack::new()
        .spacing(16.0)
        .align(Align::Center)
        .child(title_text(t(
          &lang,
          "install.title",
          &[("name", &payload.footer.name)],
        )))
        .child(
          BasicText::new(format!("{} {}", payload.footer.name, payload.footer.version))
            .style(TextStyle::Subheadline)
            .alignment(TextAlignment::Center)
            .width(640.0),
        )
        .child(Spacer::new().min_size(24.0))
        .child(primary_button(
          t(&lang, "button.install", &[]),
          move || install_shared.borrow_mut().install_requested = true,
        ))
        .child(plain_button(t(&lang, "button.back", &[]), move || {
          back_shared.borrow_mut().screen = if back_license {
            Screen::License
          } else {
            Screen::Destination
          };
        }))
    };

    // Done screen is rebuilt after the install with the real message.
    let done = VStack::new().spacing(16.0).align(Align::Center);

    Self {
      bar: Titlebar::new(title),
      dest,
      license,
      install,
      done,
      payload,
      lang,
      shared,
      result: None,
      watcher: ThemeWatcher::new(),
      focused: true,
      bg: TontooUI::renderer::window::BACKGROUND,
      command: None,
    }
  }

  fn active_is_license(&self) -> bool {
    self.shared.borrow().screen == Screen::License
  }

  fn active_stack(&mut self) -> &mut VStack {
    match self.shared.borrow().screen {
      Screen::Destination => &mut self.dest,
      Screen::License => &mut self.license,
      Screen::Install => &mut self.install,
      Screen::Done => &mut self.done,
    }
  }

  /// Refresh the destination option labels (selected marker).
  fn refresh_options(&mut self) {
    let all_users = self.shared.borrow().all_users;
    let base_all = t(&self.lang, "option.all", &[]);
    let base_self = t(&self.lang, "option.self", &[("user", &username())]);
    if let Some(button) = self.dest.child_mut::<Button>(2) {
      button.set_label(format!("{} {base_all}", if all_users { "●" } else { "○" }));
    }
    if let Some(button) = self.dest.child_mut::<Button>(3) {
      button.set_label(format!("{} {base_self}", if all_users { "○" } else { "●" }));
    }
  }

  fn rebuild_done(&mut self) {
    let message = match &self.result {
      Some(Ok(target)) => t(
        &self.lang,
        "done.success",
        &[("name", &self.payload.footer.name), ("target", target)],
      ),
      Some(Err(err)) => t(&self.lang, "done.error", &[("error", err)]),
      None => String::new(),
    };
    let close_shared = self.shared.clone();
    let close_label = t(&self.lang, "button.close", &[]);
    self.done = VStack::new()
      .spacing(16.0)
      .align(Align::Center)
      .child(
        BasicText::new(message)
          .style(TextStyle::Body)
          .alignment(TextAlignment::Center)
          .width(640.0),
      )
      .child(Spacer::new().min_size(24.0))
      .child(primary_button(close_label, move || {
        close_shared.borrow_mut().close_requested = true;
      }));
  }

  fn do_install(&self) -> Result<String, String> {
    if self.shared.borrow().all_users {
      let exe = std::fs::read_link("/proc/self/exe")
        .map_err(|e| format!("cannot resolve self: {e}"))?;
      let status = Command::new("pkexec")
        .arg(&exe)
        .arg(SYSTEM_INSTALL_ARG)
        .status()
        .map_err(|e| format!("cannot run pkexec: {e}"))?;
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

impl App for InstallerApp {
  fn draw(
    &mut self,
    scene: &mut Scene,
    fonts: &mut FontSystem,
    images: &mut ImageLoader<'_>,
    viewport: Viewport,
    time_secs: f64,
  ) {
    self.watcher.poll(time_secs);
    self.watcher.set_focused(self.focused, time_secs);
    let palette = self.watcher.palette(time_secs);
    self.bg = palette.bg;
    let mode = self.watcher.theme().mode;
    let focused = self.focused;

    if self.shared.borrow().install_requested {
      self.shared.borrow_mut().install_requested = false;
      self.result = Some(self.do_install());
      self.shared.borrow_mut().screen = Screen::Done;
      self.rebuild_done();
    }
    self.refresh_options();

    theme_stack(&mut self.dest, mode, focused);
    theme_stack(&mut self.license, mode, focused);
    theme_stack(&mut self.install, mode, focused);
    theme_stack(&mut self.done, mode, focused);

    self
      .bar
      .set_palette(palette.titlebar_bg, palette.titlebar_text, palette.divider);
    self.bar.set_rect(viewport.x, viewport.y, viewport.width);
    self.bar.draw(scene, fonts);

    // Content below the 31px title bar with a 28px top gap. The license
    // screen gets the full body height so its ScrollView flexes and
    // scrolls internally; other screens keep their measured size.
    let top = viewport.y + 31.0 + 28.0;
    let body_h = (viewport.height - 31.0 - 28.0 - 16.0).max(0.0);
    if self.active_is_license() {
      let (stack_w, _) = self.license.measure(fonts);
      let x = viewport.x + ((viewport.width - stack_w) / 2.0).max(0.0);
      self.license.place(fonts, x, top, stack_w, body_h);
      self.license.draw(scene, fonts, images);
    } else {
      let stack = self.active_stack();
      let (stack_w, stack_h) = stack.measure(fonts);
      let x = viewport.x + ((viewport.width - stack_w) / 2.0).max(0.0);
      stack.place(fonts, x, top, stack_w, stack_h);
      self.active_stack().draw(scene, fonts, images);
    }
  }

  fn background(&self) -> Color {
    self.bg
  }

  fn drag_region(&self) -> Option<(f32, f32, f32, f32)> {
    Some(self.bar.drag_rect())
  }

  fn poll_window_command(&mut self) -> Option<WindowCommand> {
    if self.shared.borrow().close_requested {
      self.shared.borrow_mut().close_requested = false;
      return Some(WindowCommand::Close);
    }
    self.command.take()
  }

  fn mouse_down(&mut self, x: f64, y: f64) {
    match self.bar.press(x as f32, y as f32) {
      Some(TrafficAction::Close) => self.command = Some(WindowCommand::Close),
      Some(TrafficAction::Minimize) => self.command = Some(WindowCommand::Minimize),
      Some(TrafficAction::Maximize) => self.command = Some(WindowCommand::ToggleMaximize),
      None => self.active_stack().mouse_down(x, y),
    }
  }

  fn mouse_up(&mut self, x: f64, y: f64) {
    self.active_stack().mouse_up(x, y);
  }

  fn mouse_move(&mut self, x: f64, y: f64) {
    self.bar.set_hover(x as f32, y as f32);
    self.active_stack().set_hover(x as f32, y as f32);
  }

  fn mouse_wheel(&mut self, dx: f64, dy: f64) {
    // Only the license ScrollView scrolls; it lives at index 1.
    if self.active_is_license() {
      if let Some(scroll) = self.license.child_mut::<ScrollView>(1) {
        scroll.mouse_wheel(dx, dy);
      }
    }
  }

  fn key(&mut self, key: Key) {
    if key == Key::Escape {
      self.shared.borrow_mut().close_requested = true;
    }
  }

  fn set_focused(&mut self, focused: bool) {
    self.focused = focused;
    self.bar.set_focused(focused);
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
        eprintln!("error: {err}");
        std::process::exit(1);
      }
    }
  }

  let payload = match read_payload() {
    Ok(payload) => payload,
    Err(err) => {
      eprintln!("error: {err}");
      std::process::exit(1);
    }
  };
  let lang = load_lang();
  let title = t(&lang, "window.title", &[]);

  let app = InstallerApp::new(title.clone(), payload, lang);
  if let Err(err) = run(&title, 720, 680, app) {
    eprintln!("error: {err}");
    std::process::exit(1);
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn footer_parses_fixed_schema() {
    let text = r#"{"magic":"tontinstaller","app_size":10,"license_size":0,"name":"Foo","bundle_id":"com.tontoo.foo","version":"1.0"}"#;
    let doc = JsonDocument::parse(text).expect("footer parses");
    assert_eq!(
      doc.str_field("magic").unwrap(),
      Some("tontinstaller".to_string())
    );
    assert_eq!(doc.u64_field("app_size").unwrap(), Some(10));
  }

  #[test]
  fn license_wraps_long_lines() {
    let wrapped = wrap_text("word ".repeat(30).trim(), 72);
    assert!(wrapped.lines().count() > 1);
    assert!(wrapped.lines().all(|l| l.chars().count() <= 72));
  }

  #[test]
  fn installer_lang_files_parse() {
    for raw in [
      include_str!("../lang/en_us.json"),
      include_str!("../lang/de_de.json"),
    ] {
      let (_, map) = JSONSerialization::parse_lang_file(raw).expect("lang parses");
      assert!(map.contains_key("window.title"));
      assert!(map.contains_key("button.close"));
    }
  }
}
