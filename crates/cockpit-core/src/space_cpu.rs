use std::{collections::{HashMap, HashSet, VecDeque}, time::{Duration, Instant}};
use cockpit_protocol::v1::{SpaceCpuPane, SpaceCpuProcess, SpaceCpuStatus, SpaceCpuStatusRequest, SpaceCpuStatusResponse};
use tokio::sync::Mutex;
use crate::{HerdrAdapter, InspectionError};

#[derive(Default)]
pub struct SpaceCpuSampler { sessions: Mutex<HashMap<String, Session>> }
#[derive(Default)]
struct Session {
    server: String,
    panes: Vec<SpaceCpuPane>,
    roots: HashMap<String, (u32, u64)>,
    previous: HashMap<u32, Process>,
    last: Option<Instant>,
    windows: HashMap<String, VecDeque<(Instant, f64, f64)>>,
    active: HashMap<String, bool>,
    retry_missing: Option<Instant>,
}
#[derive(Clone, Debug)]
struct Process { pid: u32, parent: u32, start: u64, cpu: f64, name: String }

impl SpaceCpuSampler {
    pub async fn sample(&self, adapter: &dyn HerdrAdapter, session_id: &str, request: &SpaceCpuStatusRequest) -> Result<SpaceCpuStatusResponse, InspectionError> {
        let empty = || SpaceCpuStatusResponse { session_id: session_id.into(), server_instance: request.server_instance.clone(), available: false, spaces: vec![] };
        if !cfg!(any(target_os = "linux", target_os = "macos")) { return Ok(empty()); }
        let mut sessions = self.sessions.lock().await;
        let state = sessions.entry(session_id.to_owned()).or_default();
        let mut panes = request.panes.clone();
        panes.sort_by(|a,b| (&a.pane_id, &a.space_id, &a.terminal_id).cmp(&(&b.pane_id, &b.space_id, &b.terminal_id)));
        panes.dedup();
        let changed = state.server != request.server_instance || state.panes != panes;
        if changed {
            let same_server = state.server == request.server_instance;
            let old_panes = if same_server { std::mem::take(&mut state.panes) } else { Vec::new() };
            if !same_server { *state = Session::default(); }
            let mut roots = HashMap::new();
            for pane in &panes {
                if old_panes.contains(pane) {
                    if let Some(root) = state.roots.get(&pane.pane_id) { roots.insert(pane.pane_id.clone(), *root); }
                } else {
                    match adapter.pane_shell_pid(session_id, &request.server_instance, &pane.pane_id).await {
                        Ok(Some(pid)) => { roots.insert(pane.pane_id.clone(), (pid, 0)); }
                        Ok(None) => {},
                        Err(_) => { state.server.clear(); return Ok(empty()); }
                    }
                }
            }
            let affected: HashSet<_> = old_panes.iter().filter(|p| !panes.contains(p))
                .chain(panes.iter().filter(|p| !old_panes.contains(p))).map(|p| p.space_id.clone()).collect();
            for id in affected { state.windows.remove(&id); state.active.remove(&id); }
            state.server = request.server_instance.clone();
            state.panes = panes.clone();
            state.roots = roots;
            state.retry_missing = Some(Instant::now());
        }
        if state.retry_missing.is_some_and(|last| last.elapsed() >= Duration::from_secs(30)) {
            state.retry_missing = Some(Instant::now());
            for pane in &panes {
                if !state.roots.contains_key(&pane.pane_id) {
                    if let Ok(Some(pid)) = adapter.pane_shell_pid(session_id, &request.server_instance, &pane.pane_id).await {
                        state.roots.insert(pane.pane_id.clone(), (pid, 0));
                        state.windows.remove(&pane.space_id);
                        state.active.remove(&pane.space_id);
                    }
                }
            }
        }
        let roots = state.roots.clone();
        let Ok(Some(processes)) = tokio::task::spawn_blocking(move || scan(&roots)).await else { return Ok(empty()); };
        for (pid, start) in state.roots.values_mut() {
            if *start == 0 { if let Some(p) = processes.get(pid) { *start = p.start; } }
        }
        let now = Instant::now();
        if state.last.is_some_and(|last| now.duration_since(last) > Duration::from_secs(30)) {
            state.previous.clear();
            state.windows.clear();
            state.active.clear();
            state.last = None;
        }
        let elapsed = state.last.map(|last| now.duration_since(last).as_secs_f64()).unwrap_or(0.0);
        let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
        for p in processes.values() { children.entry(p.parent).or_default().push(p.pid); }
        let mut by_space: HashMap<String, HashSet<u32>> = HashMap::new();
        for pane in &panes {
            let Some(&(pid, start)) = state.roots.get(&pane.pane_id) else { continue };
            if start == 0 || processes.get(&pid).is_none_or(|p| p.start != start) { continue; }
            let set = by_space.entry(pane.space_id.clone()).or_default();
            let mut stack = vec![pid];
            while let Some(pid) = stack.pop() {
                if set.insert(pid) { if let Some(kids) = children.get(&pid) { stack.extend(kids); } }
            }
        }
        let space_ids: HashSet<_> = panes.iter().map(|p| p.space_id.clone()).collect();
        let mut spaces = Vec::with_capacity(space_ids.len());
        for space_id in space_ids {
            let mut cpu = 0.0;
            let mut top: Option<SpaceCpuProcess> = None;
            if elapsed > 0.0 {
                for pid in by_space.get(&space_id).into_iter().flatten() {
                    let Some(p) = processes.get(pid) else { continue };
                    let Some(old) = state.previous.get(pid).filter(|old| old.start == p.start && p.cpu >= old.cpu) else { continue };
                    let percent = (p.cpu - old.cpu) / elapsed * 100.0;
                    cpu += percent;
                    if top.as_ref().is_none_or(|t| percent > t.percent) { top = Some(SpaceCpuProcess { pid: *pid, name: p.name.clone(), percent }); }
                }
                let active = state.active.entry(space_id.clone()).or_default();
                cpu = update_window(state.windows.entry(space_id.clone()).or_default(), active, now, cpu, elapsed);
            }
            spaces.push(SpaceCpuStatus { active: state.active.get(&space_id).copied().unwrap_or(false), space_id, percent: cpu, top_process: top });
        }
        spaces.sort_by(|a,b| a.space_id.cmp(&b.space_id));
        state.previous = processes;
        state.last = Some(now);
        Ok(SpaceCpuStatusResponse { session_id: session_id.into(), server_instance: request.server_instance.clone(), available: true, spaces })
    }
}

