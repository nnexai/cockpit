#!/usr/bin/env python3
"""Repeatable isolated acceptance check for native browser frame density and scrolling."""

import argparse
import base64
import hashlib
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
    state = {"reports": [], "errors": [], "hidden_scroll_armed": False, "raster_marker": False}

    def do_GET(self):
        control = self.path == "/control"
        body = json.dumps({"hidden_scroll_armed": self.state["hidden_scroll_armed"]}).encode() if control else (FIXTURE_HTML + (RASTER_MARKER_HTML if self.state["raster_marker"] else "")).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json" if control else "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_POST(self):
        if self.path == "/arm-hidden-scroll":
            self.state["hidden_scroll_armed"] = True
            self.send_response(204)
            self.end_headers()
            return
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
#nested{height:65px;overflow:auto;background:#e5f4ff;border:2px solid #578;margin-top:4px}#nested div{height:700px;padding:5px;background:repeating-linear-gradient(#e5f4ff 0 22px,#9fc5dd 22px 44px)}
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
const sustainedMs=Number(new URLSearchParams(location.search).get('sustained')||0)*1000;
addEventListener('pointerup',()=>{if(started)return;started=true;scrollStart=performance.now();document.querySelector('#status').textContent='scrolling';report();
 const begin=scrollStart, from=scrollY, duration=sustainedMs||6000;
 const tick=now=>{const t=Math.min(1,(now-begin)/duration);scrollTo(0,sustainedMs ? 1700-700*Math.cos((now-begin)*Math.PI/4000) : from+3900*t);if(t<1)requestAnimationFrame(tick);else{scrollTo(0,3900);document.querySelector('header').style.background='#00a84b';scrollFinish=performance.now();document.querySelector('#status').textContent='scroll complete';report();if(new URLSearchParams(location.search).has('hiddenScroll')){let mutated=false;const poll=setInterval(()=>{fetch('/control').then(r=>r.json()).then(value=>{if(mutated||!value.hidden_scroll_armed)return;mutated=true;clearInterval(poll);scrollTo(0,4100);document.querySelector('header').style.background='#1645ad';document.querySelector('#status').textContent='hidden scroll complete';report();setInterval(report,500)}).catch(()=>{})},250)}}};
 requestAnimationFrame(tick);
},{once:true});
</script>"""


RASTER_MARKER_HTML = r"""<style>#raster-marker{position:fixed;left:400px;top:5px;width:120px;height:80px;background:#f000f0;color:white;z-index:200;font:bold 40px sans-serif;text-align:center;line-height:80px}</style>
<div id=raster-marker>G0</div><script>window.__rasterAdvance=()=>{const marker=document.querySelector('#raster-marker');marker.textContent='G1';marker.style.background='#00f0f0';return marker.textContent;};</script>"""

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


def choose_browser_command(driver, label, children):
    """Click an actual Commands row; do not call private React callbacks."""
    driver.execute("(()=>{const trigger=[...document.querySelectorAll('button')].find(b=>b.textContent.trim()==='Commands');if(!trigger)throw Error('Commands trigger unavailable');trigger.click();return true})()")
    wait_until(lambda: driver.execute("!!document.querySelector('input[aria-label=\"Find a command\"]')"),
               "native Commands search field", 3, children)
    driver.execute("(()=>{const input=document.querySelector('input[aria-label=\"Find a command\"]');const setter=Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set;setter.call(input,"
                   + json.dumps(label) + ");input.dispatchEvent(new Event('input',{bubbles:true}));return true})()")
    option = "[...document.querySelectorAll('button[role=\"option\"]')].find(b=>b.querySelector('.command-row-label')?.textContent.trim()===" + json.dumps(label) + ")"
    wait_until(lambda: driver.execute("(()=>{const row=" + option + ";return !!row&&!row.disabled})()"),
               f"enabled native Commands row {label}", 3, children)
    driver.execute("(()=>{const row=" + option + ";if(!row||row.disabled)throw Error('Commands row unavailable');row.click();return true})()")




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


def sample_owned_processes(children):
    """Linux /proc PSS KiB and CPU ticks for this run's live process trees."""
    roots = [("acceptance-runner", os.getpid())] + [(label, child.pid) for label, child in children]
    seen, by_root = set(), {}
    for label, root_pid in roots:
        stack, pss_kib, cpu_ticks, count, processes = [root_pid], 0, 0, 0, []
        while stack:
            pid = stack.pop()
            if pid in seen:
                continue
            seen.add(pid)
            proc = Path("/proc") / str(pid)
            try:
                stat = (proc / "stat").read_text().rsplit(") ", 1)[1].split()
                comm = (proc / "comm").read_text().strip()
                memory = (proc / "smaps_rollup").read_text()
                pss = next(int(line.split()[1]) for line in memory.splitlines() if line.startswith("Pss:"))
                descendants = ((proc / "task" / str(pid) / "children").read_text().split()
                               if label != "acceptance-runner" else [])
            except (OSError, IndexError, StopIteration, ValueError):
                continue  # A child exited while this sample was taken.
            stack.extend(int(child) for child in descendants)
            count += 1
            pss_kib += pss
            ticks = int(stat[11]) + int(stat[12])
            cpu_ticks += ticks
            processes.append({"pid": pid, "comm": comm, "start_ticks": int(stat[19]),
                              "pss_kib": pss, "cpu_ticks": ticks})
        by_root[label] = {"pss_kib": pss_kib, "cpu_ticks": cpu_ticks, "process_count": count,
                          "processes": processes}
    return {"pss_kib": sum(part["pss_kib"] for part in by_root.values()),
            "cpu_ticks": sum(part["cpu_ticks"] for part in by_root.values()),
            "process_count": sum(part["process_count"] for part in by_root.values()),
            "roots": by_root}

def owned_helper_processes(helper_path):
    """Only return live PIDs whose argv names this fixture's helper module."""
    marker = os.fsencode(helper_path)
    found = []
    for proc in Path("/proc").iterdir():
        if not proc.name.isdigit():
            continue
        try:
            if marker not in (proc / "cmdline").read_bytes().split(b"\0"):
                continue
            stat = (proc / "stat").read_text().rsplit(") ", 1)[1].split()
            found.append({"pid": int(proc.name), "start_ticks": int(stat[19]),
                          "comm": (proc / "comm").read_text().strip()})
        except (OSError, IndexError, ValueError):
            continue
    return sorted(found, key=lambda item: item["pid"])

