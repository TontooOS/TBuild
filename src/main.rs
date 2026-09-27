//! TBuild: builds TontooOS `.app` containers (TAPP format, `.app` extension).
//!
//! Stages the release binary plus `Resources/` into `<Name>.app/` with an
//! `App/` dir, converts the project icon (PNG) into `.tico` via CoreIcon,
//! writes the manifest (`Info.tontoo`, fico syntax) and packs everything
//! with ArchiveKit (`AppBuilder`). The container carries a central
//! directory, so readers (FishRunner, CoreWindows, AboutThisApp) only load
//! the entries they need (manifest, icon, binary) instead of extracting
//! the whole app.

use clap::Parser;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

sdk::preinclude!();

use crate::ArchiveKit::{AppBuilder, AppManifest};
use crate::CoreIcon::generator::{Background, IconCanvas};
use crate::CoreIcon::tico::Tico;
use crate::Foundation::serialization::{JSONSerialization, JsonDocument};

mod tinstaller;

#[derive(Parser)]
#[command(name = "tbuild", about = "TontooOS app builder")]
struct Args {
    /// Either "<project>" or "<command> <project>" where command is "app" or "tinstaller"
    #[arg(num_args = 1..=2)]
    positional: Vec<String>,
    /// Output directory for the built files (defaults to the project dir)
    #[arg(short, long)]
    out: Option<PathBuf>,
}

const PROJ_FILE: &str = "tontoo.proj";
const LOCALES: [&str; 2] = ["en_us", "de_de"];

pub struct AppArtifact {
    pub name: String,
    pub version: String,
    pub bundle_id: String,
    pub app_file: PathBuf,
}

