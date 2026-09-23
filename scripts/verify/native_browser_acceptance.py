#!/usr/bin/env python3
"""Repeatable isolated acceptance check for native browser frame density and scrolling."""

import argparse
import base64
import functools
import http.server
import json
import os
from pathlib import Path
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time
import urllib.error
import urllib.request
import uuid

REPO = Path(__file__).resolve().parents[2]


class FixtureHandler(http.server.BaseHTTPRequestHandler):
    state = {"reports": [], "errors": []}

    def do_GET(self):
        body = FIXTURE_HTML.encode()
        self.send_response(200)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_POST(self):
        try:
            value = json.loads(self.rfile.read(int(self.headers.get("Content-Length", "0"))))
            value["received_at"] = time.monotonic()
            self.state["reports"].append(value)
        except Exception as error:
            self.state["errors"].append(str(error))
        self.send_response(204)
        self.end_headers()

    def log_message(self, *_args):
        pass

class StaticHandler(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *_args):
        pass


FIXTURE_HTML = r"""<!doctype html><meta charset=utf-8><title>Native scroll fixture</title>
<style>*{box-sizing:border-box}html,body{margin:0;width:100%;font:20px sans-serif}body{min-height:8000px;background:linear-gradient(#fff,#dceeff)}
header{position:sticky;top:0;background:#fff;padding:12px;border-bottom:2px solid #567;z-index:2}#status{font-weight:bold}
#nested{height:65px;overflow:auto;background:#e5f4ff;border:2px solid #578;margin-top:4px}#nested div{height:700px;padding:5px}
section{padding:24px;height:900px;border-bottom:2px solid #678}h1{margin:0}</style>
<header><h1>Native browser acceptance</h1><div id=status>waiting for routed native pointer</div><div id=nested><div>Nested scroll target</div></div></header>
<section><p>This fixture must fill the entire browser viewport without doubled CSS width.</p></section>
<section><p>Scroll target: 3900 CSS pixels in six seconds.</p></section>
<section><p>Repeated responsive content.</p></section><section><p>Repeated responsive content.</p></section>
<section><p>Repeated responsive content.</p></section><section><p>Repeated responsive content.</p></section>
<section><p>Repeated responsive content.</p></section><section><p>Repeated responsive content.</p></section>
<script>
const status=()=>document.querySelector('#status').textContent;
let scrollStart=null, scrollFinish=null;
const report=()=>fetch('/report',{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({innerWidth,innerHeight,dpr:devicePixelRatio,scrollY,nestedScrollTop:document.querySelector('#nested').scrollTop,nestedWheelY:window.nestedWheelY??null,ready:document.readyState,heading:document.querySelector('h1').textContent,headingWidth:document.querySelector('h1').getBoundingClientRect().width,contentText:document.querySelector('section p').textContent,status:status(),started:scrollStart!==null,scrollStart,scrollFinish})}).catch(()=>{});
addEventListener('load',report); addEventListener('resize',report);
let reportTimer;
addEventListener('scroll',()=>{clearTimeout(reportTimer);reportTimer=setTimeout(report,100)},{passive:true});
const nested=document.querySelector('#nested');
nested.addEventListener('wheel',event=>{window.nestedWheelY=event.deltaY},{passive:true});
nested.addEventListener('scroll',()=>{clearTimeout(reportTimer);reportTimer=setTimeout(report,100)},{passive:true});
let started=false;
addEventListener('pointerup',()=>{if(started)return;started=true;scrollStart=performance.now();document.querySelector('#status').textContent='scrolling';report();
 const begin=scrollStart, from=scrollY, duration=6000;
 const tick=now=>{const t=Math.min(1,(now-begin)/duration);scrollTo(0,from+3900*t);if(t<1)requestAnimationFrame(tick);else{scrollTo(0,3900);document.querySelector('header').style.background='#00a84b';scrollFinish=performance.now();document.querySelector('#status').textContent='scroll complete';report()}};
 requestAnimationFrame(tick);
},{once:true});
</script>"""


class WebDriver:
    def __init__(self, base, timeout=10):
        self.base, self.timeout, self.session, self.commands = base, timeout, None, 0
    def request(self, method, path, body=None):
        data = None if body is None else json.dumps(body).encode()
        request = urllib.request.Request(self.base + path, data=data, method=method,
                                         headers={"Content-Type": "application/json"})
        self.commands += 1
        try:
            with urllib.request.urlopen(request, timeout=self.timeout) as response:
                return json.loads(response.read() or b"{}")
        except urllib.error.HTTPError as error:
            raw = error.read()
            try:
                envelope = json.loads(raw)
                details = envelope.get("value", envelope) if isinstance(envelope, dict) else envelope
                if isinstance(details, dict):
                    details = {key: details[key][:1200] if isinstance(details.get(key), str) else details[key]
                               for key in ("error", "message", "stacktrace") if key in details}
                summary = json.dumps(details)[:1200]
            except (ValueError, TypeError):
                summary = raw.decode(errors="replace")[:1200]
            raise RuntimeError(f"WebDriver {method} {path} returned HTTP {error.code}: {summary}") from error
    def start(self, native):
        response = self.request("POST", "/session", {"capabilities": {"alwaysMatch": {
            "webkitgtk:browserOptions": {"binary": str(native), "useOverlayScrollbars": False}}}})
        self.session = response["value"]["sessionId"]
        return self.session

    def path(self, suffix):
        return f"/session/{self.session}{suffix}"

    def execute(self, expression):
        return self.request("POST", self.path("/execute/sync"), {"script": "return " + expression + ";", "args": []})["value"]

    def delete(self):
        if self.session:
            try:
                self.request("DELETE", self.path(""))
            except Exception:
                pass
            self.session = None


