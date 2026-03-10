#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.12"
# dependencies = [
#   "rich>=13",
#   "tomlkit>=0.13",
# ]
# ///
"""Matrix compatibility test: compile against multiple zellij-tile versions.

Each version runs in an isolated temp directory — the project source is never
modified in-place. Tests WASM compilation (wasm32-wasip1) to ensure the
configurable tab-bar builds correctly across zellij-tile versions.
"""

import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import TypedDict

import tomlkit
from rich import box
from rich.console import Console
from rich.table import Table

console = Console()

# ── Configuration ─────────────────────────────────────────────────────────────

# Versions always included. Host + latest are appended automatically.
PINNED_VERSIONS: list[str] = ["0.41.2", "0.42.2", "0.43.1"]

PROJECT_ROOT = Path(__file__).resolve().parent.parent
TAB_BAR_DIR = PROJECT_ROOT / "default-plugins" / "tab-bar"

# Local cargo registry cache — shared across all matrix versions to avoid
# re-downloading crates on each run.
CARGO_CACHE_DIR = PROJECT_ROOT / ".cache" / "cargo"


# ── Type hints ────────────────────────────────────────────────────────────────


class VersionResult(TypedDict, total=False):
    compiled: bool
    error: str


# ── Version discovery ─────────────────────────────────────────────────────────


def get_host_zellij_version() -> str | None:
    try:
        out = subprocess.check_output(
            ["zellij", "--version"], text=True, timeout=5
        )
        m = re.search(r"(\d+\.\d+\.\d+)", out)
        return m.group(1) if m else None
    except (FileNotFoundError, subprocess.TimeoutExpired, subprocess.CalledProcessError):
        return None


def get_latest_crate_version(crate: str) -> str | None:
    try:
        out = subprocess.check_output(
            [
                "curl", "-sf",
                f"https://crates.io/api/v1/crates/{crate}",
                "-H", "User-Agent: tab-bar-matrix-test/0.1",
            ],
            text=True,
            timeout=10,
        )
        return json.loads(out)["crate"]["newest_version"]
    except Exception:
        return None


def get_host_rust_target() -> str:
    try:
        out = subprocess.check_output(["cargo", "-vV"], text=True)
        for line in out.splitlines():
            if line.startswith("host:"):
                return line.split(":", 1)[1].strip()
    except Exception:
        pass
    return "x86_64-unknown-linux-gnu"


def build_version_list(
    host: str | None, latest: str | None
) -> list[tuple[str, str]]:
    """Return a deduplicated (version, label) list, sorted by version."""
    labels: dict[str, list[str]] = {}
    for v in PINNED_VERSIONS:
        labels.setdefault(v, []).append("pinned")
    if latest:
        labels.setdefault(latest, []).append("latest")
    if host:
        labels.setdefault(host, []).append("host")

    def ver_key(v: str) -> tuple[int, ...]:
        return tuple(int(x) for x in v.split("."))

    return [(v, "+".join(tags)) for v, tags in sorted(labels.items(), key=lambda kv: ver_key(kv[0]))]


# ── Cargo.toml patching ───────────────────────────────────────────────────────


def patch_zellij_tile_version(cargo_toml: Path, version: str) -> None:
    """Rewrite the zellij-tile dependency to an exact pinned version."""
    doc = tomlkit.parse(cargo_toml.read_text())
    deps = doc["dependencies"]
    current = deps.get("zellij-tile")
    if isinstance(current, str):
        # Simple "version" string
        deps["zellij-tile"] = f"={version}"
    elif isinstance(current, dict):
        # Inline table with features, etc.
        current["version"] = f"={version}"
    else:
        deps["zellij-tile"] = f"={version}"
    cargo_toml.write_text(tomlkit.dumps(doc))


# ── Test runner ───────────────────────────────────────────────────────────────


def run_cargo(
    args: list[str], cwd: Path, env: dict[str, str] | None = None
) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["cargo"] + args,
        capture_output=True,
        text=True,
        cwd=cwd,
        env=env,
    )