fn main() {
    let args = Args::parse();
    if let Err(err) = run(&args) {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

fn run(args: &Args) -> Result<(), String> {
    let (command, project_dir) = match args.positional.as_slice() {
        [project] => ("app", project.as_str()),
        [command, project] if command == "app" || command == "tinstaller" => {
            (command.as_str(), project.as_str())
        }
        _ => {
            return Err(
                "usage: tbuild [app|tinstaller] <project> [--out <dir>]".to_string(),
            );
        }
    };

    let project = PathBuf::from(project_dir)
        .canonicalize()
        .map_err(|e| format!("cannot open project dir '{project_dir}': {e}"))?;
    if !project.is_dir() {
        return Err(format!("'{}' is not a directory", project.display()));
    }

    let out_dir = match &args.out {
        Some(dir) => dir.clone(),
        None => project.clone(),
    };
    fs::create_dir_all(&out_dir)
        .map_err(|e| format!("cannot create output dir '{}': {}", out_dir.display(), e))?;

    let artifact = build_app(&project, &out_dir)?;

    if command == "tinstaller" {
        let file = tinstaller::make_tinstaller(&project, &out_dir, &artifact)?;
        println!("Built '{}'", file.display());
    } else {
        println!("Built '{}'", artifact.app_file.display());
    }
    Ok(())
}

fn build_app(project: &Path, out_dir: &Path) -> Result<AppArtifact, String> {
    let proj_path = project.join(PROJ_FILE);
    let proj_text = fs::read_to_string(&proj_path)
        .map_err(|e| format!("cannot read '{}': {}", proj_path.display(), e))?;
    let proj = JsonDocument::parse(&proj_text)
        .map_err(|e| format!("cannot parse '{PROJ_FILE}': {e}"))?;

    let required = |field: &str| {
        proj
            .str_field(field)
            .map_err(|e| format!("cannot parse '{PROJ_FILE}': {e}"))?
            .filter(|s| !s.is_empty())
            .ok_or_else(|| format!("{PROJ_FILE} is missing '{field}'"))
    };
    let name = required("name")?;
    let version = required("version")?;
    let bundle_id = required("bundle_id")?;

    println!("Building {name} v{version}");

    build_release(project)?;
    let binary = find_binary_name(project)?;
    let bin_path = project.join("target").join("release").join(&binary);
    if !bin_path.exists() {
        return Err(format!("binary not found at '{}'", bin_path.display()));
    }

    let staging = std::env::temp_dir().join(format!("tbuild-{}", std::process::id()));
    let app_dir = staging.join(format!("{name}.app"));
    let bin_dir = app_dir.join("App");
    let resources_dir = app_dir.join("Resources");
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&bin_dir).map_err(|e| format!("cannot create '{}': {}", bin_dir.display(), e))?;
    fs::create_dir_all(&resources_dir)
        .map_err(|e| format!("cannot create '{}': {}", resources_dir.display(), e))?;

    fs::copy(&bin_path, bin_dir.join(&binary))
        .map_err(|e| format!("cannot copy binary '{}': {}", bin_path.display(), e))?;
    let mut perms = fs::metadata(bin_dir.join(&binary))
        .map_err(|e| format!("cannot read binary metadata: {e}"))?
        .permissions();
    PermissionsExt::set_mode(&mut perms, 0o755);
    fs::set_permissions(bin_dir.join(&binary), perms)
        .map_err(|e| format!("cannot set binary permissions: {e}"))?;

    // Project icon source (PNG): converted to .tico below and therefore
    // skipped when staging Resources, so the container never holds both.
    let proj_icon = proj
        .str_field("icon")
        .map_err(|e| format!("cannot parse '{PROJ_FILE}': {e}"))?
        .map(|p| project.join(p))
        .unwrap_or_else(|| project.join("Resources").join("icon.png"));
    let icon_source = proj_icon
        .canonicalize()
        .ok()
        .filter(|p| p.is_file());

    let proj_resources = project.join("Resources");
    if proj_resources.is_dir() {
        copy_dir_recursive_skip(&proj_resources, &resources_dir, icon_source.as_deref())?;
    }

    let mut manifest = AppManifest::new(&bundle_id, &version, format!("App/{binary}"));
    if icon_source.is_some() {
        manifest.icon = Some("App/icon.tico".to_string());
    }
    for (locale, display) in bundle_names(&project.join("lang")) {
        manifest.names.push((locale, display));
    }

    // Pack staging (App/ + Resources/) plus the generated .tico icons.
    // The manifest covers Info.tontoo, so pack_tree never sees one.
    let mut builder = AppBuilder::new(&name)
        .map_err(|e| format!("cannot create app container: {e}"))?;
    builder.set_manifest(manifest);
    builder
        .pack_tree(&app_dir)
        .map_err(|e| format!("cannot pack '{}': {e}", app_dir.display()))?;
    if let Some(png) = icon_source {
        let tico = build_icon_tico(&png, &name)?;
        builder
            .add_icon_tico("App/icon.tico", tico.clone())
            .map_err(|e| format!("cannot add App/icon.tico: {e}"))?;
        builder
            .add_icon_tico("Resources/icon.tico", tico)
            .map_err(|e| format!("cannot add Resources/icon.tico: {e}"))?;
    }
    let bytes = builder
        .finish()
        .map_err(|e| format!("cannot finish app container: {e}"))?;

    // The container keeps the `.app` extension so binfmt_misc can dispatch
    // "./Foo.app" to FishRunner.
    let app_path = out_dir.join(format!("{name}.app"));
    fs::write(&app_path, &bytes)
        .map_err(|e| format!("cannot write '{}': {}", app_path.display(), e))?;
    let mut perms = fs::metadata(&app_path)
        .map_err(|e| format!("cannot read metadata of '{}': {}", app_path.display(), e))?
        .permissions();
    PermissionsExt::set_mode(&mut perms, 0o755);
    fs::set_permissions(&app_path, perms)
        .map_err(|e| format!("cannot set permissions on '{}': {}", app_path.display(), e))?;
    let _ = fs::remove_dir_all(&staging);

    Ok(AppArtifact {
        name,
        version,
        bundle_id,
        app_file: app_path,
    })
}

fn build_release(project: &Path) -> Result<(), String> {
    let status = Command::new("cargo")
        .arg("build")
        .arg("--release")
        .current_dir(project)
        .status()
        .map_err(|e| format!("failed to run cargo: {e}"))?;
    if !status.success() {
        return Err("cargo build --release failed".to_string());
    }
    Ok(())
}

fn find_binary_name(project: &Path) -> Result<String, String> {
    let cargo_toml =
        fs::read_to_string(project.join("Cargo.toml")).map_err(|e| format!("cannot read Cargo.toml: {e}"))?;
    let value: toml::Value = toml::from_str(&cargo_toml).map_err(|e| format!("cannot parse Cargo.toml: {e}"))?;
    if let Some(bins) = value.get("bin").and_then(|v| v.as_array()) {
        if let Some(first) = bins.first() {
            if let Some(name) = first.get("name").and_then(|v| v.as_str()) {
                return Ok(name.to_string());
            }
        }
    }
    if let Some(name) = value.get("package").and_then(|p| p.get("name")).and_then(|v| v.as_str()) {
        return Ok(name.to_string());
    }
    Err("cannot determine binary name from Cargo.toml".to_string())
}

/// Localized display names from `lang/<locale>.json`.
///
/// Accepts both the Accessibility shape (`{"lang", "translations": {"name"}}`)
/// and the flat shape (`{"name"}`), reading the `name` key.
fn bundle_names(lang_dir: &Path) -> Vec<(String, String)> {
    let mut names = Vec::new();
    for locale in LOCALES {
        let path = lang_dir.join(format!("{locale}.json"));
        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };
        let name = JSONSerialization::parse_lang_file(&content)
            .ok()
            .and_then(|(_, map)| map.get("name").cloned())
            .or_else(|| {
                JSONSerialization::parse_flat_string_map(&content)
                    .ok()
                    .and_then(|map| map.get("name").cloned())
            });
        if let Some(name) = name {
            if !name.is_empty() {
                names.push((locale.to_string(), name));
            }
        }
    }
    names
}