def wait_until(predicate, description, seconds, children=()):
    deadline = time.monotonic() + seconds
    last = None
    while time.monotonic() < deadline:
        for name, child in children:
            if child.poll() is not None:
                raise RuntimeError(f"{name} exited early ({child.returncode})")
        try:
            value = predicate()
            if value:
                return value
        except Exception as error:
            last = error
        time.sleep(0.1)
    raise RuntimeError(f"Timed out waiting for {description}" + (f": {last}" if last else ""))


def post_json(url, value, timeout=10):
    req = urllib.request.Request(url, json.dumps(value).encode(), {"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=timeout) as response:
        raw = response.read()
        return json.loads(raw) if raw else {}




def terminate_owned(children):
    result = []
    for label, child in reversed(children):
        if child.poll() is None:
            try:
                os.killpg(child.pid, signal.SIGTERM)
                child.wait(timeout=3)
            except (ProcessLookupError, subprocess.TimeoutExpired):
                try:
                    os.killpg(child.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                try:
                    child.wait(timeout=2)
                except subprocess.TimeoutExpired:
                    pass
        result.append({"name": label, "pid": child.pid, "returncode": child.poll()})
    return result


def require_binary(value, label):
    found = shutil.which(value) if "/" not in value else value
    if not found or not Path(found).is_file() or not os.access(found, os.X_OK):
        raise RuntimeError(f"Required {label} executable unavailable: {value}; set the corresponding -- option")
    return str(Path(found).resolve())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--native", default=str(REPO / "target/debug/cockpit-tauri"), help="native Cockpit executable")
    parser.add_argument("--herdr", default="herdr", help="Herdr executable")
    parser.add_argument("--gateway", default=str(REPO / "target/debug/cockpit"), help="Cockpit gateway executable")
    parser.add_argument("--driver", default="/usr/bin/WebKitWebDriver", help="WebKitWebDriver executable")
    parser.add_argument("--compositor", default="niri", help="Niri compositor executable")
    parser.add_argument("--driver-port", type=int, default=0)
    parser.add_argument("--width", type=int, default=1392)
    parser.add_argument("--height", type=int, default=835)
    parser.add_argument("--fps-min", type=float, default=15.0)
    parser.add_argument("--timeout", type=float, default=20)
    parser.add_argument("--skip-build", action="store_true", help="use supplied prebuilt binaries and frontend without rebuilding")
    parser.add_argument("--annotation", action="store_true", help="also exercise native region and note saves against the retained draft")
    parser.add_argument("--nested-wheel", action="store_true", help="also check repeated inner and outer wheel routing without claiming physical OS input")
    args = parser.parse_args()
    if not args.skip_build:
        for label, command, seconds in (
            ("frontend", ["bun", "run", "build"], 300),
            ("gateway owner", ["cargo", "build", "-p", "cockpit-host", "--bin", "cockpit"], 600),
            ("native app", ["cargo", "build", "-p", "cockpit-tauri"], 600),
        ):
            build = subprocess.run(command, cwd=REPO, capture_output=True, text=True, timeout=seconds)
            if build.returncode:
                raise RuntimeError(f"{label} build failed ({build.returncode}): {(build.stdout + build.stderr)[-2000:]}")
    bins = {"native": require_binary(args.native, "native Cockpit"), "herdr": require_binary(args.herdr, "Herdr"),
            "gateway": require_binary(args.gateway, "Cockpit gateway"), "driver": require_binary(args.driver, "WebKitWebDriver"),
            "compositor": require_binary(args.compositor, "Niri")}
    root = Path(tempfile.mkdtemp(prefix="cnative-", dir="/tmp")).resolve()
    for folder in ("config/herdr", "state", "data", "cache", "space", "www"):
        (root / folder).mkdir(parents=True, exist_ok=True)
    session = "native-" + uuid.uuid4().hex[:12]
    space_label = "Native browser " + uuid.uuid4().hex[:8]
    session_socket = root / f"config/herdr/sessions/{session}/herdr.sock"
    (root / "config/herdr/config.toml").write_text("")
    (root / "cockpit.toml").write_text(
        f'version = 1\nrepository_roots = ["{root}"]\nworktree_root = "{root}/worktrees"\n'
        f'companion_root = "{root}/companions"\nstate_root = "{root}/cockpit-state"\n')
    fixture = http.server.ThreadingHTTPServer(("127.0.0.1", 0), FixtureHandler)
    fixture_thread = threading.Thread(target=fixture.serve_forever, daemon=True)
    fixture_thread.start()
    fixture_url = f"http://127.0.0.1:{fixture.server_port}/"
    env = {k: v for k, v in os.environ.items() if not k.startswith(("HERDR_", "COCKPIT_"))}
    env.update(HOME=str(root), XDG_CONFIG_HOME=str(root / "config"), XDG_STATE_HOME=str(root / "state"),
               XDG_CACHE_HOME=str(root / "cache"), XDG_DATA_HOME=str(root / "data"),
               HERDR_CONFIG_PATH=str(root / "config/herdr/config.toml"), HERDR_SOCKET_PATH=str(session_socket),
               COCKPIT_CONFIG=str(root / "cockpit.toml"), COCKPIT_HERDR_SESSION=session, COCKPIT_HERDR_SOCKET=str(session_socket),
               COCKPIT_HERDR_EXECUTABLE=bins["herdr"])
    children, driver, frontend = [], None, None
    result = {"session": session, "space": space_label, "fixture_url": fixture_url, "root": str(root),
              "failure": None, "geometries": {}, "paints": {}, "cleanup": []}

    def launch(label, argv, runenv):
        log = (root / f"{label}.log").open("w")
        child = subprocess.Popen(argv, cwd=REPO, env=runenv, stdin=subprocess.DEVNULL,
                                 stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        children.append((label, child))
        return child

    try:
        dev_port = 5173  # The debug Tauri executable loads this fixed devUrl.
        if not (REPO / "dist/index.html").is_file():
            raise RuntimeError("Built frontend unavailable; run `bun run build` before native acceptance")
        try:
            frontend = http.server.ThreadingHTTPServer(
                ("127.0.0.1", dev_port),
                functools.partial(StaticHandler, directory=str(REPO / "dist")),
            )
        except OSError as error:
            raise RuntimeError(f"Tauri dev URL port 127.0.0.1:{dev_port} is occupied; refusing to stop a non-owned process") from error
        threading.Thread(target=frontend.serve_forever, daemon=True).start()
        wait_until(lambda: urllib.request.urlopen(f"http://127.0.0.1:{dev_port}/", timeout=1).status == 200,
                   "owned built frontend on Tauri dev URL port 5173", args.timeout)
        herdr = launch("herdr", [bins["herdr"], "--session", session, "server"], env)
        wait_until(lambda: session_socket.exists(), "private Herdr socket", args.timeout, [("Herdr", herdr)])
        space_cmd = [bins["herdr"], "--session", session, "workspace", "create", "--cwd", str(root / "space"), "--label", space_label, "--focus"]
        space_result = subprocess.run(space_cmd, cwd=REPO, env=env, check=True, text=True, capture_output=True, timeout=args.timeout)
        workspace = json.loads(space_result.stdout)
        space_id = workspace.get("result", {}).get("workspace", {}).get("workspace_id")
        if not space_id:
            raise RuntimeError(f"Herdr workspace create omitted workspace_id: {space_result.stdout}")
        result["space_id"] = space_id

        gateway = launch("gateway", [bins["gateway"], "serve", "--herdr-session", session,
            "--herdr-socket", str(session_socket), "--config", str(root / "cockpit.toml"),
            "--bind", "127.0.0.1:0", "--static-dir", str(REPO / "dist")], env)
        def gateway_ready():
            text = (root / "gateway.log").read_text(errors="replace")
            for line in text.splitlines():
                if "listening http://" in line:
                    return line.split("listening http://", 1)[1].strip().rstrip("/")
            return None
        gateway_hostport = wait_until(gateway_ready, "private gateway bind readiness", args.timeout, [("gateway", gateway)])
        gateway_url = "http://" + gateway_hostport
        wait_until(lambda: urllib.request.urlopen(gateway_url, timeout=1).status == 200,
                   "private gateway HTTP readiness", args.timeout, [("gateway", gateway)])
        action = {"target": {"session_id": session, "space_id": space_id, "pane_id": None,
                             "endpoint_path": str(session_socket)}, "action": {"kind": "open", "url": fixture_url}}
        post_json(gateway_url + "/api/v1/browser/action", action)

        config = root / "niri.kdl"
        config.write_text('output "Smithay Winit Unknown" { scale 2; }\n')
        compositor_env = dict(env)
        compositor_env.pop("DISPLAY", None)
        compositor = launch("compositor", [bins["compositor"], "-c", str(config)], compositor_env)
        logpath = root / "compositor.log"
        def compositor_env_ready():
            text = logpath.read_text(errors="replace") if logpath.exists() else ""
            wayland = next((line.split("listening on Wayland socket: ", 1)[1].strip() for line in text.splitlines() if "listening on Wayland socket: " in line), None)
            ipc = next((line.split("IPC listening on: ", 1)[1].strip() for line in text.splitlines() if "IPC listening on: " in line), None)
            return (wayland, ipc) if wayland and ipc else None
        wayland, niri_socket = wait_until(compositor_env_ready, "private Niri Wayland and IPC sockets", args.timeout, [("compositor", compositor)])
        app_env = dict(env, WAYLAND_DISPLAY=wayland, NIRI_SOCKET=niri_socket)
        app_env.pop("DISPLAY", None)
        launcher = root / "native-launch.sh"
        launcher.write_text("#!/bin/sh\n" + "".join(f"export {key}={json.dumps(app_env[key])}\n" for key in (
            "HOME", "XDG_CONFIG_HOME", "XDG_STATE_HOME", "XDG_CACHE_HOME", "XDG_DATA_HOME", "HERDR_CONFIG_PATH",
            "HERDR_SOCKET_PATH", "COCKPIT_CONFIG", "COCKPIT_HERDR_SESSION", "COCKPIT_HERDR_SOCKET", "COCKPIT_HERDR_EXECUTABLE",
            "WAYLAND_DISPLAY", "NIRI_SOCKET")) + "unset HERDR_SESSION HERDR_NAME DISPLAY\nexport TAURI_WEBVIEW_AUTOMATION=true\nexec " + json.dumps(bins["native"]) + "\n")
        launcher.chmod(0o700)
        with socket.socket() as reserve:
            reserve.bind(("127.0.0.1", 0))
            driver_port = args.driver_port or reserve.getsockname()[1]
        driver_process = launch("webdriver", [bins["driver"], f"--port={driver_port}", "--host=127.0.0.1"], app_env)
        webdriver = WebDriver(f"http://127.0.0.1:{driver_port}", args.timeout)
        wait_until(lambda: webdriver.request("GET", "/status").get("value", {}).get("ready"),
                   "WebKitWebDriver readiness", args.timeout, [("WebKitWebDriver", driver_process)])
        webdriver.start(launcher)
        driver = webdriver
        webdriver.request("POST", webdriver.path("/window/rect"), {"width": args.width, "height": args.height})
        # Niri may initially tile the driver window; make only this private column full-width.
        subprocess.run([bins["compositor"], "msg", "action", "set-column-width", "100%"],
                       env=dict(app_env, NIRI_SOCKET=niri_socket), check=True, timeout=5,
                       stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        result["geometries"]["outer_requested"] = {"width": args.width, "height": args.height, "dpr": 2}
        last_outer, stable_outer_samples = None, 0
        def fixed_outer():
            nonlocal last_outer, stable_outer_samples
            raw = webdriver.execute("({w:innerWidth,h:innerHeight,d:devicePixelRatio})")
            observed = {"width": raw["w"], "height": raw["h"], "dpr": raw["d"]}
            result["geometries"]["outer_last_observed"] = observed
            if observed["width"] >= 320 and observed["height"] >= 240 and observed["dpr"] == 2:
                if observed == last_outer:
                    stable_outer_samples += 1
                else:
                    last_outer, stable_outer_samples = observed, 1
                if stable_outer_samples >= 3:
                    result["geometries"]["outer"] = observed
                    return observed
            else:
                last_outer, stable_outer_samples = observed, 0
            return None
        wait_until(fixed_outer, "stable fixed DPR2 app viewport (at least 320×240)", args.timeout,
                   [("WebKitWebDriver", driver_process)])
        # Open the uniquely named Space and its browser canvas; no splitter is moved during this sequence.
        webdriver.execute("(()=>{window.__nativeAcceptanceErrors=[];addEventListener('error',e=>window.__nativeAcceptanceErrors.push(String(e.message||e.error||'window error').slice(0,300)));addEventListener('unhandledrejection',e=>window.__nativeAcceptanceErrors.push(String(e.reason).slice(0,300)));return true})()")
        space_selector = "(()=>[...document.querySelectorAll('button.resource-select')].some(x=>x.title===" + json.dumps(space_label) + "))()"
        try:
            wait_until(lambda: webdriver.execute(space_selector), "unique Space selector after frontend session load",
                       args.timeout, [("WebKitWebDriver", driver_process)])
        except RuntimeError as error:
            diagnostic = "(()=>({href:location.href,title:document.title,bodyText:(document.body?.innerText||'').slice(0,500),resourceSelectTitles:[...document.querySelectorAll('button.resource-select')].slice(0,20).map(x=>x.title),sessionSpaceButtons:[...document.querySelectorAll('button')].filter(x=>/session|space/i.test((x.title||'')+' '+(x.getAttribute('aria-label')||''))).slice(0,20).map(x=>({title:x.title,label:x.getAttribute('aria-label'),text:x.innerText.slice(0,100)})),javascriptErrors:window.__nativeAcceptanceErrors||[] }))()"
            try:
                result["failure_diagnostics"] = {"space_selector": webdriver.execute(diagnostic)}
            except Exception as diagnostic_error:
                result["failure_diagnostics"] = {"space_selector_error": str(diagnostic_error)[:1200]}
            raise RuntimeError(f"{error}; app diagnostics: {json.dumps(result['failure_diagnostics'])}") from error
        webdriver.execute("(()=>{const b=[...document.querySelectorAll('button.resource-select')].find(x=>x.title===" + json.dumps(space_label) + ");b.click();return true})()")
        space_selected = "(()=>{const b=[...document.querySelectorAll('button.resource-select')].find(x=>x.title===" + json.dumps(space_label) + ");return !!b?.closest('.resource-row')?.classList.contains('is-selected')})()"
        wait_until(lambda: webdriver.execute(space_selected), "unique Space selection", args.timeout,
                   [("WebKitWebDriver", driver_process)])
        # The toolbar's "Open browser" label is a toggle: once the preloaded
        # association arrives it changes to "Close browser". Use the stable
        # command action instead so an asynchronous state update cannot close it.
        webdriver.execute("(()=>{const b=[...document.querySelectorAll('.tab-strip-actions button')].find(x=>x.textContent.includes('Commands'));if(!b)throw Error('Commands action absent');b.click();return true})()")
        wait_until(lambda: webdriver.execute("!!document.querySelector('.command-footer button')"),
                   "command palette readiness", args.timeout, [("WebKitWebDriver", driver_process)])
        webdriver.execute("(()=>{document.querySelector('.command-footer button').click();return true})()")
        browser_action = "(()=>{const b=[...document.querySelectorAll('button.command-row')].find(x=>x.textContent.trim()==='Open browser for Space');return !!b&&!b.disabled})()"
        wait_until(lambda: webdriver.execute(browser_action), "enabled browser open command",
                   args.timeout, [("WebKitWebDriver", driver_process)])
        webdriver.execute("(()=>{[...document.querySelectorAll('button.command-row')].find(x=>x.textContent.trim()==='Open browser for Space').click();return true})()")
        def first_canvas():
            probe = webdriver.execute("(()=>{const c=document.querySelector('canvas.browser-frame'),r=c?.getBoundingClientRect();return {painted:!!c && document.querySelector('.browser-toolbar-status')?.textContent==='Live browser view' && c.width!==300 && c.height!==150,status:document.querySelector('.browser-toolbar-status')?.textContent,canvas:{width:c?.width||0,height:c?.height||0,left:r?.left||0,top:r?.top||0,widthCss:r?.width||0,heightCss:r?.height||0},viewport:{width:innerWidth,height:innerHeight,dpr:devicePixelRatio},surface:!!document.querySelector('.browser-surface'),body:(document.body?.innerText||'').slice(-600)}})()")
            result["geometries"]["first_canvas_last_observed"] = probe
            return probe if probe["painted"] and probe["canvas"]["widthCss"] > 0 and probe["canvas"]["heightCss"] > 0 else None
        geometry = wait_until(first_canvas, "first native browser canvas frame paint", args.timeout,
                              [("WebKitWebDriver", driver_process)])
        metrics = wait_until(lambda: next((report for report in reversed(FixtureHandler.state["reports"])
            if report.get("ready") == "complete"
            and abs(report.get("innerWidth", 0) - geometry["canvas"]["widthCss"]) <= 2
            and abs(report.get("innerHeight", 0) - geometry["canvas"]["heightCss"]) <= 2), None),
            "fixture viewport report matching the first canvas paint", args.timeout, [("WebKitWebDriver", driver_process)])
        density = {"bitmap_width_per_css": geometry["canvas"]["width"] / geometry["canvas"]["widthCss"],
                   "bitmap_height_per_css": geometry["canvas"]["height"] / geometry["canvas"]["heightCss"]}
        result["geometries"].update({"first_canvas_viewport": geometry["viewport"],
            "native_canvas": geometry["canvas"], "fixture_first_paint": metrics,
            "bitmap_density": density, "splitter_moved": False})
        first_screenshot = webdriver.request("GET", webdriver.path("/screenshot"))["value"]
        first_screenshot_path = root / "native-first.png"
        first_screenshot_path.write_bytes(base64.b64decode(first_screenshot))
        result["screenshots"] = {"first_paint_before_gesture": str(first_screenshot_path)}
        if geometry["viewport"] != result["geometries"].get("outer"):
            raise RuntimeError(f"native outer viewport changed before browser view attach: {geometry['viewport']}")
        if abs(metrics["dpr"] - 2) > .01 or abs(metrics["innerWidth"] - geometry["canvas"]["widthCss"]) > 2 or abs(metrics["innerHeight"] - geometry["canvas"]["heightCss"]) > 2:
            raise RuntimeError(f"fixture first-paint CSS geometry/DPR disagrees with native canvas: {metrics} vs {geometry['canvas']}")
        if metrics.get("heading") != "Native browser acceptance" or "without doubled CSS width" not in metrics.get("contentText", "") or metrics.get("headingWidth", 0) < metrics["innerWidth"] * .9:
            raise RuntimeError(f"first-paint CSS text/width report is incomplete or distorted: {metrics}")
        if abs(density["bitmap_width_per_css"] - 2) > .05 or abs(density["bitmap_height_per_css"] - 2) > .05:
            raise RuntimeError(f"distorted first paint: canvas bitmap density is not DPR2: {density}")
        if args.annotation:
            webdriver.execute("(()=>{const s=document.querySelector('.browser-surface');if(!s)throw Error('browser surface unavailable');s.setPointerCapture=()=>{};s.hasPointerCapture=()=>false;s.releasePointerCapture=()=>{};return true})()")
            def draft_opened():
                listed = post_json(gateway_url + "/api/v1/browser/drafts/recovery",
                                   {"target": action["target"], "action": {"type": "list"}})
                return listed.get("type") == "draft_inventory" and any(
                    draft.get("revision", 0) >= 1 for draft in listed.get("inventory", {}).get("drafts", []))
            wait_until(draft_opened, "authoritative native draft open", 5)
            # The store may acknowledge slightly before the native view consumes its open response.
            time.sleep(0.2)
            webdriver.execute("(()=>{const b=document.querySelector('button[aria-label=\"Region\"]');if(!b)throw Error('region tool unavailable');b.click();return b.getAttribute('aria-pressed')})()")
            wait_until(lambda: webdriver.execute("document.querySelector('.browser-pane')?.classList.contains('browser-tool-region')"),
                       "native region tool selection", 3)
            gesture = webdriver.execute("(()=>{const c=document.querySelector('canvas.browser-frame'),s=document.querySelector('.browser-surface'),r=c.getBoundingClientRect(),x=r.left+100,y=r.top+35;for(const [type,dx,dy,buttons] of [['pointerdown',0,0,1],['pointerup',120,55,0]])s.dispatchEvent(new PointerEvent(type,{bubbles:true,cancelable:true,pointerId:19,pointerType:'mouse',clientX:x+dx,clientY:y+dy,button:0,buttons}));return {x,y}})()")
            def annotation_state():
                return webdriver.execute("(()=>({notes:document.querySelector('.browser-annotation-notes')?.getAttribute('aria-label'),marks:document.querySelectorAll('.browser-annotation-region').length,status:document.querySelector('.browser-toolbar-status')?.textContent,retained:!!document.querySelector('[aria-label=\"Retry retained annotation changes\"]')}))()")
            try:
                wait_until(lambda: (state if state["marks"] == 1 and state["notes"] == "Notes 1" and not state["retained"] else None)
                           if (state := annotation_state()) else None, "native region acknowledgement", 5)
            except RuntimeError as error:
                result["annotation_wait_error"] = str(error)
            state = annotation_state()
            inventory = post_json(gateway_url + "/api/v1/browser/drafts/recovery",
                                  {"target": action["target"], "action": {"type": "list"}})
            drafts = inventory.get("inventory", {}).get("drafts", []) if inventory.get("type") == "draft_inventory" else []
            result["annotation"] = {"gesture": gesture, "ui": state,
                                    "inventory": [{"draft_id": draft.get("draft_id"), "revision": draft.get("revision"),
                                                   "annotations": [{"id": mark.get("id"), "kind": mark.get("kind"),
                                                                    "bounds": mark.get("bounds")} for mark in draft.get("annotations", [])]}
                                                  for draft in drafts]}
            if state["marks"] != 1 or state["notes"] != "Notes 1" or state["retained"] or not any(
                len(draft["annotations"]) == 1 and draft["annotations"][0]["kind"] == "region"
                for draft in result["annotation"]["inventory"]
            ):
                raise RuntimeError(f"native annotation save was not acknowledged by UI and store: {result['annotation']}")
            screenshot = webdriver.request("GET", webdriver.path("/screenshot"))["value"]
            (root / "native-annotation.png").write_bytes(base64.b64decode(screenshot))
            result["screenshots"]["annotation_acknowledged"] = str(root / "native-annotation.png")
            wait_until(lambda: webdriver.execute("!!document.querySelector('.browser-note-editor textarea[aria-label=\"Annotation note\"]')"),
                       "native annotation note editor", 3)
            note_text = f"Native note {session}"
            entered = webdriver.execute("(()=>{const t=document.querySelector('.browser-note-editor textarea');const setter=Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,'value').set;setter.call(t,"
                                        + json.dumps(note_text) + ");t.dispatchEvent(new Event('input',{bubbles:true}));return t.value})()")
            if entered != note_text:
                raise RuntimeError(f"native note editor returned the wrong text: {entered!r}")
            webdriver.execute("(()=>{const b=[...document.querySelectorAll('.browser-note-editor button')].find(x=>x.textContent.trim()==='Save');if(!b)throw Error('annotation Save button unavailable');b.click();return true})()")
            def note_saved():
                listing = post_json(gateway_url + "/api/v1/browser/drafts/recovery",
                                    {"target": action["target"], "action": {"type": "list"}})
                return next((draft for draft in listing.get("inventory", {}).get("drafts", [])
                    if draft.get("draft_id") == drafts[0]["draft_id"] and draft.get("revision", 0) >= 3
                    and len(draft.get("annotations", [])) == 1
                    and draft["annotations"][0].get("comment") == note_text), None)
            saved_note = wait_until(note_saved, "durable native annotation note", 5)
            wait_until(lambda: webdriver.execute("!document.querySelector('.browser-note-editor') && !document.querySelector('[aria-label=\"Retry retained annotation changes\"]')"),
                       "native note acknowledgement and closed editor", 5)
            result["annotation"]["saved_note"] = {"draft_id": saved_note["draft_id"],
                "revision": saved_note["revision"], "annotation_id": saved_note["annotations"][0]["id"],
                "comment": saved_note["annotations"][0]["comment"]}
            screenshot = webdriver.request("GET", webdriver.path("/screenshot"))["value"]
            (root / "native-annotation-note.png").write_bytes(base64.b64decode(screenshot))
            result["screenshots"]["note_acknowledged"] = str(root / "native-annotation-note.png")
            webdriver.execute("(()=>{const b=document.querySelector('button[aria-label=\"Browse\"]');if(!b)throw Error('browse tool unavailable');b.click();return true})()")
            wait_until(lambda: webdriver.execute("document.querySelector('.browser-pane')?.classList.contains('browser-tool-browse')"),
                       "native browsing restored after region save", 3)
        result["activity"] = {"webdriver_commands_including_polling": webdriver.commands,
                              "excluded_from_fps": "WebDriver/CDP requests and helper activity",
                              "paint_rate_derived_from": "CanvasRenderingContext2D.drawImage timestamps only"}
        # Count actual drawImage calls on the displayed canvas, not WebDriver polling or helper activity.
        webdriver.execute("(()=>{const c=document.querySelector('canvas.browser-frame');if(!c)throw Error('canvas disappeared');const p=CanvasRenderingContext2D.prototype,d=p.drawImage;window.__nativePaintTimes=[];p.drawImage=function(...a){if(this.canvas===c)window.__nativePaintTimes.push(performance.now());return d.apply(this,a)};const s=document.querySelector('.browser-surface');if(!s)throw Error('browser surface absent');s.setPointerCapture=()=>{};s.hasPointerCapture=()=>false;s.releasePointerCapture=()=>{};return true})()")
        webdriver.execute("(()=>{const c=document.querySelector('canvas.browser-frame'),s=document.querySelector('.browser-surface'),r=c.getBoundingClientRect(),x=r.left+100,y=r.top+30;for(const type of ['pointerdown','pointerup'])s.dispatchEvent(new PointerEvent(type,{bubbles:true,cancelable:true,pointerId:1,pointerType:'mouse',clientX:x,clientY:y,button:0,buttons:type==='pointerdown'?1:0}));return {x,y}})()")
        completed = wait_until(lambda: next((report for report in reversed(FixtureHandler.state["reports"])
            if report.get("status") == "scroll complete" and report.get("started")
            and report.get("scrollFinish") is not None and report.get("scrollY", 0) >= 3899), None),
            "fixture scroll completion and final position 3900", 10, [("WebKitWebDriver", driver_process)])
        started_report = next((report for report in FixtureHandler.state["reports"]
                               if report.get("started") and report.get("status") == "scrolling"), None)
        if not started_report:
            raise RuntimeError("fixture did not report the routed pointer starting the scroll")
        duration = completed["received_at"] - started_report["received_at"]
        scroll_duration = completed["scrollFinish"] - completed["scrollStart"]
        paint_times = webdriver.execute("window.__nativePaintTimes||[]")
        fps = len(paint_times) / duration if duration > 0 else 0
        result["paints"] = {"source": "canvas.browser-frame CanvasRenderingContext2D.drawImage", "timestamps": len(paint_times),
                             "interval_seconds": duration, "fixture_animation_ms": scroll_duration, "fps": fps,
                             "threshold_fps_exclusive": args.fps_min, "final_scroll_y": completed.get("scrollY")}
        result["activity"]["webdriver_commands_including_polling"] = webdriver.commands
        if abs(scroll_duration - 6000) > 250:
            raise RuntimeError(f"fixture scroll animation did not sustain its fixed six-second duration: {scroll_duration:.0f} ms")
        if fps <= args.fps_min:
            raise RuntimeError(f"actual native canvas drawImage presentation rate {fps:.2f} FPS is not greater than {args.fps_min}")
        final_marker = wait_until(
            lambda: (lambda rgb: rgb if rgb[1] >= 100 and rgb[0] < 80 and rgb[2] < 130 else None)(
                webdriver.execute("(()=>{const c=document.querySelector('canvas.browser-frame');return [...c.getContext('2d').getImageData(4,4,1,1).data].slice(0,3)})()")),
            "completed scroll marker painted on native canvas", 3, [("WebKitWebDriver", driver_process)])
        result["paints"]["visible_final_marker_rgb"] = final_marker
        if args.nested_wheel:
            # The WebKit driver cannot send OS wheel actions on this compositor.
            # These are DOM-dispatched Cockpit surface events, then real helper/CDP page scrolls.
            wheel_steps = (("nested forward", 180, 100, 180, 3900),
                           ("nested reverse", -70, 100, 110, 3900),
                           ("outer forward", 160, 166, 110, 4060),
                           ("outer reverse", -90, 166, 110, 3970))
            routed = []
            previous_paints = len(paint_times)
            for name, delta, y_offset, inner_expected, outer_expected in wheel_steps:
                before = time.monotonic()
                position = webdriver.execute("(()=>{const c=document.querySelector('canvas.browser-frame'),s=document.querySelector('.browser-surface'),r=c.getBoundingClientRect(),x=r.left+120,y=r.top+"
                    + str(y_offset) + ";s.dispatchEvent(new WheelEvent('wheel',{bubbles:true,cancelable:true,clientX:x,clientY:y,deltaX:0,deltaY:"
                    + str(delta) + ",deltaMode:0}));return {x,y}})()")
                report = wait_until(lambda: next((item for item in reversed(FixtureHandler.state["reports"])
                    if item["received_at"] >= before
                    and abs(item.get("nestedScrollTop", -1000) - inner_expected) <= 3
                    and abs(item.get("scrollY", -1000) - outer_expected) <= 3), None),
                    f"{name} authoritative nested/document scroll", 5, [("WebKitWebDriver", driver_process)])
                def visible_paint():
                    current = webdriver.execute("window.__nativePaintTimes.length")
                    return current if current > previous_paints else None
                previous_paints = wait_until(visible_paint, f"{name} painted native frame", 3,
                                             [("WebKitWebDriver", driver_process)])
                host_scroll = webdriver.execute("window.scrollY")
                if host_scroll:
                    raise RuntimeError(f"{name} scrolled the Cockpit host: {host_scroll}")
                routed.append({"step": name, "delta_y_css": delta, "surface_position": position,
                               "page_scroll_y": report["scrollY"], "nested_scroll_top": report["nestedScrollTop"],
                               "nested_wheel_y": report.get("nestedWheelY"), "paint_count": previous_paints,
                               "host_scroll_y": host_scroll})
            result["nested_wheel"] = routed
            # Chromium may report a DPR-scaled DOM delta while default scrolling
            # still moves by the requested CSS pixels. Assert routing and sign,
            # not equality of CDP's synthetic event delta with natural movement.
            if any(step["nested_wheel_y"] is None or step["nested_wheel_y"] * step["delta_y_css"] <= 0
                   for step in routed[:2]):
                raise RuntimeError(f"nested wheel gesture did not reach the intended page container: {routed}")
        # Preserve visual evidence from the actual native WebKit window.
        screenshot = webdriver.request("GET", webdriver.path("/screenshot"))["value"]
        (root / "native-window.png").write_bytes(base64.b64decode(screenshot))
        result["screenshot"] = str(root / "native-window.png")
        result["ok"] = True
    except Exception as error:
        result["failure"] = f"{type(error).__name__}: {error}"
        result["ok"] = False
        if driver:
            try:
                failure_screenshot = root / "native-failure.png"
                failure_screenshot.write_bytes(base64.b64decode(driver.request("GET", driver.path("/screenshot"))["value"]))
                result["failure_screenshot"] = str(failure_screenshot)
            except Exception:
                pass
    finally:
        if driver:
            driver.delete()
            result.setdefault("activity", {})["webdriver_commands_including_polling"] = driver.commands
        if frontend:
            frontend.shutdown()
            frontend.server_close()
            result["cleanup"].append({"name": "built-frontend-http-server", "stopped": True})
        fixture.shutdown()
        fixture.server_close()
        result["cleanup"].extend(terminate_owned(children))
        result["cleanup"].append({"name": "fixture-http-server", "stopped": True})
        result["logs"] = {p.stem: str(p) for p in root.glob("*.log")}
        print(json.dumps(result, indent=2))
    return 0 if result.get("ok") else 1


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (RuntimeError, OSError, subprocess.SubprocessError, urllib.error.URLError) as error:
        print(json.dumps({"ok": False, "failure": str(error)}), file=sys.stderr)
        raise SystemExit(1)
