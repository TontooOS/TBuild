//! `.tinstaller` packer: self-extracting installer binary with the TAPP
//! `.app` container and the license appended.
//!
//! Layout: `[installer ELF][.app TAPP][license bytes][footer JSON]`
//! + `[u32 LE footer_len][TONTINST]`. The footer is fixed-schema JSON
//! (built without dependencies); the installer runtime parses it with
//! Foundation and extracts the container with ArchiveKit.

use crate::AppArtifact;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

const MAGIC: &[u8; 8] = b"TONTINST";
const LICENSE_NAMES: [&str; 3] = ["license.md", "license.txt", "license"];

/// Minimal JSON string escaping for the fixed footer schema.
fn json_escape(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 2);
    for c in raw.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

pub fn make_tinstaller(project: &Path, out_dir: &Path, app: &AppArtifact) -> Result<PathBuf, String> {
    let installer_exe = build_installer_binary()?;

    let app_bytes = fs::read(&app.app_file)
        .map_err(|e| format!("cannot read '{}': {}", app.app_file.display(), e))?;

    let license = find_license(project)?;
    let license_bytes = match &license {
        Some(path) => fs::read(path).map_err(|e| format!("cannot read license '{}': {}", path.display(), e))?,
        None => Vec::new(),
    };

    let footer = format!(
        "{{\"magic\":\"tontinstaller\",\"app_size\":{},\"license_size\":{},\"name\":\"{}\",\"bundle_id\":\"{}\",\"version\":\"{}\"}}",
        app_bytes.len(),
        license_bytes.len(),
        json_escape(&app.name),
        json_escape(&app.bundle_id),
        json_escape(&app.version),
    );
    let footer_bytes = footer.into_bytes();

    let exe_bytes = fs::read(&installer_exe)
        .map_err(|e| format!("cannot read installer binary '{}': {}", installer_exe.display(), e))?;

    let mut out = Vec::with_capacity(
        exe_bytes.len() + app_bytes.len() + license_bytes.len() + footer_bytes.len() + 12,
    );
    out.extend_from_slice(&exe_bytes);
    out.extend_from_slice(&app_bytes);
    out.extend_from_slice(&license_bytes);
    out.extend_from_slice(&footer_bytes);
    out.extend_from_slice(&(footer_bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(MAGIC);

    let out_path = out_dir.join(format!("{}.tinstaller", app.name));
    fs::write(&out_path, &out).map_err(|e| format!("cannot write '{}': {}", out_path.display(), e))?;
    let mut perms = fs::metadata(&out_path)
        .map_err(|e| format!("cannot read metadata of '{}': {}", out_path.display(), e))?
        .permissions();
    PermissionsExt::set_mode(&mut perms, 0o755);
    fs::set_permissions(&out_path, perms)
        .map_err(|e| format!("cannot set permissions on '{}': {}", out_path.display(), e))?;

    Ok(out_path)
}

fn build_installer_binary() -> Result<PathBuf, String> {
    let installer_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("installer");
    let manifest = installer_dir.join("Cargo.toml");
    if !manifest.exists() {
        return Err(format!("installer template not found at '{}'", installer_dir.display()));
    }
    let status = Command::new("cargo")
        .args(["build", "--release", "--manifest-path"])
        .arg(&manifest)
        .status()
        .map_err(|e| format!("failed to run cargo for installer template: {}", e))?;
    if !status.success() {
        return Err("installer template build failed".to_string());
    }
    let exe = installer_dir.join("target").join("release").join("tinstaller");
    if !exe.exists() {
        return Err(format!("installer binary not found at '{}'", exe.display()));
    }
    Ok(exe)
}

fn find_license(project: &Path) -> Result<Option<PathBuf>, String> {
    let mut found: Vec<(usize, PathBuf)> = Vec::new();
    let entries = fs::read_dir(project).map_err(|e| format!("cannot list project dir: {e}"))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("cannot read project dir entry: {e}"))?;
        let name = entry.file_name().to_string_lossy().to_lowercase();
        if let Some(index) = LICENSE_NAMES.iter().position(|candidate| *candidate == name) {
            found.push((index, entry.path()));
        }
    }
    found.sort_by_key(|(index, _)| *index);
    Ok(found.into_iter().next().map(|(_, path)| path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn footer_escapes_names() {
        assert_eq!(json_escape("plain"), "plain");
        assert_eq!(json_escape("a\"b\\c"), "a\\\"b\\\\c");
    }
}
