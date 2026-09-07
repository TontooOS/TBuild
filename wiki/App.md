# App

The `.app` builder compiles a TontooOS app project and assembles a `.app`
bundle. The `.app` is a ZIP archive with the `.app` extension; at runtime the
system extracts it into the `Applications` folder.

## Project Layout

An app project is a directory with a Rust crate plus TontooOS metadata:

```
helloworld/
├── tontoo.proj          # project metadata (JSON)
├── Cargo.toml           # Rust crate
├── src/main.rs
├── Resources/           # copied into the bundle (icon.png, assets, ...)
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
| `icon` | `string` | Path to the icon relative to the project dir |

Example:

```json
{
  "bundle_id": "com.tontoo.helloworld",
  "name": "HelloWorld",
  "version": "0.1.0",
  "icon": "Resources/icon.png"
}
```

## Bundle Layout

```
HelloWorld.app/                 # ZIP archive named *.app
├── Info.tontoo                 # built metadata (JSON)
├── App/
│   ├── helloworld              # compiled binary (executable)
│   └── icon.png                # app icon copy
└── Resources/
    ├── icon.png                # canonical icon location
    └── lang/                   # copied from the project
        ├── en_us.json
        └── de_de.json
```

### Info.tontoo

The built `Info.tontoo` is generated from `tontoo.proj` and the `lang/` files.
The `name` field is localized from the `name` key of each language file. The
`icon` and `entry` fields are not written.

```json
{
  "bundle_id": "com.tontoo.helloworld",
  "name": {
    "en_us": "Hello world!",
    "de_de": "Hallo welt!"
  },
  "version": "0.1.0"
}
```

## Behavior

- `tbuild app <project> [--out <dir>]` runs `cargo build --release` in the
  project, then assembles and zips the bundle.
- The binary name is read from `[[bin]] name` in `Cargo.toml`, falling back to
  `[package] name`.
- The binary is made executable (`0o755`) and all ZIP entries store Unix
  permissions `0o755`.
- The finished `.app` ZIP file itself gets mode `0o755`, so executing it
  directly (`./Demo.app`) dispatches through the system runner via
  `binfmt_misc`.
- `Resources/` is copied recursively into `Resources/` of the bundle. The icon
  from `tontoo.proj` is copied to both `App/icon.png` and `Resources/icon.png`.
- `Returns Err` when `tontoo.proj` is missing or misses `name`, `version` or
  `bundle_id`, when `Cargo.toml` cannot be parsed, when the release binary is
  missing, or when the output directory cannot be written.

## Cross References

- [Tinstaller.md](Tinstaller.md) – packs the `.app` into a `.tinstaller`
- [MAIN.md](MAIN.md) – overview
- FishRunner (`tapp`) – launches the `.app` bundles on TontooOS