#!/usr/bin/env python3
"""Exercise the real AppImage in a disposable X11 session and retain a screenshot."""
import os
from pathlib import Path
import signal
import socket
import subprocess
import sys
import tempfile
import time

image = Path(sys.argv[1]).resolve()
screenshot = Path(sys.argv[2]).resolve()
screenshot.parent.mkdir(parents=True, exist_ok=True)

if not os.environ.get("OPENMANGO_GUI_TEST_SESSION"):
    with tempfile.TemporaryDirectory(prefix="openmango-gui-") as directory:
        env = os.environ.copy()
        env.update(
            OPENMANGO_GUI_TEST_SESSION="1",
            APPIMAGE_EXTRACT_AND_RUN="1",
            LIBGL_ALWAYS_SOFTWARE="1",
            XDG_CONFIG_HOME=f"{directory}/config",
            XDG_DATA_HOME=f"{directory}/My ფაილები % data",
            XDG_CACHE_HOME=f"{directory}/cache",
            XDG_RUNTIME_DIR=f"{directory}/run",
        )
        env.pop("WAYLAND_DISPLAY", None)
        env.pop("APPDIR", None)
        env.pop("APPIMAGE", None)
        driver = Path("/usr/share/vulkan/icd.d/lvp_icd.json")
        if driver.exists():
            env["VK_DRIVER_FILES"] = str(driver)
        for key in ("XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_CACHE_HOME", "XDG_RUNTIME_DIR"):
            Path(env[key]).mkdir(mode=0o700)
        subprocess.run(
            ["xvfb-run", "-a", "-s", "-screen 0 1280x900x24", "dbus-run-session", "--",
             sys.executable, str(Path(__file__).resolve()), str(image), str(screenshot)],
            env=env, check=True,
        )
    sys.exit(0)

import gi

gi.require_version("Gdk", "3.0")
from gi.repository import Gdk

# Let GIO interpret Desktop Entry escaping and launch arguments. This helper
# waits for its child so the outer test can monitor and clean up the whole group.
DESKTOP_LAUNCH = """
import os, sys
from gi.repository import Gio, GLib
children = []
entry = Gio.DesktopAppInfo.new_from_filename(sys.argv[1])
ok = entry.launch_uris_as_manager([], None, GLib.SpawnFlags.DO_NOT_REAP_CHILD,
    None, None, lambda info, pid, data: children.append(pid), None)
assert ok and len(children) == 1
_, status = os.waitpid(children[0], 0)
sys.exit(os.waitstatus_to_exitcode(status))
"""


def run(*args, **kwargs):
    return subprocess.run(args, check=True, capture_output=True, timeout=30, **kwargs)


def stop(process):
    if process and process.poll() is None:
        os.killpg(process.pid, signal.SIGTERM)
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait()


def wait_for_window(process, listener):
    deadline = time.monotonic() + 90
    listener.settimeout(0.25)
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError(f"AppImage exited before its first frame: {process.returncode}")
        try:
            connection, _ = listener.accept()
        except TimeoutError:
            continue
        with connection:
            connection.settimeout(2)
            response = b""
            while len(response) < 5:
                chunk = connection.recv(5 - len(response))
                if not chunk:
                    break
                response += chunk
            if response == b"ready":
                return
    raise TimeoutError("The packaged app did not render its first window")


