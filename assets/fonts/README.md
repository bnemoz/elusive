# Fonts

`DESIGN_SYSTEM.md` §2 specifies **Inter** for UI text and **JetBrains Mono** for
paths, identifiers and exact analytical values.

Neither is committed here. Both are OFL-licensed and redistributable, but vendoring
binaries into a source repository means shipping their licence files and keeping
them updated, which is a decision for whoever cuts the first signed release rather
than something to do by default.

To use the intended typefaces, drop these files into this directory:

```text
assets/fonts/Inter-Regular.ttf
assets/fonts/JetBrainsMono-Regular.ttf
```

`egui_adapter::install_fonts` picks them up at startup. When they are absent the
app falls back to egui's bundled faces and says so in the navigation rail, because
a missing font should not stop anyone opening a chromatogram.

- Inter — https://rsms.me/inter/ (SIL Open Font License 1.1)
- JetBrains Mono — https://www.jetbrains.com/lp/mono/ (SIL Open Font License 1.1)
