# KeepWindow

KeepWindow is a native Windows utility that protects selected windows from accidental closing.

## Languages

The application includes Simplified Chinese and English. Choose **More → Language** to switch immediately; the selection is saved in the user configuration. When no language is configured, KeepWindow follows the Windows UI language and falls back to Simplified Chinese for unsupported languages.

Locale JSON files are embedded and validated at build time, including matching keys and named placeholders. The executable also runs independently. Optional catalogs beside the executable in `locales/` override the embedded catalogs; malformed or incomplete external catalogs fall back to embedded Chinese. Missing external files use the embedded language. Configuration version remains 2. Recent events retain the language used when each event occurred.

## Features

- System close-command protection and calibrated mouse shields
- Automatic protection rules with import/export
- Temporary ten-second release and retry recovery
- Tray operation, hotkeys, and diagnostic export with a privacy setting

Recent events stay in memory, with up to 200 entries. In **Recent events and diagnostics**, use **Copy diagnostics** to copy the report or **Export diagnostics** to save a UTF-8 text file through the Windows Save dialog. Both actions omit window titles, paths, and full errors by default. Enable **Include titles, paths, and full errors in diagnostics** to include them.

See [README.md](README.md) for the Chinese documentation.