fn update_window(window: &mut VecDeque<(Instant, f64, f64)>, active: &mut bool, now: Instant, percent: f64, elapsed: f64) -> f64 {
    window.push_back((now, percent * elapsed, elapsed));
    while window.front().is_some_and(|(at, _, _)| now.duration_since(*at) >= Duration::from_secs(30)) { window.pop_front(); }
    let (work, duration) = window.iter().fold((0.0, 0.0), |(a,b), (_,c,d)| (a+c,b+d));
    let average = if duration > 0.0 { work / duration } else { 0.0 };
    if !*active && duration >= 30.0 && average >= 80.0 { *active = true; }
    else if *active && average < 50.0 { *active = false; }
    average
}

#[cfg(target_os = "linux")]
fn parse_stat(text: &str, ticks: f64) -> Option<Process> {
    let open = text.find('(')?;
    let close = text.rfind(')')?;
    let pid = text[..open].trim().parse().ok()?;
    let fields: Vec<_> = text.get(close + 2..)?.split_whitespace().collect();
    Some(Process { pid, name: text[open+1..close].to_owned(), parent: fields.get(1)?.parse().ok()?,
        cpu: (fields.get(11)?.parse::<u64>().ok()? + fields.get(12)?.parse::<u64>().ok()?) as f64 / ticks,
        start: fields.get(19)?.parse().ok()? })
}
#[cfg(target_os = "linux")]
fn scan(_roots: &HashMap<String, (u32, u64)>) -> Option<HashMap<u32, Process>> {
    let ticks = nix::unistd::sysconf(nix::unistd::SysconfVar::CLK_TCK).ok()??;
    if ticks <= 0 { return None; }
    let mut result = HashMap::new();
    for entry in std::fs::read_dir("/proc").ok()?.flatten() {
        if entry.file_name().to_str().and_then(|s| s.parse::<u32>().ok()).is_none() { continue; }
        if let Ok(text) = std::fs::read_to_string(entry.path().join("stat")) {
            if let Some(process) = parse_stat(&text, ticks as f64) { result.insert(process.pid, process); }
        }
    }
    Some(result)
}
#[cfg(target_os = "macos")]
fn scan(roots: &HashMap<String, (u32, u64)>) -> Option<HashMap<u32, Process>> {
    use libproc::{processes::{pids_by_type, ProcFilter}, proc_pid::{name, pidinfo}, task_info::TaskAllInfo};
    // Task CPU times are Mach absolute-time units: nanoseconds on Intel, 125/3 ns ticks on Apple Silicon.
    const MACH_TICK_NS: f64 = if cfg!(target_arch = "aarch64") { 125.0 / 3.0 } else { 1.0 };
    let mut processes = HashMap::new();
    let mut stack: Vec<u32> = roots.values().map(|(pid, _)| *pid).collect();
    let mut visited = HashSet::new();
    while let Some(pid) = stack.pop() {
        if !visited.insert(pid) { continue; }
        let Ok(raw_pid) = i32::try_from(pid) else { continue };
        let Ok(info) = pidinfo::<TaskAllInfo>(raw_pid, 0) else { continue };
        let start = info.pbsd.pbi_start_tvsec.checked_mul(1_000_000)?.checked_add(info.pbsd.pbi_start_tvusec)?;
        let process = Process { pid, parent: info.pbsd.pbi_ppid, start,
            cpu: (info.ptinfo.pti_total_user as f64 + info.ptinfo.pti_total_system as f64) * MACH_TICK_NS / 1_000_000_000.0,
            name: name(raw_pid).unwrap_or_default() };
        processes.insert(pid, process);
        let Ok(children) = pids_by_type(ProcFilter::ByParentProcess { ppid: pid }) else { return None };
        stack.extend(children.into_iter().filter(|child| *child != 0));
    }
    Some(processes)
}
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn scan(_roots: &HashMap<String, (u32, u64)>) -> Option<HashMap<u32, Process>> { None }

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    #[test]
    fn parses_stat_with_parentheses_and_process_identity() {
        let p = parse_stat("42 (a ) command) S 7 0 0 0 0 0 0 0 0 0 100 50 0 0 0 0 0 0 1234", 100.0).unwrap();
        assert_eq!((p.pid, p.parent, p.start, p.name.as_str()), (42, 7, 1234, "a ) command"));
        assert_eq!(p.cpu, 1.5);
    }
    #[test]
    fn requires_sustained_load_and_uses_hysteresis() {
        let start = Instant::now();
        let mut window = VecDeque::new();
        let mut active = false;
        for n in 1..=6 {
            update_window(&mut window, &mut active, start + Duration::from_secs(n * 5), 90.0, 5.0);
            assert_eq!(active, n == 6);
        }
        for n in 7..=9 {
            update_window(&mut window, &mut active, start + Duration::from_secs(n * 5), 40.0, 5.0);
            assert!(active);
        }
        for n in 10..=12 {
            update_window(&mut window, &mut active, start + Duration::from_secs(n * 5), 0.0, 5.0);
        }
        assert!(!active);
    }
}
