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
| `icon` | `string` | Path to the icon (PNG) relative to the project dir |

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

The project icon (PNG from `tontoo.proj`, else `Resources/icon.png`) is
converted into `.tico` via CoreIcon (artwork kept, Liquid Glass finish)
and stored as both `App/icon.tico` and `Resources/icon.tico`. The source
PNG is skipped when staging `Resources/`, so the container never holds
both. Without a source icon the manifest has no `icon` field and readers
fall back to their placeholder.

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
