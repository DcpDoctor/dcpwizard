#!/usr/bin/env python3
# release GUI against a grok build with the plugin, launched so grok finds it
import os
import subprocess
import sys
from pathlib import Path

PLUGIN_FILE = "libgrokj2k_plugin.so"

if len(sys.argv) < 2:
    sys.exit("usage: run-gpu-gui.py <grok root with lib64/libgrokj2k_plugin.so> [gui args]")
grok_root = Path(sys.argv[1]).expanduser()
lib_dir = grok_root / "lib64"
if not (lib_dir / PLUGIN_FILE).is_file():
    sys.exit(f"no {PLUGIN_FILE} in {lib_dir}")

os.environ["PKG_CONFIG_PATH"] = str(lib_dir / "pkgconfig")
os.environ["LD_LIBRARY_PATH"] = str(lib_dir)
os.environ["GRK_PLUGIN_PATH"] = str(lib_dir)

root = Path(__file__).resolve().parent
cli_manifest = root / "rust" / "Cargo.toml"
gui_dir = root / "gui"
gui_manifest = gui_dir / "src-tauri" / "Cargo.toml"


def run(*command, cwd=root):
    subprocess.run(command, cwd=cwd, check=True)


# grokj2k-sys caches the grok it last linked, so a CPU-only build would be kept
run("cargo", "clean", "-q", "-p", "grokj2k-sys", "--manifest-path", cli_manifest)
run("cargo", "build", "-q", "--release", "-p", "dcpwizard-cli", "--manifest-path", cli_manifest)
run(root / "scripts" / "setup-tauri-bin.sh")
run("cargo", "clean", "-q", "-p", "grokj2k-sys", "--manifest-path", gui_manifest)
run("pnpm", "tauri", "build", "--no-bundle", cwd=gui_dir)
os.execv(gui_dir / "src-tauri" / "target" / "release" / "dcpwizard-gui", ["dcpwizard-gui", *sys.argv[2:]])
