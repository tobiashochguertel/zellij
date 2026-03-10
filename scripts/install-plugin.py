#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.12"
# dependencies = [
#   "rich>=13",
# ]
# ///
"""
Install configurable tab-bar WASM to ~/.config/zellij and wire up config.kdl.

Steps:
  1. Back up current ~/.config/zellij/plugins/tab-bar.wasm (with timestamp).
  2. Copy the new WASM into place.
  3. Ensure ~/.config/zellij/config.kdl overrides the tab-bar alias to our file.
  4. Ensure ~/.config/zellij/layouts/default.kdl uses pane size=2 for tab-bar.
  5. Prune backups older than MAX_BACKUPS (default 10) per file.
"""

from __future__ import annotations

import argparse
import re
import shutil
import sys
from datetime import datetime
from pathlib import Path

from rich.console import Console
from rich.panel import Panel

console = Console()
TIMESTAMP_FMT = "%Y%m%d_%H%M%S"


def backup_dir(config_dir: Path) -> Path:
    d = config_dir / "backups"
    d.mkdir(parents=True, exist_ok=True)
    return d


def create_backup(src: Path, backup_root: Path, label: str, ts: str) -> Path | None:
    if not src.exists():
        return None
    dest = backup_root / f"{label}_{ts}{src.suffix}"
    shutil.copy2(src, dest)
    return dest


def prune_backups(backup_root: Path, label: str, max_backups: int) -> list[Path]:
    existing = sorted(backup_root.glob(f"{label}_*"))
    removed: list[Path] = []
    while len(existing) > max_backups:
        oldest = existing.pop(0)
        oldest.unlink()
        removed.append(oldest)
    return removed


def install_wasm(wasm_src: Path, config_dir: Path, max_backups: int) -> None:
    plugins_dir = config_dir / "plugins"
    plugins_dir.mkdir(parents=True, exist_ok=True)
    dest = plugins_dir / "tab-bar.wasm"
    backup_root = backup_dir(config_dir)
    ts = datetime.now().strftime(TIMESTAMP_FMT)

    backed_up = create_backup(dest, backup_root, "tab-bar", ts)
    if backed_up:
        console.print(f"  [dim]Backed up WASM → {backed_up.name}[/dim]")

    removed = prune_backups(backup_root, "tab-bar", max_backups)
    for r in removed:
        console.print(f"  [dim]Pruned old backup: {r.name}[/dim]")

    shutil.copy2(wasm_src, dest)
    console.print(f"  [green]✓[/green] WASM installed → {dest}")


def patch_config_kdl(config_kdl: Path, wasm_path: str, rows: int) -> bool:
    """Ensure config.kdl overrides the tab-bar alias. Returns True if file was changed."""
    text = config_kdl.read_text()
    alias_block = f'    tab-bar location="file:{wasm_path}" {{\n        rows {rows}\n    }}'

    # Replace any existing tab-bar alias line (single-line or multi-line block)
    # Match: tab-bar location="..." optionally followed by a { ... } block
    pattern = re.compile(
        r'    tab-bar location="[^"]*"(?:\s*\{[^}]*\})?',
        re.MULTILINE | re.DOTALL,
    )
    if pattern.search(text):
        new_text = pattern.sub(alias_block, text)
    else:
        # No existing tab-bar line – insert before closing } of plugins block
        new_text = text.replace(
            "\n}",
            f"\n{alias_block}\n}}",
            1,
        )

    if new_text != text:
        config_kdl.write_text(new_text)
        return True
    return False


def patch_default_layout(layout_kdl: Path, rows: int) -> bool:
    """Ensure the tab-bar pane in the default layout has size=<rows>. Returns True if changed."""
    if not layout_kdl.exists():
        return False
    text = layout_kdl.read_text()
    # Match: pane size=N ... { ... location="tab-bar" ... }
    pattern = re.compile(
        r'pane size=\d+(.*?plugin location="tab-bar")',
        re.DOTALL,
    )
    new_text = pattern.sub(
        lambda m: f'pane size={rows}{m.group(1)}plugin location="tab-bar"',
        text,
    )
    if new_text != text:
        layout_kdl.write_text(new_text)
        return True
    return False


def grant_permission(wasm_path: Path) -> bool:
    """Pre-grant ReadApplicationState in ~/.cache/zellij/permissions.kdl. Returns True if added."""
    cache_dir = Path.home() / ".cache" / "zellij"
    cache_dir.mkdir(parents=True, exist_ok=True)
    perms_file = cache_dir / "permissions.kdl"

    entry_key = f'"{wasm_path}"'
    block = f'{entry_key} {{\n    ReadApplicationState\n}}\n'

    if perms_file.exists():
        text = perms_file.read_text()
        if entry_key in text:
            return False  # already granted
        perms_file.write_text(text.rstrip() + "\n\n" + block)
    else:
        perms_file.write_text(block)
    return True


def main() -> None:
    ap = argparse.ArgumentParser(description="Install configurable tab-bar plugin")
    ap.add_argument("--wasm-src", required=True, help="Path to compiled tab-bar.wasm")
    ap.add_argument("--config-dir", default="~/.config/zellij", help="Zellij config dir")
    ap.add_argument("--rows", type=int, default=2, help="Number of tab-bar rows (default: 2)")
    ap.add_argument("--max-backups", type=int, default=10, help="Max backups to keep (default: 10)")
    args = ap.parse_args()

    wasm_src = Path(args.wasm_src).expanduser().resolve()
    config_dir = Path(args.config_dir).expanduser().resolve()

    if not wasm_src.exists():
        console.print(f"[red]Error:[/red] WASM not found: {wasm_src}")
        sys.exit(1)

    console.print(Panel(f"Installing configurable tab-bar (rows={args.rows})", style="bold blue"))

    install_wasm(wasm_src, config_dir, args.max_backups)

    wasm_install_path = "~/.config/zellij/plugins/tab-bar.wasm"
    wasm_abs_path = (config_dir / "plugins" / "tab-bar.wasm").resolve()
    config_kdl = config_dir / "config.kdl"
    if config_kdl.exists():
        changed = patch_config_kdl(config_kdl, wasm_install_path, args.rows)
        status = "[green]✓ patched[/green]" if changed else "[dim]already correct[/dim]"
        console.print(f"  {status} config.kdl tab-bar alias")

    layout_kdl = config_dir / "layouts" / "default.kdl"
    changed = patch_default_layout(layout_kdl, args.rows)
    status = "[green]✓ patched[/green]" if changed else "[dim]already correct[/dim]"
    console.print(f"  {status} layouts/default.kdl pane size={args.rows}")

    changed = grant_permission(wasm_abs_path)
    status = "[green]✓ granted[/green]" if changed else "[dim]already granted[/dim]"
    console.print(f"  {status} ReadApplicationState permission in ~/.cache/zellij/permissions.kdl")

    console.print("\n[bold green]Installation complete.[/bold green]")
    console.print("Restart Zellij (or open a new session) to see the two-row tab bar.")


if __name__ == "__main__":
    main()
