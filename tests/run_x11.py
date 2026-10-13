#!/usr/bin/env python3
"""Synthetic X11 integration test. Never targets an existing/user desktop."""
import os, subprocess, tempfile, time, random
from pathlib import Path
root=Path(__file__).resolve().parents[1]
subprocess.run(["cargo","test","--locked","--manifest-path",str(root/"rust/Cargo.toml"),"--test","remote_x11","--no-run"],check=True)
with tempfile.TemporaryDirectory(prefix="localconnect-x11-") as directory:
    auth=Path(directory)/"authority"
    number=random.randrange(100,1000)
    display=f"127.0.0.1:{number}"
    subprocess.run(["xauth","-f",str(auth),"add",display,"MIT-MAGIC-COOKIE-1",os.urandom(16).hex()],check=True)
    server=subprocess.Popen(["Xvfb",f":{number}","-screen","0","800x600x24","-listen","tcp","-nolisten","unix","-noreset","-auth",str(auth)],stdout=subprocess.DEVNULL,stderr=subprocess.PIPE)
    try:
        time.sleep(1)
        if server.poll() is not None:raise RuntimeError(server.stderr.read().decode())
        env=os.environ.copy();env.update(DISPLAY=display,XAUTHORITY=str(auth),XDG_SESSION_TYPE="x11",LOCALCONNECT_ISOLATED_X11="1")
        env.pop("WAYLAND_DISPLAY",None)
        subprocess.run(["cargo","test","--locked","--manifest-path",str(root/"rust/Cargo.toml"),"--test","remote_x11","--","--ignored","--nocapture"],env=env,check=True,timeout=60)
    finally:
        server.terminate();server.wait(timeout=5)
