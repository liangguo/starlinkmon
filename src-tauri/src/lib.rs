mod proto {
    tonic::include_proto!("space_x.api.device");
}

use proto::{device_client::DeviceClient, Request as DeviceRequest, GetStatusRequest, GetHistoryRequest};
use proto::dish_outage::Cause;
use serde::{Serialize, Deserialize};
use std::sync::Mutex;
use tauri::{
    Emitter, Manager, State,
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    image::Image,
};
use tokio::time::{interval, Duration};

const STARLINK_ADDR: &str = "http://192.168.100.1:9200";
const BG_INTERVAL_SECS: u64 = 60;
const PERF_RETENTION_HOURS: i64 = 24;
const EVENT_RETENTION_HOURS: i64 = 176;
const GPS_EPOCH_OFFSET: i64 = 315964800;

#[derive(Default, Clone, Serialize)]
pub struct DishStatus {
    pub connected: bool,
    pub downlink_mbps: f32,
    pub uplink_mbps: f32,
    pub latency_ms: f32,
    pub drop_rate: f32,
    pub uptime_s: u64,
    pub obstruction_pct: f32,
    pub snr_above_noise: bool,
    pub eth_speed_mbps: i32,
    pub software_version: String,
    pub hardware_version: String,
    pub country_code: String,
    pub azimuth_deg: f32,
    pub elevation_deg: f32,
    pub gps_valid: bool,
    pub gps_sats: u32,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct OutageEvent {
    pub cause: String,
    pub time: String,
    pub timestamp_ns: i64,
    pub duration_s: f64,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct PerfSample {
    pub ts: i64,
    pub dl: f32,
    pub ul: f32,
    pub latency: f32,
    pub drop_rate: f32,
    #[serde(default)]
    pub obstruction: f32,
}

#[derive(Default, Clone, Serialize)]
pub struct ObstructionMap {
    pub num_rows: u32,
    pub num_cols: u32,
    pub snr: Vec<f32>,
}

#[derive(Default, Clone, Serialize, Deserialize)]
pub struct PersistedData {
    pub perf: Vec<PerfSample>,
    pub events: Vec<OutageEvent>,
}

#[derive(Default, Clone, Serialize)]
pub struct AppState {
    pub status: DishStatus,
    pub obstruction_map: ObstructionMap,
}

struct SharedState {
    live: Mutex<AppState>,
    persisted: Mutex<PersistedData>,
}

// --- Persistence ---

fn data_dir() -> std::path::PathBuf {
    let dir = dirs::data_dir().unwrap_or_default().join("com.starlinkmon.app");
    std::fs::create_dir_all(&dir).ok();
    dir
}

fn load_persisted() -> PersistedData {
    let path = data_dir().join("data.json");
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_persisted(data: &PersistedData) {
    let path = data_dir().join("data.json");
    if let Ok(json) = serde_json::to_string(data) {
        let _ = std::fs::write(path, json);
    }
}

fn prune_persisted(data: &mut PersistedData) {
    let now = chrono::Utc::now().timestamp();
    let perf_cutoff = now - PERF_RETENTION_HOURS * 3600;
    let event_cutoff = now - EVENT_RETENTION_HOURS * 3600;
    data.perf.retain(|s| s.ts > perf_cutoff);
    data.events.retain(|e| (e.timestamp_ns / 1_000_000_000 + GPS_EPOCH_OFFSET) > event_cutoff);
}

// --- gRPC ---

fn cause_name(c: i32) -> &'static str {
    match Cause::try_from(c) {
        Ok(Cause::Booting) => "Booting",
        Ok(Cause::Stowed) => "Stowed",
        Ok(Cause::ThermalShutdown) => "Thermal Shutdown",
        Ok(Cause::NoSchedule) => "No Schedule",
        Ok(Cause::NoSats) => "No Satellites",
        Ok(Cause::Obstructed) => "Obstructed",
        Ok(Cause::NoDownlink) => "No Downlink",
        Ok(Cause::NoPings) => "No Pings",
        Ok(Cause::ActuatorActivity) => "Actuator",
        Ok(Cause::CableTest) => "Cable Test",
        Ok(Cause::Sleeping) => "Sleeping",
        Ok(Cause::MovingTooFast) => "Moving Too Fast",
        Ok(Cause::SkySearch) => "Sky Search",
        Ok(Cause::InhibitRf) => "RF Inhibited",
        _ => "Unknown",
    }
}

fn format_timestamp_ns(ns: i64) -> String {
    use chrono::{Local, TimeZone};
    let secs = ns / 1_000_000_000 + GPS_EPOCH_OFFSET;
    Local.timestamp_opt(secs, 0)
        .single()
        .map_or("--".to_string(), |dt| dt.format("%m-%d %H:%M:%S").to_string())
}

async fn poll_status(state: &SharedState) {
    let client = match DeviceClient::connect(STARLINK_ADDR).await {
        Ok(c) => c,
        Err(_) => { state.live.lock().unwrap().status.connected = false; return; }
    };
    let mut client = client;
    let req = DeviceRequest {
        id: 1, target_id: String::new(), epoch_id: 0,
        request: Some(proto::request::Request::GetStatus(GetStatusRequest {})),
    };
    if let Ok(resp) = client.handle(req).await {
        if let Some(proto::response::Response::DishGetStatus(s)) = resp.into_inner().response {
            let di = s.device_info.as_ref();
            let status = DishStatus {
                connected: true,
                downlink_mbps: s.downlink_throughput_bps / 1_000_000.0,
                uplink_mbps: s.uplink_throughput_bps / 1_000_000.0,
                latency_ms: s.pop_ping_latency_ms,
                drop_rate: s.pop_ping_drop_rate,
                uptime_s: s.device_state.as_ref().map_or(0, |st| st.uptime_s),
                obstruction_pct: s.obstruction_stats.as_ref().map_or(0.0, |o| o.fraction_obstructed * 100.0),
                snr_above_noise: s.is_snr_above_noise_floor,
                eth_speed_mbps: s.eth_speed_mbps,
                software_version: di.map_or(String::new(), |d| d.software_version.clone()),
                hardware_version: di.map_or(String::new(), |d| d.hardware_version.clone()),
                country_code: di.map_or(String::new(), |d| d.country_code.clone()),
                azimuth_deg: s.boresight_azimuth_deg,
                elevation_deg: s.boresight_elevation_deg,
                gps_valid: s.gps_stats.as_ref().map_or(false, |g| g.gps_valid),
                gps_sats: s.gps_stats.as_ref().map_or(0, |g| g.gps_sats),
            };
            state.live.lock().unwrap().status = status;
        }
    }
}

async fn poll_and_persist(state: &SharedState) {
    let client = match DeviceClient::connect(STARLINK_ADDR).await {
        Ok(c) => c,
        Err(_) => { state.live.lock().unwrap().status.connected = false; return; }
    };
    let mut client = client;

    // Get status
    let req = DeviceRequest {
        id: 1, target_id: String::new(), epoch_id: 0,
        request: Some(proto::request::Request::GetStatus(GetStatusRequest {})),
    };
    if let Ok(resp) = client.handle(req).await {
        if let Some(proto::response::Response::DishGetStatus(s)) = resp.into_inner().response {
            let di = s.device_info.as_ref();
            let status = DishStatus {
                connected: true,
                downlink_mbps: s.downlink_throughput_bps / 1_000_000.0,
                uplink_mbps: s.uplink_throughput_bps / 1_000_000.0,
                latency_ms: s.pop_ping_latency_ms,
                drop_rate: s.pop_ping_drop_rate,
                uptime_s: s.device_state.as_ref().map_or(0, |st| st.uptime_s),
                obstruction_pct: s.obstruction_stats.as_ref().map_or(0.0, |o| o.fraction_obstructed * 100.0),
                snr_above_noise: s.is_snr_above_noise_floor,
                eth_speed_mbps: s.eth_speed_mbps,
                software_version: di.map_or(String::new(), |d| d.software_version.clone()),
                hardware_version: di.map_or(String::new(), |d| d.hardware_version.clone()),
                country_code: di.map_or(String::new(), |d| d.country_code.clone()),
                azimuth_deg: s.boresight_azimuth_deg,
                elevation_deg: s.boresight_elevation_deg,
                gps_valid: s.gps_stats.as_ref().map_or(false, |g| g.gps_valid),
                gps_sats: s.gps_stats.as_ref().map_or(0, |g| g.gps_sats),
            };
            state.live.lock().unwrap().status = status;
        }
    }

    // Get history (perf data + outages) and merge
    let req = DeviceRequest {
        id: 2, target_id: String::new(), epoch_id: 0,
        request: Some(proto::request::Request::GetHistory(GetHistoryRequest {})),
    };
    if let Ok(resp) = client.handle(req).await {
        if let Some(proto::response::Response::DishGetHistory(h)) = resp.into_inner().response {
            let mut persisted = state.persisted.lock().unwrap();

            let n = h.downlink_throughput_bps.len();
            if n > 0 {
                let now_ts = chrono::Utc::now().timestamp();
                let last_ts = persisted.perf.last().map_or(0, |p| p.ts);
                for i in 0..n {
                    let ts = now_ts - (n as i64 - 1 - i as i64);
                    if ts <= last_ts { continue; }
                    persisted.perf.push(PerfSample {
                        ts,
                        dl: h.downlink_throughput_bps.get(i).copied().unwrap_or(0.0) / 1_000_000.0,
                        ul: h.uplink_throughput_bps.get(i).copied().unwrap_or(0.0) / 1_000_000.0,
                        latency: h.pop_ping_latency_ms.get(i).copied().unwrap_or(0.0),
                        drop_rate: h.pop_ping_drop_rate.get(i).copied().unwrap_or(0.0),
                        obstruction: state.live.lock().unwrap().status.obstruction_pct,
                    });
                }
            }

            for o in &h.outages {
                let ts = o.start_timestamp_ns;
                if !persisted.events.iter().any(|e| e.timestamp_ns == ts) {
                    persisted.events.push(OutageEvent {
                        cause: cause_name(o.cause).to_string(),
                        time: format_timestamp_ns(ts),
                        timestamp_ns: ts,
                        duration_s: o.duration_ns as f64 / 1_000_000_000.0,
                    });
                }
            }
            persisted.events.sort_by(|a, b| b.timestamp_ns.cmp(&a.timestamp_ns));
        }
    }

    // Get obstruction map
    let req = DeviceRequest {
        id: 3, target_id: String::new(), epoch_id: 0,
        request: Some(proto::request::Request::DishGetObstructionMap(proto::DishGetObstructionMapRequest {})),
    };
    if let Ok(resp) = client.handle(req).await {
        if let Some(proto::response::Response::DishGetObstructionMap(m)) = resp.into_inner().response {
            state.live.lock().unwrap().obstruction_map = ObstructionMap {
                num_rows: m.num_rows, num_cols: m.num_cols, snr: m.snr,
            };
        }
    }

    let mut persisted = state.persisted.lock().unwrap();
    prune_persisted(&mut persisted);
    save_persisted(&persisted);
}

// --- Tauri commands ---

#[tauri::command]
fn get_status(state: State<SharedState>) -> DishStatus {
    state.live.lock().unwrap().status.clone()
}

#[tauri::command]
fn get_outages(state: State<SharedState>) -> Vec<OutageEvent> {
    state.persisted.lock().unwrap().events.iter().take(50).cloned().collect()
}

#[tauri::command]
fn get_perf_history(state: State<SharedState>) -> Vec<PerfSample> {
    state.persisted.lock().unwrap().perf.clone()
}

#[tauri::command]
fn get_obstruction_map(state: State<SharedState>) -> ObstructionMap {
    state.live.lock().unwrap().obstruction_map.clone()
}

// --- App setup ---

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let persisted_data = load_persisted();
    let shared = SharedState {
        live: Mutex::new(AppState::default()),
        persisted: Mutex::new(persisted_data),
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(shared)
        .invoke_handler(tauri::generate_handler![get_status, get_outages, get_perf_history, get_obstruction_map])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                window.hide().unwrap();
                api.prevent_close();
            }
        })
        .setup(move |app| {
            #[cfg(target_os = "macos")]
            {
                use cocoa::appkit::{NSApp, NSApplication, NSApplicationActivationPolicy};
                unsafe {
                    NSApp().setActivationPolicy_(NSApplicationActivationPolicy::NSApplicationActivationPolicyAccessory);
                }
            }

            use tauri::menu::PredefinedMenuItem;
            let mi_status = MenuItem::with_id(app, "mi_status", "Starlink: Connecting...", false, None::<&str>)?;
            let mi_throughput = MenuItem::with_id(app, "mi_throughput", "↓ --  ↑ -- Mbps", false, None::<&str>)?;
            let mi_latency = MenuItem::with_id(app, "mi_latency", "Latency: --", false, None::<&str>)?;
            let mi_obstruction = MenuItem::with_id(app, "mi_obstruction", "Obstruction: --", false, None::<&str>)?;
            let mi_event1 = MenuItem::with_id(app, "mi_ev1", "--", false, None::<&str>)?;
            let mi_event2 = MenuItem::with_id(app, "mi_ev2", "--", false, None::<&str>)?;
            let mi_event3 = MenuItem::with_id(app, "mi_ev3", "--", false, None::<&str>)?;
            let sep = PredefinedMenuItem::separator(app)?;
            let sep2 = PredefinedMenuItem::separator(app)?;
            let sep3 = PredefinedMenuItem::separator(app)?;
            let show_i = MenuItem::with_id(app, "show", "Show Monitor", true, None::<&str>)?;
            let quit_i = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;

            let menu = Menu::with_items(app, &[
                &mi_status, &mi_throughput, &mi_latency, &mi_obstruction,
                &sep,
                &mi_event1, &mi_event2, &mi_event3,
                &sep2,
                &show_i, &sep3, &quit_i,
            ])?;

            let icon = Image::from_bytes(include_bytes!("../icons/32x32.png"))?;
            let _tray = TrayIconBuilder::new()
                .icon(icon)
                .icon_as_template(true)
                .menu(&menu)
                .tooltip("Starlink Monitor")
                .show_menu_on_left_click(true)
                .on_menu_event(|app, event| {
                    match event.id().as_ref() {
                        "show" => {
                            if let Some(w) = app.get_webview_window("main") {
                                let _ = w.show();
                                let _ = w.set_focus();
                            }
                        }
                        "quit" => app.exit(0),
                        _ => {}
                    }
                })
                .build(app)?;

            // Single background task
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let mut status_ticker = interval(Duration::from_secs(1));
                let mut extras_ticker = interval(Duration::from_secs(BG_INTERVAL_SECS));

                // Initial full poll
                let state = handle.state::<SharedState>();
                poll_and_persist(&*state).await;
                update_menu(&state, &mi_status, &mi_throughput, &mi_latency, &mi_obstruction, &mi_event1, &mi_event2, &mi_event3);
                let _ = handle.emit("starlink-update", ());

                loop {
                    tokio::select! {
                        _ = status_ticker.tick() => {
                            let state = handle.state::<SharedState>();
                            poll_status(&*state).await;
                        }
                        _ = extras_ticker.tick() => {
                            let state = handle.state::<SharedState>();
                            poll_and_persist(&*state).await;
                        }
                    }

                    let state = handle.state::<SharedState>();
                    update_menu(&state, &mi_status, &mi_throughput, &mi_latency, &mi_obstruction, &mi_event1, &mi_event2, &mi_event3);
                    let _ = handle.emit("starlink-update", ());
                }
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn update_menu(
    state: &SharedState,
    mi_status: &MenuItem<tauri::Wry>,
    mi_throughput: &MenuItem<tauri::Wry>,
    mi_latency: &MenuItem<tauri::Wry>,
    mi_obstruction: &MenuItem<tauri::Wry>,
    mi_ev1: &MenuItem<tauri::Wry>,
    mi_ev2: &MenuItem<tauri::Wry>,
    mi_ev3: &MenuItem<tauri::Wry>,
) {
    let live = state.live.lock().unwrap();
    if live.status.connected {
        let _ = mi_status.set_text("Starlink: Connected");
        let _ = mi_throughput.set_text(format!("↓ {:.1}  ↑ {:.1} Mbps", live.status.downlink_mbps, live.status.uplink_mbps));
        let _ = mi_latency.set_text(format!("Latency: {:.0} ms", live.status.latency_ms));
        let _ = mi_obstruction.set_text(format!("Obstruction: {:.2}%", live.status.obstruction_pct));
    } else {
        let _ = mi_status.set_text("Starlink: Disconnected");
        let _ = mi_throughput.set_text("↓ --  ↑ -- Mbps");
        let _ = mi_latency.set_text("Latency: --");
        let _ = mi_obstruction.set_text("Obstruction: --");
    }
    drop(live);

    let persisted = state.persisted.lock().unwrap();
    let events = [mi_ev1, mi_ev2, mi_ev3];
    for (i, mi) in events.iter().enumerate() {
        if let Some(ev) = persisted.events.get(i) {
            let _ = mi.set_text(format!("{} {} ({:.1}s)", ev.time, ev.cause, ev.duration_s));
        } else {
            let _ = mi.set_text("--");
        }
    }
}