app = window_manager = keyring = None
manager = os.environ.get("OPENMANGO_GUI_WINDOW_MANAGER", "openbox")
assert manager in ("openbox", "xfwm4"), "Unsupported test window manager"
with screenshot.with_suffix(".log").open("wb") as log:
    try:
        manager_args = ["xfwm4", "--compositor=on", "--vblank=off"] if manager == "xfwm4" else ["openbox"]
        window_manager = subprocess.Popen(manager_args, stdout=log, stderr=log, start_new_session=True)
        deadline = time.monotonic() + 10
        while b"window id # 0x" not in run("xprop", "-root", "_NET_SUPPORTING_WM_CHECK").stdout:
            if time.monotonic() > deadline:
                raise TimeoutError("The test window manager did not start")
            time.sleep(0.1)
        keyring = subprocess.Popen(
            ["gnome-keyring-daemon", "--foreground", "--unlock", "--components=secrets"],
            stdin=subprocess.PIPE, stdout=log, stderr=log, start_new_session=True,
        )
        keyring.stdin.write(b"openmango-disposable-test-keyring\n")
        keyring.stdin.close()
        run(str(image), "--install-desktop")
        data = Path(os.environ["XDG_DATA_HOME"])
        installed = data / "openmango/OpenMango.AppImage"
        desktop = data / "applications/com.openmango.app.desktop"
        run("desktop-file-validate", str(desktop))
        run(str(installed), "--install-desktop")  # Registering the same copy is idempotent.
        with socket.socket(socket.AF_UNIX) as listener:
            ready_path = Path(os.environ["XDG_RUNTIME_DIR"]) / "ready.sock"
            listener.bind(str(ready_path))
            listener.listen(1)
            env = dict(os.environ, OPENMANGO_UPDATE_READY_SOCKET=str(ready_path))
            env.pop("APPIMAGE_EXTRACT_AND_RUN", None)
            previous_key = None
            for iteration in range(2):
                app = subprocess.Popen([sys.executable, "-c", DESKTOP_LAUNCH, str(desktop)], env=env, stdout=log, stderr=log,
                                       start_new_session=True)
                wait_for_window(app, listener)
                window = run("xdotool", "search", "--onlyvisible", "--class", "com.openmango.app").stdout.splitlines()[0].decode()
                pid = int(run("xdotool", "getwindowpid", window).stdout)
                runtime_env = Path(f"/proc/{pid}/environ").read_bytes().split(b"\0")
                assert b"APPIMAGE_EXTRACT_AND_RUN=1" in runtime_env, "Desktop launch lost the no-FUSE fallback"
                if manager == "xfwm4":
                    hints = run("xprop", "-id", window, "_MOTIF_WM_HINTS").stdout.decode().split("=", 1)[1]
                    flags = [int(value.strip(), 0) for value in hints.split(",")]
                    assert flags[0] & 2 and flags[2] == 0, "The app requested a second, system title bar"
                    extents = run("xprop", "-id", window, "_NET_FRAME_EXTENTS").stdout.decode().split("=", 1)[1]
                    assert all(int(value.strip()) == 0 for value in extents.split(",")), "The window manager added a second title bar"
                run("xdotool", "windowactivate", "--sync", window)
                run("xdotool", "key", "--clearmodifiers", "ctrl+comma")
                time.sleep(1)
                assert app.poll() is None, "Opening Settings crashed the app"
                deadline = time.monotonic() + 15
                while True:
                    key = subprocess.run(
                        ["secret-tool", "lookup", "url", "com.openmango.history.key", "username", "history"],
                        capture_output=True, timeout=3,
                    )
                    if key.returncode == 0 and key.stdout:
                        break
                    if time.monotonic() > deadline:
                        raise RuntimeError("The app did not save its history key in Secret Service")
                    time.sleep(0.25)
                if previous_key is not None:
                    assert key.stdout == previous_key, "The history key changed after restart"
                previous_key = key.stdout  # Never print or persist the disposable secret.
                if iteration == 1:
                    screen = Gdk.get_default_root_window()
                    capture = Gdk.pixbuf_get_from_window(screen, 0, 0, screen.get_width(), screen.get_height())
                    capture.savev(str(screenshot), "png", [], [])
                run("xdotool", "key", "--clearmodifiers", "alt+F4")
                app.wait(timeout=15)
                assert app.returncode == 0, f"AppImage failed on quit: {app.returncode}"
        print(f"Packaged X11 startup ({manager}), desktop registration, Settings shortcut, keyring persistence, and quit/reopen passed")
        print(f"Screenshot: {screenshot}")
    finally:
        stop(app)
        stop(keyring)
        stop(window_manager)