def traced_helper_source(source, capture_lanes=False, frame_publication=False, skip_screenshot_restore=False, raster_generation=False, physical_screencast_ceiling=False, snapshot_throughput_probe=False):
    """Instrument only a disposable helper copy, leaving packaged source intact."""
    import_line = "import readline from 'node:readline';\n"
    original = """    await expectedCdp.send('Emulation.setDeviceMetricsOverride', {
      width: requested.width,
      height: requested.height,
      deviceScaleFactor: requested.dpr,
      mobile: false,
      screenWidth: Math.max(1, Math.round(requested.width * requested.dpr)),
      screenHeight: Math.max(1, Math.round(requested.height * requested.dpr)),
    });"""
    replacement = r"""    const metricsOverride = {
      width: requested.width,
      height: requested.height,
      deviceScaleFactor: requested.dpr,
      mobile: false,
      screenWidth: Math.max(1, Math.round(requested.width * requested.dpr)),
      screenHeight: Math.max(1, Math.round(requested.height * requested.dpr)),
    };
    const traceOverride = (phase, error = null) => {
      try {
        const stat = readFileSync('/proc/self/stat', 'utf8').split(') ')[1].trim().split(/\s+/);
        appendFileSync(process.env.COCKPIT_BROWSER_TRACE_PATH, JSON.stringify({
          phase, monotonic_ms: Number(process.hrtime.bigint() / 1000000n),
          pid: process.pid, start_ticks: Number(stat[19]),
          target_id: state?.targetId ?? null, page_binding_generation: expectedBinding,
          commit_request: commitRequest, requested: metricsOverride,
          error: error ? String(error.message || error).slice(0, 180) : null,
        }) + '\n', { mode: 0o600 });
      } catch {}
    };
    traceOverride('send');
    try {
      await expectedCdp.send('Emulation.setDeviceMetricsOverride', metricsOverride);
      traceOverride('accepted');
    } catch (error) {
      traceOverride('failed', error);
      throw error;
    }"""
    if source.count(import_line) != 1 or source.count(original) != 1:
        raise RuntimeError("packaged helper CDP override source changed; refusing unmatched diagnostic instrumentation")
    fs_import = "import { appendFileSync, readFileSync, writeFileSync } from 'node:fs';\n"
    instrumented = source.replace(import_line, import_line + fs_import).replace(original, replacement)
    if physical_screencast_ceiling:
        screencast_site = "        format: 'jpeg', quality: 80,\n        everyNthFrame: 1,\n"
        if instrumented.count(screencast_site) != 1:
            raise RuntimeError("packaged helper screencast options changed; refusing unmatched diagnostic instrumentation")
        instrumented = instrumented.replace(screencast_site, screencast_site + "        maxWidth: Math.round(state.captureBaseline.cssWidth * state.captureBaseline.dpr),\n        maxHeight: Math.round(state.captureBaseline.cssHeight * state.captureBaseline.dpr),\n", 1)
    if not capture_lanes:
        return instrumented
    lane_trace = r"""
const captureLaneCounts = { screencast: 0, screenshot_send: 0, screenshot_accepted: 0, density_result: 0, physical_published: 0 };
function traceCaptureLane(lane, geometry = null) {
  const count = ++captureLaneCounts[lane];
  if (lane === 'density_result' || lane === 'physical_published' ? count > 16 : count !== 1 && (count & (count - 1)) !== 0) return;
  try {
    const stat = readFileSync('/proc/self/stat', 'utf8').split(') ')[1].trim().split(/\s+/);
    appendFileSync(process.env.COCKPIT_BROWSER_TRACE_PATH, JSON.stringify({
      phase: 'capture_lane', lane, count, geometry,
      monotonic_ms: Number(process.hrtime.bigint() / 1000000n),
      pid: process.pid, start_ticks: Number(stat[19]),
      target_id: state?.targetId ?? null, page_binding_generation: pageBindingGeneration,
      capture_token: state?.captureToken ?? null,
    }) + '\n', { mode: 0o600 });
  } catch {}
}
"""
    screencast_site = "  screencastListener = (frame) => {\n"
    screenshot_site = "    const capture = await cdp.send('Page.captureScreenshot', options);\n"
    if instrumented.count(fs_import) != 1 or instrumented.count(screencast_site) != 1 or instrumented.count(screenshot_site) != 1:
        raise RuntimeError("packaged helper capture lanes changed; refusing unmatched diagnostic instrumentation")
    instrumented = (instrumented.replace(fs_import, fs_import + lane_trace, 1)
                    .replace(screencast_site, screencast_site + "    traceCaptureLane('screencast');\n", 1)
                    .replace(screenshot_site,
                             "    traceCaptureLane('screenshot_send');\n" + screenshot_site
                             + "    traceCaptureLane('screenshot_accepted', typeof capture.data === 'string' ? encodedJpegDimensions(Buffer.from(capture.data, 'base64')) : null);\n", 1))
    result_site = "      result = await captureStableDensityFrame(current);\n"
    if instrumented.count(result_site) != 1:
        raise RuntimeError("packaged helper density result source changed; refusing unmatched diagnostic instrumentation")
    instrumented = instrumented.replace(result_site, result_site + "      traceCaptureLane('density_result', { result, expected_frame: current.expectedFrameSequence, current_frame: state?.frameSequence, expected_arrival: current.expectedStreamArrivalSequence, current_arrival: screencastFrameArrivalSequence, attempts: state?.densityRefinementAttempts, expected_revision: current.baseline.viewportRevision, current_revision: state?.captureBaseline?.viewportRevision });\n", 1)
    if frame_publication:
        counter_site = "screencast: 0, screenshot_send: 0, screenshot_accepted: 0, density_result: 0, physical_published: 0"
        publication_site = "    emit({ type: 'frame', descriptor });\n"
        if instrumented.count(counter_site) != 1 or instrumented.count(publication_site) != 1:
            raise RuntimeError("packaged helper frame publication changed; refusing unmatched diagnostic instrumentation")
        instrumented = (instrumented.replace(counter_site, counter_site + ", frame_published: 0", 1)
                        .replace(publication_site, publication_site + "    if (frame._screenshotCapture) traceCaptureLane('physical_published', { frame_sequence: descriptor.frame_sequence, viewport_revision: descriptor.viewport_revision, document_generation: descriptor.document_generation, image_width: descriptor.image_width, scroll_y: descriptor.scroll_y });\n    traceCaptureLane('frame_published', { image_width: descriptor.image_width, image_height: descriptor.image_height, viewport_css_width: descriptor.viewport_css_width, viewport_css_height: descriptor.viewport_css_height, viewport_revision: descriptor.viewport_revision, document_generation: descriptor.document_generation });\n", 1))
    if skip_screenshot_restore:
        restore_site = """      const applied = await applyRequestedViewport(
        requested,
        context.cdp,
        context.binding,
        false,
        () => screenshotGeometryCurrent(context),
      );"""
        observe_only = """      const applied = await context.page.evaluate(() => ({
        width: window.innerWidth, height: window.innerHeight, dpr: window.devicePixelRatio,
      }));"""
        if instrumented.count(restore_site) != 1:
            raise RuntimeError("packaged helper screenshot restoration changed; refusing unmatched diagnostic instrumentation")
        instrumented = instrumented.replace(restore_site, observe_only, 1)
    if raster_generation:
        marker_trace = r"""
let rasterFramesSaved = 0;
let rasterCapturesAtFinal = 0;
function saveRasterImage(kind, sequence, jpeg) {
  if (rasterFramesSaved >= 16 || !jpeg?.length) return;
  const path = `${process.env.COCKPIT_BROWSER_TRACE_PATH}.${kind}-${sequence}.jpg`;
  writeFileSync(path, jpeg, { mode: 0o600 });
  rasterFramesSaved++;
  traceCaptureLane('raster_image', { kind, sequence, path });
}
"""
        mutation_site = "      if (!screenshotGeometryCurrent(context)) return;\n"
        screenshot_result_site = "    if (!viewportRestored || !screenshotContextCurrent(context) || typeof capture.data !== 'string') return 'stale';\n"
        publication_site = "    emit({ type: 'frame', descriptor });\n"
        if any(instrumented.count(site) != 1 for site in (mutation_site, screenshot_result_site, publication_site)):
            raise RuntimeError("packaged helper raster sites changed; refusing unmatched diagnostic instrumentation")
        instrumented = (instrumented.replace(fs_import, fs_import + marker_trace, 1)
                        .replace(mutation_site, mutation_site + "      if (context.baseline.scrollY >= 3899 && ++rasterCapturesAtFinal === 2) { const generation = await context.page.evaluate(() => window.__rasterAdvance?.() ?? null); traceCaptureLane('raster_mutation', { generation }); }\n", 1)
                        .replace(screenshot_result_site,
                                 "    if (context.baseline.scrollY >= 3899 && typeof capture.data === 'string') saveRasterImage('capture', rasterCapturesAtFinal, Buffer.from(capture.data, 'base64'));\n" + screenshot_result_site, 1)
                        .replace(publication_site, publication_site + "    if (descriptor.scroll_y >= 3899) saveRasterImage('frame', descriptor.frame_sequence, jpeg);\n", 1))
    if snapshot_throughput_probe:
        probe_source = r"""
async function probePhysicalSnapshotThroughput(expectedCdp, expectedBinding) {
  const captureToken = state.captureToken;
  let captures = 0;
  let scrolling = 0;
  let scrollFirst = 0;
  let scrollLast = 0;
  let reportedScroll = false;
  const sample = async () => {
    if (!pageBindingIsCurrent(page, expectedCdp, expectedBinding) || !screencastActive || !state.viewIds.size || state.captureToken !== captureToken) return;
    const before = geometrySnapshot();
    try {
      const response = await expectedCdp.send('Page.captureScreenshot', {
        format: 'jpeg', quality: 80, captureBeyondViewport: false,
        clip: { x: before.scrollX, y: before.scrollY, width: before.cssWidth, height: before.cssHeight, scale: 1 },
      });
      const now = performance.now();
      const dimensions = encodedJpegDimensions(Buffer.from(response.data, 'base64'));
      captures++;
      if (before.scrollY > 0 && before.scrollY < 3900 && screenshotDimensionsMatchDensity(dimensions, before)) {
        scrolling++;
        scrollFirst ||= now;
        scrollLast = now;
      }
      if (!reportedScroll && before.scrollY >= 3900 && scrolling) {
        reportedScroll = true;
        traceCaptureLane('snapshot_probe_scroll', { captures, scrolling, scroll_seconds: (scrollLast - scrollFirst) / 1000,
          image_width: dimensions.width, image_height: dimensions.height, dpr: before.dpr });
      }
      if (captures === 1 || (captures & (captures - 1)) === 0) {
        traceCaptureLane('snapshot_probe', { captures, scrolling, scroll_seconds: (scrollLast - scrollFirst) / 1000,
          before_scroll: before.scrollY, after_scroll: state.scrollY, image_width: dimensions.width,
          image_height: dimensions.height, dpr: before.dpr });
      }
    } catch (error) {
      traceCaptureLane('snapshot_probe_error', { message: String(error?.message ?? error) });
    }
    setTimeout(sample, 0);
  };
  void sample();
}
"""
        startup_site = "      if (pageBindingIsCurrent(page, expectedCdp, expectedBinding)) screencastActive = true;\n"
        if instrumented.count(startup_site) != 1:
            raise RuntimeError("packaged helper stream start changed; refusing unmatched throughput probe")
        instrumented = (instrumented.replace(fs_import, fs_import + probe_source, 1)
                        .replace(startup_site, startup_site + "      if (screencastActive) void probePhysicalSnapshotThroughput(expectedCdp, expectedBinding);\n", 1))
    return instrumented



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
    parser.add_argument("--sustain-seconds", type=int, default=0, help="animate at least 330 seconds; sample owned process PSS/CPU after 30-second warm-up")
    parser.add_argument("--idle-resource-seconds", type=int, default=3, help="observe static native WebKit process PSS for at least 3 seconds after sustained animation")
    parser.add_argument("--hide-resource-seconds", type=int, default=0, help="hide the browser for at least 30 seconds after sustained animation, then require the same page to repaint on show")
    parser.add_argument("--hidden-scroll-smoke", action="store_true", help="quick native hide/reopen check after the page scrolls while no view is visible")
    parser.add_argument("--native-only-open", action="store_true", help="skip gateway browser pre-open; open and navigate using only the actual native UI")
    parser.add_argument("--trace-cdp-overrides", action="store_true", help="log CDP device-metrics sends from only a run-owned helper copy")
    parser.add_argument("--trace-capture-lanes", action="store_true", help="also count screencast and screenshot capture in the run-owned helper copy")
    parser.add_argument("--trace-frame-publication", action="store_true", help="also count helper frame-descriptor publication from only the run-owned helper copy")
    parser.add_argument("--trace-density-convergence", action="store_true", help="sample bounded native bitmap/paint convergence after the first usable image without relaxing its DPR2 gate")
    parser.add_argument("--density-diagnostic-continue", action="store_true", help="collect post-scroll evidence after a first-frame DPR2 failure; still fail the run")
    parser.add_argument("--trace-no-restore-override", action="store_true", help="diagnostic helper copy omits only the post-screenshot identical CDP override; never an acceptance run")
    parser.add_argument("--trace-raster-generation", action="store_true", help="save bounded run-only JPEGs and mutate the raster after the second settled capture; never an acceptance run")
    parser.add_argument("--trace-physical-screencast-ceiling", action="store_true", help="request physical-sized WebKit screencast JPEGs only in the run-owned helper; never an acceptance run")
    parser.add_argument("--trace-snapshot-throughput", action="store_true", help="measure physical snapshot throughput under real scroll alongside native live stream; never an acceptance run")
    args = parser.parse_args()
    if args.sustain_seconds and args.sustain_seconds < 330:
        parser.error("--sustain-seconds must be at least 330 for a full 300-second post-warm-up sample")
    if args.idle_resource_seconds < 3 or (args.idle_resource_seconds != 3 and not args.sustain_seconds):
        parser.error("--idle-resource-seconds requires --sustain-seconds and a value of at least 3")
    if args.hide_resource_seconds and (not args.sustain_seconds or args.hide_resource_seconds < 30):
        parser.error("--hide-resource-seconds requires --sustain-seconds and at least 30 seconds")
    if args.hidden_scroll_smoke and args.sustain_seconds:
        parser.error("--hidden-scroll-smoke uses the six-second scroll fixture, not sustained sampling")
    if args.native_only_open and args.annotation:
        parser.error("--native-only-open cannot use gateway-owned draft recovery for --annotation")
    if args.trace_cdp_overrides and not args.native_only_open:
        parser.error("--trace-cdp-overrides requires --native-only-open")
    if args.trace_capture_lanes and not args.trace_cdp_overrides:
        parser.error("--trace-capture-lanes requires --trace-cdp-overrides and --native-only-open")
    if args.trace_frame_publication and not args.trace_capture_lanes:
        parser.error("--trace-frame-publication requires --trace-capture-lanes and --native-only-open")
    if args.trace_density_convergence and not args.trace_frame_publication:
        parser.error("--trace-density-convergence requires --trace-frame-publication")
    if args.density_diagnostic_continue and not args.trace_density_convergence:
        parser.error("--density-diagnostic-continue requires --trace-density-convergence")
    if args.trace_no_restore_override and not args.density_diagnostic_continue:
        parser.error("--trace-no-restore-override requires --density-diagnostic-continue")
    if args.trace_raster_generation and not args.density_diagnostic_continue:
        parser.error("--trace-raster-generation requires --density-diagnostic-continue")
    if args.trace_physical_screencast_ceiling and not args.density_diagnostic_continue:
        parser.error("--trace-physical-screencast-ceiling requires --density-diagnostic-continue")
    if args.trace_snapshot_throughput and not args.density_diagnostic_continue:
        parser.error("--trace-snapshot-throughput requires --density-diagnostic-continue")
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
    FixtureHandler.state["raster_marker"] = args.trace_raster_generation
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
    fixture_url = f"http://127.0.0.1:{fixture.server_port}/" + (f"?sustained={args.sustain_seconds}" if args.sustain_seconds else "?hiddenScroll=1" if args.hidden_scroll_smoke else "")
    env = {k: v for k, v in os.environ.items()
           if not k.startswith(("HERDR_", "COCKPIT_")) and k not in ("PI_TOOL_BRIDGE_TOKEN", "SSH_AUTH_SOCK")}
    env.update(HOME=str(root), XDG_CONFIG_HOME=str(root / "config"), XDG_STATE_HOME=str(root / "state"),
               XDG_CACHE_HOME=str(root / "cache"), XDG_DATA_HOME=str(root / "data"),
               HERDR_CONFIG_PATH=str(root / "config/herdr/config.toml"), HERDR_SOCKET_PATH=str(session_socket),
               COCKPIT_CONFIG=str(root / "cockpit.toml"), COCKPIT_HERDR_SESSION=session, COCKPIT_HERDR_SOCKET=str(session_socket),
               COCKPIT_HERDR_EXECUTABLE=bins["herdr"])
    helper_path = root / "browser-helper-trace.mjs"
    override_trace_path = root / "browser-cdp-overrides.jsonl"
    children, driver, frontend = [], None, None
    result = {"session": session, "space": space_label, "fixture_url": fixture_url, "root": str(root),
              "failure": None, "geometries": {}, "paints": {}, "cleanup": []}
    resource_samples = []
    sampler_stop = threading.Event()
    sampler = None

    def sample_resources(start):
        next_sample = start
        while not sampler_stop.wait(max(0, next_sample - time.monotonic())):
            resource_samples.append({"elapsed_seconds": time.monotonic() - start,
                                     **sample_owned_processes(children)})
            next_sample += 1

    def launch(label, argv, runenv):
        log = (root / f"{label}.log").open("w")
        child = subprocess.Popen(argv, cwd=REPO, env=runenv, stdin=subprocess.DEVNULL,
                                 stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        children.append((label, child))
        return child

    try:
        if args.trace_cdp_overrides:
            source = (REPO / "browser-runtime/browser-helper.mjs").read_text()
            instrumented = traced_helper_source(source, capture_lanes=args.trace_capture_lanes,
                                                frame_publication=args.trace_frame_publication,
                                                skip_screenshot_restore=args.trace_no_restore_override,
                                                raster_generation=args.trace_raster_generation,
                                                physical_screencast_ceiling=args.trace_physical_screencast_ceiling,
                                                snapshot_throughput_probe=args.trace_snapshot_throughput)
            helper_path.write_text(instrumented)
            syntax = subprocess.run(["node", "--check", str(helper_path)],
                                    capture_output=True, text=True, timeout=10)
            if syntax.returncode:
                raise RuntimeError(f"disposable helper trace syntax invalid: {syntax.stderr[-600:]}")
            env.update(COCKPIT_BROWSER_HELPER=str(helper_path), COCKPIT_BROWSER_TRACE_PATH=str(override_trace_path))
            result["helper_trace_module"] = {
                "path": str(helper_path), "source_sha256": hashlib.sha256(source.encode()).hexdigest(),
                "instrumented_sha256": hashlib.sha256(instrumented.encode()).hexdigest()}
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
        if not args.native_only_open:
            post_json(gateway_url + "/api/v1/browser/action", action)
        result["association_open_source"] = "native" if args.native_only_open else "gateway-then-native"

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
        launcher_keys = (
            "HOME", "XDG_CONFIG_HOME", "XDG_STATE_HOME", "XDG_CACHE_HOME", "XDG_DATA_HOME", "HERDR_CONFIG_PATH",
            "HERDR_SOCKET_PATH", "COCKPIT_CONFIG", "COCKPIT_HERDR_SESSION", "COCKPIT_HERDR_SOCKET", "COCKPIT_HERDR_EXECUTABLE",
            "WAYLAND_DISPLAY", "NIRI_SOCKET",
        ) + (("COCKPIT_BROWSER_HELPER", "COCKPIT_BROWSER_TRACE_PATH") if args.trace_cdp_overrides else ())
        launcher.write_text("#!/bin/sh\n" + "".join(f"export {key}={json.dumps(app_env[key])}\n" for key in launcher_keys)
                            + "unset HERDR_SESSION HERDR_NAME DISPLAY\nexport TAURI_WEBVIEW_AUTOMATION=true\nexec " + json.dumps(bins["native"]) + "\n")
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
        output_response = subprocess.run([bins["compositor"], "msg", "-j", "outputs"],
                                         env=dict(app_env, NIRI_SOCKET=niri_socket), check=True,
                                         capture_output=True, text=True, timeout=5)
        outputs = json.loads(output_response.stdout)
        logical_width = max((item.get("logical") or {}).get("width", 0) for item in outputs.values())
        if logical_width < 320:
            raise RuntimeError(f"private compositor has no usable full-width output: {outputs}")
        result["geometries"]["compositor_logical_width"] = logical_width
        last_outer, stable_outer_samples = None, 0
        def fixed_outer():
            nonlocal last_outer, stable_outer_samples
            raw = webdriver.execute("({w:innerWidth,h:innerHeight,d:devicePixelRatio})")
            observed = {"width": raw["w"], "height": raw["h"], "dpr": raw["d"]}
            result["geometries"]["outer_last_observed"] = observed
            # Niri reserves its own gaps around the full-width column; the
            # WebKit content box is smaller than the logical output itself.
            if observed["width"] >= logical_width * 0.85 and observed["height"] >= 240 and observed["dpr"] == 2:
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
        wait_until(fixed_outer, "stable full-column DPR2 app viewport matching the private compositor output", args.timeout,
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
        if args.native_only_open:
            result["native_only_trace_install"] = webdriver.execute("""(()=>{
              const trace={started:performance.now(),frames:[],tailFrames:[],frameCount:0,events:[],sockets:0};
              window.__nativeBrowserTrace=trace;
              const RealSocket=window.WebSocket;
              window.WebSocket=new Proxy(RealSocket,{construct(Target,args){
                const socket=new Target(...args);trace.sockets++;
                socket.addEventListener('message',event=>{
                  if(typeof event.data!=='string')return;
                  try{
                    const body=JSON.parse(event.data),time=performance.now();
                    if(body.kind==='frame'){
                      const d=body.descriptor||{},frame={time,target_id:d.target_id,frame_sequence:d.frame_sequence,
                        viewport_revision:d.viewport_revision,viewport_css_width:d.viewport_css_width,
                        viewport_css_height:d.viewport_css_height,image_width:d.image_width,image_height:d.image_height};
                      trace.frameCount++;
                      if(trace.frames.length<24)trace.frames.push(frame);
                      trace.tailFrames.push(frame);if(trace.tailFrames.length>24)trace.tailFrames.shift();
                    }else if(body.kind==='event'&&trace.events.length<24){
                      const e=body.event||{};
                      if(['attached','viewport_changed','document_changed'].includes(e.type))
                        trace.events.push({time,type:e.type,viewport:e.viewport||e.snapshot?.viewport||null});
                    }
                  }catch{}
                });return socket;
              }});
              return {socketWrapped:true};
            })()""")
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
        if args.native_only_open:
            wait_until(lambda: webdriver.execute("(()=>!!document.querySelector('.browser-navigation input[aria-label=\"Page URL\"]') && document.querySelector('.browser-toolbar-status')?.textContent==='Live browser view')()"),
                       "live native browser and Page URL field", args.timeout, [("WebKitWebDriver", driver_process)])
            webdriver.execute("(()=>{const input=document.querySelector('.browser-navigation input[aria-label=\"Page URL\"]');const form=input.closest('form');window.__nativeUrlSubmits=[];form.addEventListener('submit',e=>{const value=input.value;setTimeout(()=>window.__nativeUrlSubmits.push({value,defaultPrevented:e.defaultPrevented}),0)},true);input.focus();input.select();return true})()")
            typed_with = "webdriver-actions"
            try:
                keys = [key for character in fixture_url for key in
                        ({"type": "keyDown", "value": character}, {"type": "keyUp", "value": character})]
                webdriver.request("POST", webdriver.path("/actions"),
                                  {"actions": [{"type": "key", "id": "url-keyboard", "actions": keys}]})
            except RuntimeError as error:
                if "unsupported operation" not in str(error):
                    raise
                typed_with = "focused-input-insertText"
                inserted = webdriver.execute("document.execCommand('insertText',false," + json.dumps(fixture_url) + ")")
                if not inserted:
                    raise RuntimeError("WebKit native Page URL insertText was not accepted") from error
            time.sleep(0.15)
            input_value = webdriver.execute("document.querySelector('.browser-navigation input[aria-label=\"Page URL\"]').value")
            result["navigation_input"] = {"method": typed_with, "field_after_render": input_value}
            if input_value != fixture_url:
                raise RuntimeError(f"native Page URL React field did not retain fixture URL: {input_value!r}")
            webdriver.execute("(()=>{document.querySelector('.browser-navigation input[aria-label=\"Page URL\"]').closest('form').requestSubmit();return true})()")
            wait_until(lambda: next((report for report in reversed(FixtureHandler.state["reports"])
                if report.get("ready") == "complete" and report.get("heading") == "Native browser acceptance"), None),
                "fixture navigation after native-only open", args.timeout, [("WebKitWebDriver", driver_process)])
        def first_canvas():
            probe = webdriver.execute("(()=>{const c=document.querySelector('canvas.browser-frame'),r=c?.getBoundingClientRect(),s=document.querySelector('.browser-surface')?.getBoundingClientRect();return {painted:!!c && document.querySelector('.browser-toolbar-status')?.textContent==='Live browser view' && c.width!==300 && c.height!==150,status:document.querySelector('.browser-toolbar-status')?.textContent,canvas:{width:c?.width||0,height:c?.height||0,left:r?.left||0,top:r?.top||0,widthCss:r?.width||0,heightCss:r?.height||0},surfaceCss:{width:s?.width||0,height:s?.height||0},viewport:{width:innerWidth,height:innerHeight,dpr:devicePixelRatio},surface:!!s,alerts:[...document.querySelectorAll('[role=alert]')].map(x=>x.textContent?.trim()).filter(Boolean),commandOpen:!!document.querySelector('button.command-row'),bodyStart:(document.body?.innerText||'').slice(0,400),body:(document.body?.innerText||'').slice(-600)}})()")
            result["geometries"]["first_canvas_last_observed"] = probe
            return probe if (probe["painted"] and probe["canvas"]["widthCss"] > 0 and probe["canvas"]["heightCss"] > 0
                             and abs(probe["canvas"]["widthCss"] - probe["surfaceCss"]["width"]) <= 1
                             and abs(probe["canvas"]["heightCss"] - probe["surfaceCss"]["height"]) <= 1) else None
        geometry = wait_until(first_canvas, "first native browser canvas frame paint", args.timeout,
                              [("WebKitWebDriver", driver_process)])
        if args.trace_cdp_overrides:
            result["owned_helper_pids_at_first_canvas"] = owned_helper_processes(helper_path)
        if args.native_only_open:
            result["geometries"]["native_only_surface"] = webdriver.execute("(()=>{const info=e=>{if(!e)return null;const r=e.getBoundingClientRect(),s=getComputedStyle(e);return {rect:{width:r.width,height:r.height},client:{width:e.clientWidth,height:e.clientHeight},css:{width:s.width,height:s.height,overflow:s.overflow,objectFit:s.objectFit}}};return {pane:info(document.querySelector('.browser-pane')),surface:info(document.querySelector('.browser-surface')),canvas:info(document.querySelector('canvas.browser-frame'))}})()")
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
        if args.trace_density_convergence:
            samples = []
            start = time.monotonic()
            deadline = start + 5
            while True:
                sample = webdriver.execute("(()=>{const c=document.querySelector('canvas.browser-frame'),r=c?.getBoundingClientRect();return {bitmap_width:c?.width||0,bitmap_height:c?.height||0,css_width:r?.width||0,css_height:r?.height||0,paints:window.__nativePaintTimes?.length||0,status:document.querySelector('.browser-toolbar-status')?.textContent}})()")
                sample["elapsed_seconds"] = round(time.monotonic() - start, 3)
                if not samples or any(sample[key] != samples[-1][key] for key in ("bitmap_width", "bitmap_height", "css_width", "css_height", "paints", "status")):
                    samples.append(sample)
                if time.monotonic() >= deadline:
                    break
                time.sleep(.2)
            result["density_convergence"] = {"samples": samples, "fixture_dpr": metrics["dpr"],
                "observed_seconds": round(time.monotonic() - start, 3),
                "final": samples[-1], "full_density_seen": any(
                    abs(s["bitmap_width"] / s["css_width"] - metrics["dpr"]) <= .05
                    and abs(s["bitmap_height"] / s["css_height"] - metrics["dpr"]) <= .05
                    for s in samples if s["css_width"] and s["css_height"])}
        if geometry["viewport"] != result["geometries"].get("outer"):
            raise RuntimeError(f"native outer viewport changed before browser view attach: {geometry['viewport']}")
        if abs(metrics["dpr"] - 2) > .01 or abs(metrics["innerWidth"] - geometry["canvas"]["widthCss"]) > 2 or abs(metrics["innerHeight"] - geometry["canvas"]["heightCss"]) > 2:
            raise RuntimeError(f"fixture first-paint CSS geometry/DPR disagrees with native canvas: {metrics} vs {geometry['canvas']}")
        if metrics.get("heading") != "Native browser acceptance" or "without doubled CSS width" not in metrics.get("contentText", "") or metrics.get("headingWidth", 0) < metrics["innerWidth"] * .9:
            raise RuntimeError(f"first-paint CSS text/width report is incomplete or distorted: {metrics}")
        if abs(density["bitmap_width_per_css"] - 2) > .05 or abs(density["bitmap_height_per_css"] - 2) > .05:
            if not args.density_diagnostic_continue:
                raise RuntimeError(f"distorted first paint: canvas bitmap density is not DPR2: {density}")
            result["diagnostic_first_density_failure"] = density
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
        if args.sustain_seconds:
            sample_start = time.monotonic()
            sampler = threading.Thread(target=sample_resources, args=(sample_start,), daemon=True)
            sampler.start()
        webdriver.execute("(()=>{const c=document.querySelector('canvas.browser-frame'),s=document.querySelector('.browser-surface'),r=c.getBoundingClientRect(),x=r.left+100,y=r.top+30;for(const type of ['pointerdown','pointerup'])s.dispatchEvent(new PointerEvent(type,{bubbles:true,cancelable:true,pointerId:1,pointerType:'mouse',clientX:x,clientY:y,button:0,buttons:type==='pointerdown'?1:0}));return {x,y}})()")
        completed = wait_until(lambda: next((report for report in reversed(FixtureHandler.state["reports"])
            if report.get("status") == "scroll complete" and report.get("started")
            and report.get("scrollFinish") is not None and report.get("scrollY", 0) >= 3899), None),
            "fixture scroll completion and final position 3900", args.sustain_seconds + 10 if args.sustain_seconds else 10,
            [("WebKitWebDriver", driver_process)])
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
        if abs(scroll_duration - (args.sustain_seconds * 1000 if args.sustain_seconds else 6000)) > 500:
            raise RuntimeError(f"fixture scroll animation did not sustain its requested duration: {scroll_duration:.0f} ms")
        if fps <= args.fps_min:
            raise RuntimeError(f"actual native canvas drawImage presentation rate {fps:.2f} FPS is not greater than {args.fps_min}")
        final_marker = wait_until(
            lambda: (lambda rgb: rgb if rgb[1] >= 100 and rgb[0] < 80 and rgb[2] < 130 else None)(
                webdriver.execute("(()=>{const c=document.querySelector('canvas.browser-frame');return [...c.getContext('2d').getImageData(4,4,1,1).data].slice(0,3)})()")),
            "completed scroll marker painted on native canvas", 3, [("WebKitWebDriver", driver_process)])
        if args.hidden_scroll_smoke:
            page_before = webdriver.execute("document.querySelector('input[aria-label=\"Page URL\"]')?.value")
            if page_before != fixture_url:
                raise RuntimeError(f"native browser URL changed before hidden-scroll smoke: {page_before!r}")
            choose_browser_command(webdriver, "Hide browser view", [("WebKitWebDriver", driver_process)])
            wait_until(lambda: webdriver.execute("document.querySelector('.browser-region')?.style.display==='none' && document.querySelector('.browser-pane')?.classList.contains('browser-pane-hidden')"),
                       "hidden native browser view before fixture scroll", 5, [("WebKitWebDriver", driver_process)])
            result["hidden_scroll"] = {}
            hidden_at = time.monotonic()
            paints_before_hidden_scroll = webdriver.execute("window.__nativePaintTimes.length")
            post_json(fixture_url.split("?", 1)[0] + "arm-hidden-scroll", {})
            moved = wait_until(lambda: next((item for item in reversed(FixtureHandler.state["reports"])
                if item["received_at"] > hidden_at and item.get("status") == "hidden scroll complete"
                and abs(item.get("scrollY", 0) - 4100) <= 1), None),
                "fixture's authoritative scroll to 4100 while browser view hidden", 8, [("WebKitWebDriver", driver_process)])
            hidden_paints = webdriver.execute("window.__nativePaintTimes.length") - paints_before_hidden_scroll
            if hidden_paints:
                raise RuntimeError(f"native browser canvas painted {hidden_paints} frames while hidden")
            shown_at = time.monotonic()
            choose_browser_command(webdriver, "Show browser view", [("WebKitWebDriver", driver_process)])
            def reopened_scrolled():
                state = webdriver.execute("(()=>{const r=document.querySelector('.browser-region'),c=document.querySelector('canvas.browser-frame');return {visible:!!r&&getComputedStyle(r).display!=='none',status:document.querySelector('.browser-toolbar-status')?.textContent,url:document.querySelector('input[aria-label=\"Page URL\"]')?.value,paints:window.__nativePaintTimes.length,marker:c?[...c.getContext('2d').getImageData(4,4,1,1).data].slice(0,3):null}})()")
                result["hidden_scroll"]["last_reopen_state"] = state
                return state if state["visible"] and state["status"] == "Live browser view" and state["url"] == fixture_url and state["paints"] > paints_before_hidden_scroll and state["marker"] and all(abs(a-b)<=8 for a,b in zip(state["marker"], (22, 69, 173))) else None
            result["hidden_scroll"].update({"moved": moved, "hidden_paints": hidden_paints})
            result["hidden_scroll"]["reopened"] = wait_until(reopened_scrolled, "new measured scroll marker on the reopened native canvas",
                                                              args.timeout, [("WebKitWebDriver", driver_process)])
            result["hidden_scroll"]["reopened_page_metrics"] = wait_until(lambda: next((item for item in reversed(FixtureHandler.state["reports"])
                if item["received_at"] >= shown_at and item.get("status") == "hidden scroll complete"
                and abs(item.get("innerWidth", 0) - geometry["canvas"]["widthCss"]) <= 2
                and abs(item.get("innerHeight", 0) - geometry["canvas"]["heightCss"]) <= 2
                and abs(item.get("dpr", 0) - 2) <= .01), None),
                "reopened page restoring the accepted CSS viewport and DPR2", 5, [("WebKitWebDriver", driver_process)])
            reopened_image = root / "native-hidden-scroll-reopened.png"
            reopened_image.write_bytes(base64.b64decode(webdriver.request("GET", webdriver.path("/screenshot"))["value"]))
            result["screenshots"]["hidden_scroll_reopened"] = str(reopened_image)
            result["hidden_scroll"]["page_reports"] = [item for item in FixtureHandler.state["reports"] if item["received_at"] >= hidden_at][-8:]
            result["hidden_scroll"]["surface"] = webdriver.execute("(()=>{const s=document.querySelector('.browser-surface'),c=document.querySelector('canvas.browser-frame'),r=s?.getBoundingClientRect();return {width:r?.width,height:r?.height,canvas_width:c?.width,canvas_height:c?.height,host_scroll_y:window.scrollY,status:document.querySelector('.browser-toolbar-status')?.textContent}})()")
            surface = result["hidden_scroll"]["surface"]
            if not surface["width"] or not surface["height"] or abs(surface["canvas_width"] / surface["width"] - 2) > .05 or abs(surface["canvas_height"] / surface["height"] - 2) > .05:
                raise RuntimeError(f"reopened native canvas did not retain DPR2 image density: {surface}")
            wheel_at = time.monotonic()
            webdriver.execute("(()=>{const c=document.querySelector('canvas.browser-frame'),s=document.querySelector('.browser-surface'),r=c.getBoundingClientRect();s.dispatchEvent(new WheelEvent('wheel',{bubbles:true,cancelable:true,clientX:r.left+120,clientY:r.top+166,deltaX:0,deltaY:120,deltaMode:0}));return true})()")
            try:
                result["hidden_scroll"]["wheel_report"] = wait_until(lambda: next((item for item in reversed(FixtureHandler.state["reports"])
                    if item["received_at"] > wheel_at and 4100 < item.get("scrollY", 0) < 4500), None),
                    "reopened viewport accepting routed wheel at scroll 4100", 5, [("WebKitWebDriver", driver_process)])
            finally:
                result["hidden_scroll"]["page_reports_after_wheel"] = [item for item in FixtureHandler.state["reports"] if item["received_at"] >= hidden_at][-8:]
        if args.sustain_seconds:
            animation_end_elapsed = completed["received_at"] - sample_start
            time.sleep(2)
            idle_start_elapsed = time.monotonic() - sample_start
            first_idle_paint = webdriver.execute("window.__nativePaintTimes.length")
            time.sleep(3)
            last_idle_paint = webdriver.execute("window.__nativePaintTimes.length")
            idle_paints = last_idle_paint - first_idle_paint
            if idle_paints > 2:
                raise RuntimeError(f"static page kept painting after sustained scroll: {idle_paints}")
            if args.idle_resource_seconds > 3:
                time.sleep(args.idle_resource_seconds - 3)
                long_idle_paints = webdriver.execute("window.__nativePaintTimes.length") - first_idle_paint
                if long_idle_paints > 2:
                    raise RuntimeError(f"static page resumed painting during idle resource observation: {long_idle_paints}")
            hidden_start_elapsed = None
            if args.hide_resource_seconds:
                page_before = webdriver.execute("document.querySelector('input[aria-label=\"Page URL\"]')?.value")
                if page_before != fixture_url:
                    raise RuntimeError(f"native browser URL changed before hide: {page_before!r}")
                choose_browser_command(webdriver, "Hide browser view", [("WebKitWebDriver", driver_process)])
                wait_until(lambda: webdriver.execute("document.querySelector('.browser-region')?.style.display==='none' && document.querySelector('.browser-pane')?.classList.contains('browser-pane-hidden')"),
                           "hidden browser view and retired stream", 5, [("WebKitWebDriver", driver_process)])
                hidden_image = root / "native-hidden.png"
                hidden_image.write_bytes(base64.b64decode(webdriver.request("GET", webdriver.path("/screenshot"))["value"]))
                result["screenshots"]["browser_hidden"] = str(hidden_image)
                hidden_start_elapsed = time.monotonic() - sample_start
                hidden_start_paints = webdriver.execute("window.__nativePaintTimes.length")
                time.sleep(args.hide_resource_seconds)
                hidden_end_elapsed = time.monotonic() - sample_start
                hidden_end_paints = webdriver.execute("window.__nativePaintTimes.length")
                if hidden_end_paints != hidden_start_paints:
                    raise RuntimeError(f"hidden native browser painted {hidden_end_paints-hidden_start_paints} frames")
                result["lifecycle"] = {"page_before_hide": page_before,
                                       "hidden_paints": hidden_end_paints - hidden_start_paints,
                                       "hidden_seconds": args.hide_resource_seconds}
            sampler_stop.set()
            sampler.join(timeout=3)
            active_post_warmup = [sample for sample in resource_samples
                                  if 30 <= sample["elapsed_seconds"] <= animation_end_elapsed]
            idle_samples = [sample for sample in resource_samples
                            if idle_start_elapsed <= sample["elapsed_seconds"]
                            and (hidden_start_elapsed is None or sample["elapsed_seconds"] < hidden_start_elapsed)]
            result["resources"] = {"raw_samples": str(root / "sustained-samples.json"),
                                   "sample_period_seconds": 1, "warmup_seconds": 30,
                                   "post_warmup_count": len(active_post_warmup),
                                   "idle_sample_count": len(idle_samples),
                                   "animation_end_elapsed_seconds": animation_end_elapsed,
                                   "idle_start_elapsed_seconds": idle_start_elapsed,
                                   "idle_end_elapsed_seconds": hidden_start_elapsed if hidden_start_elapsed is not None else time.monotonic() - sample_start,
                                   "idle_observation_seconds": args.idle_resource_seconds,
                                   "idle_paints_over_three_seconds": idle_paints,
                                   "cpu_tick_hz": os.sysconf("SC_CLK_TCK"),
                                   "sampled_roots": [label for label, _ in [("acceptance-runner", os.getpid())] +
                                                     [(label, child.pid) for label, child in children]]}
            if hidden_start_elapsed is not None:
                hidden_samples = [sample for sample in resource_samples
                                  if hidden_start_elapsed <= sample["elapsed_seconds"] <= hidden_end_elapsed]
                result["resources"].update({"hidden_start_elapsed_seconds": hidden_start_elapsed,
                                            "hidden_end_elapsed_seconds": hidden_end_elapsed,
                                            "hidden_sample_count": len(hidden_samples)})
                if len(hidden_samples) < args.hide_resource_seconds - 1:
                    raise RuntimeError(f"hidden resource sampling was incomplete: {result['resources']}")
            if args.idle_resource_seconds > 3:
                result["resources"]["idle_paints_over_observation"] = long_idle_paints
            if len(active_post_warmup) < 300 or any(sample["process_count"] < 5 or sample["pss_kib"] <= 0
                                                     for sample in active_post_warmup):
                raise RuntimeError(f"sustained process tree sampling was incomplete: {result['resources']}")
            if hidden_start_elapsed is not None:
                choose_browser_command(webdriver, "Show browser view", [("WebKitWebDriver", driver_process)])
                def reopened_browser():
                    state = webdriver.execute("(()=>{const r=document.querySelector('.browser-region'),c=document.querySelector('canvas.browser-frame'),b=c?.getBoundingClientRect();return {visible:!!r&&getComputedStyle(r).display!=='none',status:document.querySelector('.browser-toolbar-status')?.textContent,url:document.querySelector('input[aria-label=\"Page URL\"]')?.value,paints:window.__nativePaintTimes.length,marker:c?[...c.getContext('2d').getImageData(4,4,1,1).data].slice(0,3):null,canvas_width:c?.width||0,canvas_height:c?.height||0,css_width:b?.width||0,css_height:b?.height||0}})()")
                    result["lifecycle"]["last_reopen_state"] = state
                    return state if state["visible"] and state["status"] == "Live browser view" and state["url"] == fixture_url and state["paints"] > hidden_end_paints and state["marker"] and all(abs(a-b)<=8 for a,b in zip(state["marker"], final_marker)) and state["css_width"] and state["css_height"] and abs(state["canvas_width"]/state["css_width"]-2)<=.05 and abs(state["canvas_height"]/state["css_height"]-2)<=.05 else None
                result["lifecycle"]["reopened"] = wait_until(reopened_browser, "same native browser page and painted marker after show",
                                                                args.timeout, [("WebKitWebDriver", driver_process)])
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
        if args.trace_density_convergence:
            post_scroll = []
            start = time.monotonic()
            while True:
                image = webdriver.execute("(()=>{const c=document.querySelector('canvas.browser-frame'),r=c?.getBoundingClientRect();return {bitmap_width:c?.width||0,bitmap_height:c?.height||0,css_width:r?.width||0,css_height:r?.height||0,paints:window.__nativePaintTimes?.length||0,status:document.querySelector('.browser-toolbar-status')?.textContent}})()")
                image["elapsed_seconds"] = round(time.monotonic() - start, 3)
                if not post_scroll or any(image[key] != post_scroll[-1][key] for key in ("bitmap_width", "bitmap_height", "css_width", "css_height", "paints", "status")):
                    post_scroll.append(image)
                if time.monotonic() - start >= 5:
                    break
                time.sleep(.2)
            result["density_convergence"]["post_scroll"] = post_scroll
            result["density_convergence"]["post_scroll_observed_seconds"] = round(time.monotonic() - start, 3)
            settled = post_scroll[-1]
            if not settled["css_width"] or not settled["css_height"] or (
                abs(settled["bitmap_width"] / settled["css_width"] - metrics["dpr"]) > .05
                or abs(settled["bitmap_height"] / settled["css_height"] - metrics["dpr"]) > .05
            ):
                if not args.density_diagnostic_continue:
                    raise RuntimeError(f"native canvas lost DPR2 density after settled scroll: {settled}")
                result["diagnostic_settled_density_failure"] = settled
        # Preserve visual evidence from the actual native WebKit window.
        screenshot = webdriver.request("GET", webdriver.path("/screenshot"))["value"]
        (root / "native-window.png").write_bytes(base64.b64decode(screenshot))
        result["screenshot"] = str(root / "native-window.png")
        if result.get("diagnostic_first_density_failure") or result.get("diagnostic_settled_density_failure") or args.trace_no_restore_override or args.trace_raster_generation or args.trace_physical_screencast_ceiling or args.trace_snapshot_throughput:
            raise RuntimeError("density diagnostic retained a DPR2 failure or used a run-only helper modification")
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
        if args.native_only_open and driver:
            try:
                result["navigation_diagnostics"] = driver.execute("(()=>({field:document.querySelector('.browser-navigation input[aria-label=\"Page URL\"]')?.value,status:document.querySelector('.browser-toolbar-status')?.textContent,submits:window.__nativeUrlSubmits||[],tabs:[...document.querySelectorAll('.browser-tabs [role=\"tab\"]')].map(b=>({title:b.textContent,url:b.title})),alerts:[...document.querySelectorAll('[role=\"alert\"]')].map(e=>e.textContent?.trim()),javascriptErrors:window.__nativeAcceptanceErrors||[]}))()")
                result["navigation_diagnostics"]["fixture_report_count"] = len(FixtureHandler.state["reports"])
                result["navigation_diagnostics"]["browser_trace"] = driver.execute("window.__nativeBrowserTrace||null")
            except Exception as diagnostic_error:
                result["navigation_diagnostics_error"] = str(diagnostic_error)[:800]
        if args.native_only_open:
            census = sample_owned_processes(children)
            result["owner_processes"] = {
                label: [{"pid": process["pid"], "comm": process["comm"], "start_ticks": process["start_ticks"]}
                        for process in record["processes"]]
                for label, record in census["roots"].items() if label in ("gateway", "webdriver")}
        if args.trace_cdp_overrides:
            result["owned_helper_pids_at_failure"] = owned_helper_processes(helper_path)
    finally:
        if args.native_only_open:
            reports = FixtureHandler.state["reports"]
            result["fixture_reports"] = {"count": len(reports),
                                         "first": reports[:2], "last": reports[-2:]}
        sampler_stop.set()
        if sampler:
            sampler.join(timeout=3)
            (root / "sustained-samples.json").write_text(json.dumps(resource_samples))
            result.setdefault("resources", {})["raw_samples"] = str(root / "sustained-samples.json")
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
        if args.trace_cdp_overrides:
            lines = override_trace_path.read_text().splitlines() if override_trace_path.is_file() else []
            events = [json.loads(line) for line in lines]
            overrides = [event for event in events if event.get("phase") != "capture_lane"]
            result["cdp_override_trace"] = {"path": str(override_trace_path), "count": len(overrides),
                                             "first": overrides[:8], "last": overrides[-8:]}
            if args.trace_capture_lanes:
                lanes = [event for event in events if event.get("phase") == "capture_lane"]
                last_counts = {}
                for event in lanes:
                    last_counts[(event["pid"], event["start_ticks"], event["lane"])] = event["count"]
                result["capture_lane_trace"] = {
                    "path": str(override_trace_path), "count": len(lanes),
                    "last_counts": [{"pid": pid, "start_ticks": ticks, "lane": lane, "sampled_count": count}
                                    for (pid, ticks, lane), count in sorted(last_counts.items())],
                    "first": lanes[:8], "last": lanes[-8:],
                }
        result["logs"] = {p.stem: str(p) for p in root.glob("*.log")}
        if args.trace_cdp_overrides:
            (root / "native-cdp-result.json").write_text(json.dumps(result, indent=2) + "\n")
        print(json.dumps(result, indent=2))
    return 0 if result.get("ok") else 1


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (RuntimeError, OSError, subprocess.SubprocessError, urllib.error.URLError) as error:
        print(json.dumps({"ok": False, "failure": str(error)}), file=sys.stderr)
        raise SystemExit(1)
