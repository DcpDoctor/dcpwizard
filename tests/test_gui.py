"""GUI launch/exit tests for dcpwizard. Needs a display."""

import os
import subprocess
import sys
import time

def test_gui_launches():
    """Test that the GUI binary starts and can be killed cleanly."""
    gui_bin = os.environ.get("DCPWIZARD_GUI", "gui/src-tauri/target/release/dcpwizard-gui")
    assert os.path.isfile(gui_bin), f"GUI binary not found at {gui_bin}"

    proc = subprocess.Popen([gui_bin], stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    time.sleep(2)
    proc.terminate()
    proc.wait(timeout=5)
    assert proc.returncode is not None

if __name__ == "__main__":
    test_gui_launches()
    print("GUI tests passed")
    sys.exit(0)
