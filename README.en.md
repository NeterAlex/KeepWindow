# KeepWindow

KeepWindow is a native Windows utility that protects selected windows from accidental closing.

## Languages

The application includes Simplified Chinese and English. Choose **More → Language** to switch immediately; the selection is saved in the user configuration. When no language is configured, KeepWindow follows the Windows UI language and falls back to Simplified Chinese for unsupported languages.

Locale JSON files are bundled as resources and validated at build time. If an external resource is missing or corrupted, the compiled Simplified Chinese fallback keeps startup safe.

## Features

- System close-command protection and calibrated mouse shields
- Automatic protection rules with import/export
- Temporary ten-second release and retry recovery
- Tray operation, hotkeys, diagnostics, and privacy-aware event logs

See [README.md](README.md) for the Chinese documentation.
