# TBuild

TontooOS app builder — compiles Rust app projects into `.app` bundles and
self-extracting `.tinstaller` installers.

## Usage

```bash
cargo build --release

# build the .app bundle
./target/release/tbuild app example/helloworld

# build the .tinstaller installer
./target/release/tbuild tinstaller example/helloworld
```

An app project is a Rust crate directory containing `tontoo.proj`,
`Resources/` and `lang/en_us.json` + `lang/de_de.json`.

## Made for TontooOS

Explore more at https://github.com/TontooOS/TontooOS

## License

TCL v26.1