/// Convert the project icon (PNG) into `.tico` bytes via CoreIcon
/// (artwork kept, Liquid Glass finish baked into layers).
fn build_icon_tico(png: &Path, app_name: &str) -> Result<Vec<u8>, String> {
    let canvas = IconCanvas::new()
        .background(Background::image(png.to_string_lossy().to_string()))
        .glass();
    let out = std::env::temp_dir().join(format!(
        "tbuild-icon-{}.tico",
        app_name
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '_' })
            .collect::<String>()
    ));
    Tico::export(&canvas, app_name, &out)
        .map_err(|e| format!("cannot build icon.tico from '{}': {e}", png.display()))?;
    let bytes = fs::read(&out).map_err(|e| format!("cannot read icon.tico: {e}"))?;
    let _ = fs::remove_file(&out);
    Ok(bytes)
}

fn copy_dir_recursive_skip(from: &Path, to: &Path, skip: Option<&Path>) -> Result<(), String> {
    for entry in fs::read_dir(from).map_err(|e| format!("cannot list '{}': {}", from.display(), e))? {
        let entry = entry.map_err(|e| format!("cannot read dir entry: {e}"))?;
        let path = entry.path();
        if let Some(skip) = skip {
            if path.canonicalize().ok().as_deref() == Some(skip) {
                continue;
            }
        }
        let dest = to.join(entry.file_name());
        if path.is_dir() {
            fs::create_dir_all(&dest).map_err(|e| format!("cannot create '{}': {}", dest.display(), e))?;
            copy_dir_recursive_skip(&path, &dest, skip)?;
        } else if path.is_file() {
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent)
                    .map_err(|e| format!("cannot create '{}': {}", parent.display(), e))?;
            }
            fs::copy(&path, &dest).map_err(|e| format!("cannot copy '{}': {}", path.display(), e))?;
        }
    }
    Ok(())
}
