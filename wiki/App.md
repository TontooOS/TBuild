# App

The `.app` builder compiles a TontooOS app project and assembles a `.app`
container. The `.app` is a TAPP container (ArchiveKit: magic `TAPP`,
central directory in the footer) with the `.app` extension; readers only
load the entries they need (manifest, icon, binary) instead of extracting
the whole app. At runtime the system installs it into the `Applications`
folder.

## Project Layout

An app project is a directory with a Rust crate plus TontooOS metadata:

```
helloworld/
├── tontoo.proj          # project metadata (JSON)
├── Cargo.toml           # Rust crate
├── src/main.rs
├── Resources/           # copied into the container (assets, ...)
└── lang/
    ├── en_us.json       # app strings (required)
    └── de_de.json
```

## tontoo.proj

| Field | Type | Description |
|---|---|---|
| `bundle_id` | `string` | Global unique app identifier, e.g. `com.tontoo.helloworld` |
| `name` | `string` | App name, e.g. `HelloWorld` |
| `version` | `string` | Version, e.g. `0.1.0` |
| `icon` | `string` | Path to the icon (raster or `.tico`) relative to the project dir |

Example:

```json
{
  "bundle_id": "com.tontoo.helloworld",
  "name": "HelloWorld",
  "version": "0.1.0",
  "icon": "Resources/icon.png"
}
```

## Container Layout

```
HelloWorld.app            # TAPP container named *.app
├── Info.tontoo           # manifest (fico syntax, stored first)
├── App/
│   ├── helloworld        # compiled binary (0o755, stored)
│   └── icon.tico         # app icon (CoreIcon .tico, validated)
└── Resources/
    ├── icon.tico         # canonical icon location
    └── lang/             # copied from the project
        ├── en_us.json
        └── de_de.json
```

### Info.tontoo

The built `Info.tontoo` is generated from `tontoo.proj` and the `lang/` files
and written in fico syntax (`AppManifest`). The `name` table is localized
from the `name` key of each language file, accepting both the Accessibility
shape (`translations.name`) and the flat shape (`name`).

```text
app {
    bundle_id: com.tontoo.helloworld
    version: "0.1.0"
    executable: App/helloworld
    icon: App/icon.tico
    name {
        en_us: "Hello world!"
        de_de: "Hallo welt!"
    }
}
```

### Icons

The project icon comes from `tontoo.proj` (`icon`), else
`Resources/icon.png`. It may be either a raster (PNG, JPG, ...) or an
already built `.tico`:

| Source | Handling |
|---|---|
| Raster | Converted to `.tico` via CoreIcon |
| `.tico` (by file extension) | Passed through byte for byte |

Either way the result is stored as both `App/icon.tico` and
`Resources/icon.tico`, and the source file is skipped when staging
`Resources/`, so the container never holds both. Without a source icon
the manifest has no `icon` field and readers fall back to their
placeholder.

> **Note:** A raster becomes exactly one full-bleed
> `LayerContent::image` layer over a transparent background, matching
> `CoreIcon/examples/tico_from_png`. ArchiveKit rejects a container with an
> empty layer table (`tico has no layers`), so a canvas that carries only a
> background cannot be exported at all. The layer is stored
> non-recolorable and keeps its colors; `TicoIcon::render` adds the Apple
> app-icon finish later.

> **Note:** A `.tico` source is detected purely by extension and never
> decoded, so a project can ship a finished layered icon (Xcode,
> Terminal, SystemOverview) and skip the conversion entirely. The bundled
> bytes are identical to the project file.

## Behavior

- `tbuild app <project> [--out <dir>]` runs `cargo build --release` in the
  project, then stages and packs the container with `AppBuilder`.
- The binary name is read from `[[bin]] name` in `Cargo.toml`, falling back to
  `[package] name`.
- The binary is made executable (`0o755`); modes are preserved from staging.
  Entries are stored (fastest reads); the manifest is always first so readers
  find it without the index.
- The finished `.app` file itself gets mode `0o755`, so executing it
  directly (`./Demo.app`) dispatches through the system runner via
  `binfmt_misc`.
- `Resources/` is copied recursively into `Resources/` of the container,
  except the icon source file (see Icons).
- `Returns Err` when `tontoo.proj` is missing or misses `name`, `version` or
  `bundle_id`, when `Cargo.toml` cannot be parsed, when the release binary is
  missing, when the icon cannot be converted, or when the output directory
  cannot be written.

## Cross References

- [Tinstaller.md](Tinstaller.md) – packs the `.app` into a `.tinstaller`
- [MAIN.md](MAIN.md) – overview
- FishRunner (`tapp`) – launches the `.app` containers on TontooOS
