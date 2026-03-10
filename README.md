# Configurable Tab-Bar for Zellij

A fork of [zellij-org/zellij](https://github.com/zellij-org/zellij)'s built-in `tab-bar` plugin with one addition: **configurable row count**.

Set `rows 2` in your plugin config and `pane size=2` in your layout to get a
two-row tab bar — **tab indices on top, tab names below** — making it much easier
to tap the correct tab when SSH-ing from a phone (e.g., iPhone via Termux).

## Usage

In `~/.config/zellij/config.kdl`:
```kdl
plugins {
    tab-bar location="file:~/.config/zellij/plugins/tab-bar.wasm" {
        rows 2
    }
}
```

In your layout file (`~/.config/zellij/layouts/default.kdl`):
```kdl
layout {
    pane size=2 borderless=true {
        plugin location="tab-bar"
    }
    pane
    pane size=2 borderless=true {
        plugin location="status-bar"
    }
}
```

## Build & Install

```bash
task install          # Build WASM + install + patch config
task build            # Build only
task test:matrix      # Test compilation against multiple zellij-tile versions
```

## How it works

- **`rows 1`** (default): identical to the upstream built-in tab-bar, pixel-perfect.
- **`rows 2`**: renders two rows. Row 0 = tab index numbers (same widths as tab segments).
  Row 1 = the full upstream tab bar (tab names, mode info, session name, mouse support, arrows).
  Mouse clicks on either row focus the correct tab via the existing upstream column-position logic.

## Feature branch

This feature lives on `feature/configurable-tab-bar-rows` and is intended for upstream contribution.
