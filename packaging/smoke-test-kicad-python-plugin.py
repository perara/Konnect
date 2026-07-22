#!/usr/bin/env python3
"""Smoke-test the packaged Python action with real KiCad pcbnew/wx modules.

This is intentionally a Linux E2E test: unit stubs cannot prove that KiCad's
installed Python bindings accept and register the ActionPlugin class.
"""

import argparse
import importlib.util
import json
import os
from pathlib import Path
import sys


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--plugin-dir", type=Path, required=True)
    args = parser.parse_args()

    plugin_dir = args.plugin_dir.resolve()
    module_name = "konnect_pcm_plugin_smoke"
    sys.path.insert(0, str(plugin_dir))
    spec = importlib.util.spec_from_file_location(module_name, plugin_dir / "__init__.py")
    if spec is None or spec.loader is None:
        raise RuntimeError("could not load plugin/__init__.py")
    module = importlib.util.module_from_spec(spec)
    sys.modules[module_name] = module
    spec.loader.exec_module(module)

    if module.BINARY_NAME != "konnect":
        raise AssertionError(f"unexpected Linux binary name: {module.BINARY_NAME}")
    if not Path(module.BINARY_PATH).is_file():
        raise AssertionError(f"packaged server is missing: {module.BINARY_PATH}")

    module.register_kicad_instance()
    cache_root = Path(os.environ["XDG_CACHE_HOME"])
    record = cache_root / "konnect" / "kicad-api.json"
    discovery = json.loads(record.read_text(encoding="utf-8"))
    if discovery["socket"] != os.environ["KICAD_API_SOCKET"]:
        raise AssertionError("registered KiCad socket does not match the plugin environment")
    if discovery["token"] != os.environ.get("KICAD_API_TOKEN", ""):
        raise AssertionError("registered KiCad API token does not match")
    if record.stat().st_mode & 0o777 != 0o600:
        raise AssertionError("KiCad discovery record is not private (expected mode 0600)")

    settings_dialog = sys.modules["settings_dialog"]
    detected_cli = settings_dialog.detect_kicad_cli()
    if not detected_cli or not detected_cli.endswith("kicad-cli"):
        raise AssertionError(f"Linux kicad-cli discovery failed: {detected_cli!r}")

    print("KiCad Python action-plugin import and Linux registration passed")


if __name__ == "__main__":
    main()
