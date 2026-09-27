# TBuild – Wiki

TBuild is the TontooOS app builder. It compiles Rust app projects, assembles
`.app` bundles and packs them into self-extracting `.tinstaller` installers.

- Repository: https://github.com/TontooOS/TontooOS
- License: TCL
- Version: 0.1.0

## Feature Index

| Feature | File | Description |
|---|---|---|
| Main index | [MAIN.md](MAIN.md) | This page |
| Rules | [RULE.md](RULE.md) | Development and usage rules |
| App | [App.md](App.md) | `.app` bundle builder |
| Tinstaller | [Tinstaller.md](Tinstaller.md) | `.tinstaller` self-extracting installer maker |

## Quick Start

Build the `.app` bundle of an app project (a directory containing
`tontoo.proj`, `Cargo.toml` and `lang/`):

```bash
cargo build --release
./target/release/tbuild app example/helloworld
```

Build the `.tinstaller` installer of the same project:

```bash
./target/release/tbuild tinstaller example/helloworld
```

See [App.md](App.md) and [Tinstaller.md](Tinstaller.md) for details.

## Changelog

- 2026-09-27: TAPP containers: `.app` files are indexed ArchiveKit containers
  (fico manifest, `.tico` icons, still `.app` extension) instead of ZIPs;
  readers only load manifest, icon and binary. Installer wizard ported to the
  new TontooUI on Vello/WGPU; SDK from `/Library/System/sdk`.
- 2026-08-25: Install format finalized macOS-style: the `.tinstaller` extracts
  the bundle as a directory (`<Name>.app/`) atomically via a staging rename,
  replacing any older folder or single-file install. Built `.app` files
  stay executable so downloaded bundles launch directly through the `tapp`
  binfmt handler.
- 2026-08-20: Initial wiki, `.app` builder and `.tinstaller` maker, installer
  template with TontooUIKit wizard UI (`lang/en_us.json` + `lang/de_de.json`).