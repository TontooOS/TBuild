use clap::Parser;
use serde_json::Value;
use std::fs::{self, File};
use std::io::BufWriter;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use walkdir::WalkDir;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

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
        eprintln!("error: {}", err);
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
        .map_err(|e| format!("cannot open project dir '{}': {}", project_dir, e))?;
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
    let proj: Value = serde_json::from_str(
        &fs::read_to_string(&proj_path)
            .map_err(|e| format!("cannot read '{}': {}", proj_path.display(), e))?,
    )
    .map_err(|e| format!("cannot parse '{}': {}", PROJ_FILE, e))?;

    let name = proj
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{} is missing 'name'", PROJ_FILE))?
        .to_string();
    let version = proj
        .get("version")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{} is missing 'version'", PROJ_FILE))?
        .to_string();
    let bundle_id = proj
        .get("bundle_id")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{} is missing 'bundle_id'", PROJ_FILE))?
        .to_string();

    println!("Building {} v{}", name, version);

    build_release(project)?;
    let binary = find_binary_name(project)?;
    let bin_path = project.join("target").join("release").join(&binary);
    if !bin_path.exists() {
        return Err(format!("binary not found at '{}'", bin_path.display()));
    }

    let staging = std::env::temp_dir().join(format!("tbuild-{}", std::process::id()));
    let app_dir = staging.join(format!("{}.app", name));
    let bin_dir = app_dir.join("App");
    let resources_dir = app_dir.join("Resources");
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&bin_dir).map_err(|e| format!("cannot create '{}': {}", bin_dir.display(), e))?;
    fs::create_dir_all(&resources_dir)
        .map_err(|e| format!("cannot create '{}': {}", resources_dir.display(), e))?;

    fs::copy(&bin_path, bin_dir.join(&binary))
        .map_err(|e| format!("cannot copy binary '{}': {}", bin_path.display(), e))?;
    let mut perms = fs::metadata(bin_dir.join(&binary))
        .map_err(|e| format!("cannot read binary metadata: {}", e))?
        .permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
    fs::set_permissions(bin_dir.join(&binary), perms)
        .map_err(|e| format!("cannot set binary permissions: {}", e))?;

    let proj_resources = project.join("Resources");
    if proj_resources.is_dir() {
        copy_dir_recursive(&proj_resources, &resources_dir)?;
    }

    let proj_icon = proj
        .get("icon")
        .and_then(Value::as_str)
        .map(Path::new)
        .map(|p| project.join(p))
        .unwrap_or_else(|| project.join("Resources").join("icon.png"));
    if proj_icon.exists() {
        fs::copy(&proj_icon, bin_dir.join("icon.png"))
            .map_err(|e| format!("cannot copy icon '{}': {}", proj_icon.display(), e))?;
        fs::copy(&proj_icon, resources_dir.join("icon.png"))
            .map_err(|e| format!("cannot copy icon '{}': {}", proj_icon.display(), e))?;
    }

    let info = build_info(&proj, &project.join("lang"))?;
    let info_json = serde_json::to_string_pretty(&info)
        .map_err(|e| format!("cannot serialize Info.tontoo: {}", e))?;
    fs::write(app_dir.join("Info.tontoo"), format!("{}\n", info_json))
        .map_err(|e| format!("cannot write Info.tontoo: {}", e))?;

    let zip_path = out_dir.join(format!("{}.app", name));
    zip_dir(&app_dir, &zip_path)?;
    // Mark the bundle executable so binfmt_misc can dispatch "./Foo.app" to tapp.
    let mut perms = fs::metadata(&zip_path)
        .map_err(|e| format!("cannot read metadata of '{}': {}", zip_path.display(), e))?
        .permissions();
    PermissionsExt::set_mode(&mut perms, 0o755);
    fs::set_permissions(&zip_path, perms)
        .map_err(|e| format!("cannot set permissions on '{}': {}", zip_path.display(), e))?;
    let _ = fs::remove_dir_all(&staging);

    Ok(AppArtifact {
        name,
        version,
        bundle_id,
        app_file: zip_path,
    })
}