def test_version(version: str, host_target: str) -> VersionResult:
    # Ensure the shared cargo registry cache directory exists.
    CARGO_CACHE_DIR.mkdir(parents=True, exist_ok=True)

    env = os.environ.copy()
    env["CARGO_HOME"] = str(CARGO_CACHE_DIR)

    with tempfile.TemporaryDirectory(prefix=f"tab-bar-compat-{version}-") as tmpdir:
        tmp = Path(tmpdir)

        # Mirror the project structure needed for a standalone build
        shutil.copytree(TAB_BAR_DIR / "src", tmp / "src")
        shutil.copy(TAB_BAR_DIR / "Cargo.toml", tmp / "Cargo.toml")

        patch_zellij_tile_version(tmp / "Cargo.toml", version)

        # Build WASM binary — this is the real compatibility check
        build_proc = run_cargo(
            [
                "build",
                "--target", "wasm32-wasip1",
                "--release",
                "--manifest-path", str(tmp / "Cargo.toml"),
                "--target-dir", str(tmp / "target"),
            ],
            tmp,
            env=env,
        )
        if build_proc.returncode != 0:
            err = (build_proc.stderr or build_proc.stdout)[-2000:].strip()
            return VersionResult(compiled=False, error=err)

        return VersionResult(compiled=True)


# ── Entry point ───────────────────────────────────────────────────────────────


def main() -> None:
    import argparse

    parser = argparse.ArgumentParser(description="Zellij Tall-Tabs compatibility matrix test")
    parser.add_argument(
        "--clear-cache", action="store_true",
        help=f"Delete the cargo registry cache ({CARGO_CACHE_DIR}) before running"
    )
    args = parser.parse_args()

    if args.clear_cache and CARGO_CACHE_DIR.exists():
        import shutil as _shutil
        console.print(f"[yellow]Clearing cache:[/yellow] {CARGO_CACHE_DIR}")
        _shutil.rmtree(CARGO_CACHE_DIR)

    console.print("\n[bold cyan]Zellij Tall-Tabs — Compatibility Matrix[/bold cyan]\n")

    host_target = get_host_rust_target()
    console.print(f"Rust host target : [dim]{host_target}[/dim]")

    host_zellij = get_host_zellij_version()
    console.print(
        f"Host zellij      : [dim]{host_zellij or 'not detected'}[/dim]"
    )

    cache_size = ""
    if CARGO_CACHE_DIR.exists():
        import subprocess as _sp
        try:
            out = _sp.check_output(["du", "-sh", str(CARGO_CACHE_DIR)], text=True).split()[0]
            cache_size = f" ([dim]cache: {out}[/dim])"
        except Exception:
            pass
    console.print(f"Cargo cache      : [dim]{CARGO_CACHE_DIR}[/dim]{cache_size}")

    with console.status("[dim]Querying crates.io for latest zellij-tile…[/dim]"):
        latest = get_latest_crate_version("zellij-tile")
    console.print(
        f"Latest zellij-tile: [dim]{latest or 'unknown'}[/dim]"
    )

    versions = build_version_list(host_zellij, latest)
    console.print(
        f"\nMatrix: [bold]{len(versions)}[/bold] versions — "
        + ", ".join(f"[cyan]{v}[/cyan]([dim]{l}[/dim])" for v, l in versions)
        + "\n"
    )

    results: dict[str, VersionResult] = {}
    for version, label in versions:
        with console.status(
            f"  Testing [cyan]{version}[/cyan] ([dim]{label}[/dim]) …"
        ):
            results[version] = test_version(version, host_target)

    # ── Results table ─────────────────────────────────────────────────────────

    table = Table(box=box.ROUNDED, show_header=True, header_style="bold")
    table.add_column("zellij-tile", style="cyan", min_width=12)
    table.add_column("Label", style="dim")
    table.add_column("WASM build", justify="center")

    all_ok = True
    for version, label in versions:
        r = results[version]
        if not r.get("compiled", False):
            all_ok = False
            table.add_row(version, label, "[red]✗ build failed[/red]")
        else:
            table.add_row(version, label, "[green]✓[/green]")

    console.print(table)

    # Print error excerpts for failing versions
    for version, _ in versions:
        r = results[version]
        if err := r.get("error"):
            console.print(f"\n[bold red]── {version} error excerpt:[/bold red]")
            console.print(f"[dim]{err}[/dim]")

    if all_ok:
        console.print("\n[bold green]All versions passed ✓[/bold green]\n")
        sys.exit(0)
    else:
        console.print("\n[bold red]Some versions failed — see details above ✗[/bold red]\n")
        sys.exit(1)


if __name__ == "__main__":
    main()
