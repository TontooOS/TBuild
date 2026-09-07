# Tinstaller

The `.tinstaller` maker builds a self-extracting installer: a single executable
file containing the installer UI, the `.app` bundle and an optional license.
Running it opens a wizard that installs the `.app` into the `Applications`
folder for one user or for all users.

## File Format

```
[installer binary]      # TontooUIKit wizard (compiled from installer/)
[.app ZIP bytes]        # payload from the .app build
[license bytes]         # optional, 0 bytes when no license is found
[4-byte footer length]  # little-endian u32
[footer JSON]           # sizes + metadata
[8-byte magic]          # "TONTINST"
```

The footer JSON is:

```json
{
  "magic": "tontinstaller",
  "app_size": 253769,
  "license_size": 1024,
  "name": "HelloWorld",
  "bundle_id": "com.tontoo.helloworld",
  "version": "0.1.0"
}
```

The installer reads its own file via `/proc/self/exe`, validates the trailing
magic and footer, then slices the `.app` and license out of its own bytes.
`Returns Err` when the file is too small, the magic is missing, the footer
cannot be parsed, or the footer magic is wrong.

## License Detection

`find_license` scans the project directory for these names, case-insensitive,
first match wins:

| Priority | Name |
|---|---|
| 1 | `LICENSE.md` |
| 2 | `LICENSE.txt` |
| 3 | `LICENSE` |

`Returns None` when no license file exists (the license step is then skipped).

## Installer UI

The installer template is a separate crate at `installer/` using
`uikit = { path = "/Library/System/uikit" }`. The wizard has three screens:

| Screen | Content | Next action |
|---|---|---|
| Destination | "Where do you want to install it?" — All Users / My Self (`%username%`) | Next |
| License | License text in a scroll view (skipped when no license) | Accept |
| Install | Big Install button | Install |

The `AppDelegate` keeps the screen state; `handle_custom` handles
`select_all`, `select_self`, `next`, `back` and `install`. The view is rebuilt
after every action.

### Install Targets

| Selection | Target | Admin |
|---|---|---|
| My Self | `/Users/<username>/Applications/<Name>.app/` | none |
| All Users | `/Applications/<Name>.app/` | `pkexec` |

The installer extracts the bundle into `<Target>/<Name>.app/` as a directory
bundle so apps launch directly from disk without unpacking at runtime. The
extraction goes into a hidden staging directory first and is renamed into
place, so a crash never leaves a half-installed bundle behind. Anything
already occupying the target path (an older folder install or a single-file
zipped install from previous versions) is removed first.

### All Users / Admin

For All Users installs the installer re-executes itself through `pkexec` with
the internal `--install-system` argument. The polkit prompt requests admin
rights; the root instance installs the bundle into `/Applications` and exits.
`Returns Err` when `pkexec` is unavailable or fails.

## Localization

The installer strings live in `installer/lang/en_us.json` and
`installer/lang/de_de.json`, embedded via `include_str!`. The active locale is
detected from `LANG`, `LC_ALL` or `/etc/locale.conf` (`de_*` selects German,
everything else English). Templates use `{name}`, `{user}`, `{target}` and
`{error}` placeholders.

## Cross References

- [App.md](App.md) – the `.app` bundle built before packing
- [MAIN.md](MAIN.md) – overview