fn build_release(project: &Path) -> Result<(), String> {
    let status = Command::new("cargo")
        .arg("build")
        .arg("--release")
        .current_dir(project)
        .status()
        .map_err(|e| format!("failed to run cargo: {}", e))?;
    if !status.success() {
        return Err("cargo build --release failed".to_string());
    }
    Ok(())
}

fn find_binary_name(project: &Path) -> Result<String, String> {
    let cargo_toml =
        fs::read_to_string(project.join("Cargo.toml")).map_err(|e| format!("cannot read Cargo.toml: {}", e))?;
    let value: toml::Value = toml::from_str(&cargo_toml).map_err(|e| format!("cannot parse Cargo.toml: {}", e))?;
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

fn build_info(proj: &Value, lang_dir: &Path) -> Result<Value, String> {
    let bundle_id = proj
        .get("bundle_id")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{} is missing 'bundle_id'", PROJ_FILE))?;
    let version = proj
        .get("version")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{} is missing 'version'", PROJ_FILE))?;

    let mut info = serde_json::Map::new();
    info.insert("bundle_id".to_string(), Value::String(bundle_id.to_string()));

    let mut names = serde_json::Map::new();
    for locale in LOCALES {
        let path = lang_dir.join(format!("{}.json", locale));
        if let Ok(content) = fs::read_to_string(&path) {
            if let Ok(map) = serde_json::from_str::<Value>(&content) {
                if let Some(name) = map.get("name").and_then(Value::as_str) {
                    names.insert(locale.to_string(), Value::String(name.to_string()));
                }
            }
        }
    }
    info.insert("name".to_string(), Value::Object(names));

    info.insert("version".to_string(), Value::String(version.to_string()));
    Ok(Value::Object(info))
}

fn copy_dir_recursive(from: &Path, to: &Path) -> Result<(), String> {
    for entry in WalkDir::new(from) {
        let entry = entry.map_err(|e| format!("walk error: {}", e))?;
        let path = entry.path();
        let rel = path.strip_prefix(from).map_err(|e| format!("strip prefix: {}", e))?;
        let dest = to.join(rel);
        if path.is_dir() {
            fs::create_dir_all(&dest).map_err(|e| format!("cannot create '{}': {}", dest.display(), e))?;
        } else {
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent).map_err(|e| format!("cannot create '{}': {}", parent.display(), e))?;
            }
            fs::copy(path, &dest).map_err(|e| format!("cannot copy '{}': {}", path.display(), e))?;
        }
    }
    Ok(())
}

fn zip_dir(dir: &Path, out: &Path) -> Result<(), String> {
    let top = dir
        .file_name()
        .ok_or_else(|| format!("cannot get name of '{}'", dir.display()))?
        .to_string_lossy()
        .to_string();
    let file = File::create(out).map_err(|e| format!("cannot create '{}': {}", out.display(), e))?;
    let mut zip = ZipWriter::new(BufWriter::new(file));
    let dir_options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .unix_permissions(0o755);
    let file_options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .unix_permissions(0o755);
    for entry in WalkDir::new(dir) {
        let entry = entry.map_err(|e| format!("walk error: {}", e))?;
        let path = entry.path();
        let rel = path.strip_prefix(dir).map_err(|e| format!("strip prefix: {}", e))?;
        let rel_str = if rel.as_os_str().is_empty() {
            top.clone()
        } else {
            format!("{}/{}", top, rel.to_string_lossy().replace('\\', "/"))
        };
        if path.is_dir() {
            zip.add_directory(&rel_str, dir_options)
                .map_err(|e| format!("cannot add directory '{}': {}", rel_str, e))?;
        } else {
            zip.start_file(&rel_str, file_options)
                .map_err(|e| format!("cannot add file '{}': {}", rel_str, e))?;
            let mut f = File::open(path).map_err(|e| format!("cannot open '{}': {}", path.display(), e))?;
            std::io::copy(&mut f, &mut zip).map_err(|e| format!("cannot write '{}': {}", rel_str, e))?;
        }
    }
    zip.finish().map_err(|e| format!("cannot finish zip: {}", e))?;
    Ok(())
}