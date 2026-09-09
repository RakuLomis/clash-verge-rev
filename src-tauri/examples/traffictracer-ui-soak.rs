//! Standalone native event-bridge test. Never calls app_lib::run or production setup.
mod soak_support;
use app_lib::traffic_tracer_test_support::{ManagedChild, WORKER_API_VERSION, WorkerProcess};
use serde_json::json;
use soak_support::{Health, write_report};
use std::{
    fs,
    io::Write,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tauri_plugin_shell::process::CommandEvent;

#[derive(Clone, Default)]
struct Progress {
    last: Option<Instant>,
    received: u64,
    latest: u64,
    remounts: u64,
    errors: u64,
    rendered: u64,
    page_metrics: Option<serde_json::Value>,
}
#[derive(Clone, Default)]
struct Probe {
    last: Option<Instant>,
    data: serde_json::Value,
}
#[tauri::command]
fn soak_probe(state: tauri::State<'_, Arc<Mutex<Probe>>>, data: serde_json::Value) {
    *state.lock().unwrap() = Probe {
        last: Some(Instant::now()),
        data,
    };
}
#[tauri::command]
fn soak_heartbeat(
    state: tauri::State<'_, Arc<Mutex<Progress>>>,
    received: u64,
    latest: u64,
    remounts: u64,
    errors: u64,
    rendered: u64,
    page_metrics: Option<serde_json::Value>,
) {
    *state.lock().unwrap() = Progress {
        last: Some(Instant::now()),
        received,
        latest,
        remounts,
        errors,
        rendered,
        page_metrics,
    };
}
#[tauri::command]
async fn tt_ui_heartbeat(active: bool, app_handle: tauri::AppHandle) -> Result<app_lib::traffic_tracer_test_support::DesktopSnapshot, String> {
    // Real read-only desktop monitor; no production capture/profile supervisor.
    app_lib::traffic_tracer_test_support::tt_ui_heartbeat(active, app_handle).await.map_err(|error| error.to_string())
}
struct FakeChild;
#[tauri::command]
fn tt_notification_ack(sequence: u64) {
    app_lib::traffic_tracer_test_support::tt_notification_ack(sequence);
}
#[tauri::command]
fn tt_progress_ack(sequence: u64) {
    app_lib::traffic_tracer_test_support::tt_progress_ack(sequence);
}
#[tauri::command]
fn tt_desktop_recovery_ack(generation: u64) {
    app_lib::traffic_tracer_test_support::tt_desktop_recovery_ack(generation);
}
#[tauri::command]
fn tt_desktop_recovery_snapshot(report: app_lib::traffic_tracer_test_support::recovery_delivery::SnapshotReport) {
    app_lib::traffic_tracer_test_support::tt_desktop_recovery_snapshot(report);
}
// Inspect only this isolated process family; no production process data or arguments.
fn process_family_metrics() -> serde_json::Value {
    let mut pending = vec![std::process::id()];
    let mut seen = std::collections::BTreeSet::new();
    let mut processes = Vec::new();
    let mut total_rss_kib = 0u64;
    while let Some(pid) = pending.pop() {
        if !seen.insert(pid) || seen.len() > 128 {
            continue;
        }
        if let Ok(tasks) = fs::read_dir(format!("/proc/{pid}/task")) {
            for task in tasks.flatten() {
                if let Ok(children) = fs::read_to_string(task.path().join("children")) {
                    pending.extend(children.split_whitespace().filter_map(|id| id.parse::<u32>().ok()));
                }
            }
        }
        if let Ok(status) = fs::read_to_string(format!("/proc/{pid}/status")) {
            let field = |name: &str| {
                status
                    .lines()
                    .find_map(|line| line.strip_prefix(name))
                    .and_then(|value| value.split_whitespace().next()?.parse::<u64>().ok())
            };
            let rss = field("VmRSS:");
            let stat = fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
            let stat_fields: Vec<_> = stat
                .rsplit_once(')')
                .map(|(_, fields)| fields.split_whitespace().collect())
                .unwrap_or_default();
            let stat_number = |index: usize| stat_fields.get(index).and_then(|value| value.parse::<u64>().ok());
            total_rss_kib += rss.unwrap_or(0);
            processes.push(json!({"pid":pid,"rss_kib":rss,"threads":field("Threads:"),
                "cpu_ticks":stat_number(11).zip(stat_number(12)).map(|(user, system)|user + system),
                "major_faults":stat_number(9),
                "fd_count":fs::read_dir(format!("/proc/{pid}/fd")).map(|entries| entries.count()).ok()}));
        }
    }
    json!({"rss_sum_kib":total_rss_kib,"processes":processes})
}
impl ManagedChild for FakeChild {
    fn pid(&self) -> u32 {
        0
    } // Never an OS child; kill is a no-op.
    fn write(&mut self, _: &[u8]) -> anyhow::Result<()> {
        Ok(())
    }
    fn kill(self: Box<Self>) -> anyhow::Result<()> {
        Ok(())
    }
}
fn main() {
    let seconds: u64 = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "60".into())
        .parse()
        .expect("duration in seconds");
    assert!((15..=14400).contains(&seconds), "duration must be 15..14400 seconds");
    // Explicit diagnostic mode: continue across a lock/unlock cycle, but never
    // qualify this observation as a passing stability gate.
    let diagnostic_only = match std::env::var("TT_SOAK_OBSERVE_UNTIL_DEADLINE").as_deref() {
        Ok("1") => true,
        Ok("0") | Err(_) => false,
        _ => panic!("TT_SOAK_OBSERVE_UNTIL_DEADLINE must be 0 or 1"),
    };
    // Fault injection for sampling drift; never used by the production app.
    let sample_delay_ms: u64 = std::env::var("TT_SOAK_SAMPLE_DELAY_MS")
        .unwrap_or_else(|_| "0".into())
        .parse()
        .expect("sample delay in milliseconds");
    assert!(sample_delay_ms <= 1000);
    let fault = std::env::var("TT_SOAK_FAULT").unwrap_or_default();
    assert!(["", "event_stall", "early_close", "recovery_signal", "mixed_notifications"].contains(&fault.as_str()));
    let root = std::env::temp_dir().join(format!(
        "traffictracer-native-soak-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_micros()
    ));
    fs::create_dir(&root).expect("create new isolated report directory");
    write_report(
        &root,
        &json!({"schema_version":2,"status":"running","passed":false,"diagnostic_only":diagnostic_only,
        "scope":"native_bridge_react_progress","full_ui_soak":false}),
    )
    .expect("initial report");
    let final_root = root.clone();
    println!("SOAK_REPORT_DIR={}", root.display());
    let mut context = tauri::generate_context!("soak/tauri.conf.json");
    // Platform config overlays must never select production WebView storage.
    context.config_mut().identifier = "local.traffictracer.isolated-soak".into();
    assert_eq!(context.config().app.windows.len(), 1);
    assert_eq!(context.config().app.windows[0].label, "soak");
    context.config_mut().app.windows[0].data_directory = Some(root.join("webview"));
    let progress = Arc::new(Mutex::new(Progress::default()));
    let probe = Arc::new(Mutex::new(Probe::default()));
    let passed = Arc::new(AtomicBool::new(false));
    let report_passed = Arc::clone(&passed);
    let application = tauri::Builder::default()
        .manage(progress.clone())
        .manage(probe.clone())
        .invoke_handler(tauri::generate_handler![soak_heartbeat, soak_probe, tt_ui_heartbeat, tt_progress_ack, tt_notification_ack, tt_desktop_recovery_ack, tt_desktop_recovery_snapshot])
        .setup(move |app| {
            let process = Arc::new(WorkerProcess::new());
            process.configure_notification_journal(&root)?;
            let (sender, receiver) = tokio::sync::mpsc::channel(32);
            process.attach(receiver, Box::new(FakeChild))?;
            let bridge = process.bridge_to_tauri(app.handle().clone());
            let started = Instant::now();
            // Sampling overhead accumulates over hours. Keep producing until the
            // observer finishes, not until a different wall-clock deadline.
            let generation_finished = Arc::new(AtomicBool::new(false));
            let stop_generation = Arc::clone(&generation_finished);
            let stall_events = fault == "event_stall";
            let mixed_notifications = fault == "mixed_notifications";
            tauri::async_runtime::spawn(async move {
                let _process = process;
                let _bridge = bridge;
                let mut sequence = 0u64;
                while !generation_finished.load(Ordering::Acquire) {
                    if stall_events && started.elapsed() > Duration::from_secs(2) {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        continue;
                    }
                    sequence += 1;
                    let line = json!({"api_version":WORKER_API_VERSION,"type":"notification","method":"job.progress",
                        "params":{"job_id":"isolated-soak","state":"analyzing","stage":"analyzing","timestamp":chrono::Utc::now().to_rfc3339(),"message":format!("Synthetic analysis progress {sequence}"),"progress":0.5,"sequence":sequence}}).to_string();
                    if sender.send(CommandEvent::Stdout(line.into_bytes())).await.is_err() { break; }
                    if mixed_notifications && sequence % 2 == 0 {
                        let method = if sequence % 10 == 0 { "job.completed" } else { "worker.log" };
                        let line = json!({"api_version":WORKER_API_VERSION,"type":"notification","method":method,
                            "params":{"job_id":format!("synthetic-{}", sequence),"state":"completed","message":"Journaled synthetic notification"}}).to_string();
                        if sender.send(CommandEvent::Stdout(line.into_bytes())).await.is_err() { break; }
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            });
            let handle = app.handle().clone();
            if fault == "recovery_signal" || fault == "mixed_notifications" {
                // Isolated signal injection, not an OS lock. Heartbeat desktop
                // generation stays zero, proving notification-driven recovery.
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(Duration::from_secs(20)).await;
                    app_lib::traffic_tracer_test_support::recovery_delivery::global().lock().observe(1);
                });
            }
            if fault == "early_close" {
                let close_handle = handle.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_secs(2));
                    close_handle.exit(0);
                });
            }
            std::thread::spawn(move || {
                let mut health = Health::default();
                let mut max_gap = 0u64;
                let deadline = started + Duration::from_secs(seconds);
                let mut tick = 0u64;
                let main_pulse = Arc::new(AtomicU64::new(0));
                let pulse_pending = Arc::new(AtomicBool::new(false));
                let mut diagnostics;
                loop {
                    let wake = (Instant::now() + Duration::from_millis(1000 + sample_delay_ms)).min(deadline);
                    std::thread::sleep(wake.saturating_duration_since(Instant::now()));
                    tick += 1;
                    let now = Instant::now();
                    let elapsed_ms = now.duration_since(started).as_millis() as u64;
                    // At most one native-main-loop probe may be queued.
                    if !pulse_pending.swap(true, Ordering::AcqRel) {
                        let pulse = main_pulse.clone();
                        let pending = pulse_pending.clone();
                        if handle.run_on_main_thread(move || {
                            pulse.store(started.elapsed().as_millis() as u64, Ordering::Release);
                            pending.store(false, Ordering::Release);
                        }).is_err() {
                            pulse_pending.store(false, Ordering::Release);
                            health.fail(elapsed_ms, "MAIN_LOOP_PROBE_DISPATCH_FAILED");
                        }
                    }
                    let js_probe = probe.lock().unwrap().clone();
                    diagnostics = json!({
                        "notification_delivery": app_lib::traffic_tracer_test_support::notification_delivery::global().lock().snapshot(),
                        "recovery_delivery": &*app_lib::traffic_tracer_test_support::recovery_delivery::global().lock(),
                        "progress_delivery": app_lib::traffic_tracer_test_support::progress_delivery::global().lock().snapshot(now),
                        "desktop": app_lib::traffic_tracer_test_support::ui_desktop_snapshot(),
                        "native_main_loop_gap_ms": elapsed_ms.saturating_sub(main_pulse.load(Ordering::Acquire)),
                        "event_probe_gap_ms": js_probe.last.map(|last| now.saturating_duration_since(last).as_millis() as u64),
                        "event_probe": js_probe.data,
                    });
                    // Release the heartbeat mutex before /proc scans and disk I/O.
                    let p = progress.lock().unwrap().clone();
                    let gap = p.last.map_or(elapsed_ms, |last| now.saturating_duration_since(last).as_millis() as u64);
                    max_gap = max_gap.max(gap);
                    health.observe(elapsed_ms, gap, p.latest, p.rendered, p.errors);
                    let status = fs::read_to_string("/proc/self/status").unwrap_or_default();
                    let metrics: Vec<_> = status.lines().filter(|line| line.starts_with("VmRSS:") || line.starts_with("Threads:")).collect();
                    let sample = json!({"sample_index":tick,"elapsed_seconds":elapsed_ms / 1000,"wall_elapsed_ms":elapsed_ms,
                        "received":p.received,"latest_sequence":p.latest,
                        "remounts":p.remounts,"js_errors":p.errors,"heartbeat_gap_ms":gap,"max_gap_ms":max_gap,
                        "native_metrics":metrics,"native_fd_count":fs::read_dir("/proc/self/fd").map(|entries| entries.count()).ok(),
                        "rendered_sequence":p.rendered,"process_family":process_family_metrics(),
                        "failed":health.first_failure.is_some(),"first_failure":health.first_failure,
                        "page_metrics":p.page_metrics,"diagnostics":diagnostics,
                        "scope":if p.page_metrics.is_some() {"native_full_page_mocked_backend"} else {"native_bridge_react_progress"}});
                    if let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(root.join("samples.jsonl")) {
                        if writeln!(file, "{sample}").is_err() { health.fail(elapsed_ms, "SAMPLE_WRITE_FAILED"); }
                    } else { health.fail(elapsed_ms, "SAMPLE_WRITE_FAILED"); }
                    // Persist the first failing observation instead of wasting
                    // the rest of a multi-hour run or losing it on manual stop.
                    if soak_support::observation_finished(now >= deadline, health.first_failure.is_some(), diagnostic_only) { break; }
                }
                let p = progress.lock().unwrap().clone();
                if p.received == 0 || p.remounts == 0 || p.rendered == 0 {
                    health.fail(started.elapsed().as_millis() as u64, "MISSING_COVERAGE");
                }
                if let Some(page) = &p.page_metrics {
                    if page["detailDialogs"].as_u64().unwrap_or(0) == 0 || page["paginationClicks"].as_u64().unwrap_or(0) == 0 || page["historySelections"].as_u64().unwrap_or(0) == 0 {
                        health.fail(started.elapsed().as_millis() as u64, "MISSING_PAGE_COVERAGE");
                    }
                }
                stop_generation.store(true, Ordering::Release);
                let mut failed = health.first_failure.is_some() || diagnostic_only;
                failed |= write_report(&root, &json!({"schema_version":2,
                    "status":if diagnostic_only {"diagnostic_only"} else if failed {"failed"} else {"passed"},"first_failure":health.first_failure,
                    "diagnostic_only":diagnostic_only,
                    "passed":!failed,"seconds":seconds,"samples":tick,"fault":fault,
                    "wall_elapsed_ms":started.elapsed().as_millis(),"sample_delay_ms":sample_delay_ms,
                    "received":p.received,"remounts":p.remounts,"js_errors":p.errors,
                    "rendered_sequence":p.rendered,
                    "max_heartbeat_gap_ms":max_gap,
                    "scope":if p.page_metrics.is_some() {"native_full_page_mocked_backend"} else {"native_bridge_react_progress"},
                    "page_metrics":p.page_metrics,"diagnostics":diagnostics,"full_ui_soak":p.page_metrics.is_some()})).is_err();
                report_passed.store(!failed, Ordering::Release);
                println!("SOAK_FINISHED passed={}", !failed);
                handle.exit(if failed { 1 } else { 0 });
            });
            Ok(())
        })
        .build(context)
        .expect("isolated native test failed");
    // Builder::run may terminate with a platform-dependent code. run_return
    // lets the persisted outcome control our process status explicitly.
    application.run_return(|_, _| {});
    if !passed.load(Ordering::Acquire) {
        let current = fs::read(final_root.join("result.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok());
        if !soak_support::preserve_unsuccessful_report(current.as_ref().and_then(|v| v["status"].as_str())) {
            let _ = write_report(
                &final_root,
                &json!({"schema_version":2,"status":"interrupted",
                "passed":false,"reason":"EXIT_BEFORE_SUCCESSFUL_REPORT","full_ui_soak":false}),
            );
        }
        std::process::exit(1);
    }
    std::process::exit(0);
}
