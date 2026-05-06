use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    fs::File,
    fs,
    io::BufReader,
    net::SocketAddr,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, Runtime, State,
};
use tokio::{
    io::AsyncReadExt,
    net::TcpListener,
    sync::oneshot,
    task::JoinHandle,
};

const TRAY_ICON: tauri::image::Image<'_> = tauri::include_image!("./icons/icon.png");

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AppConfig {
    #[serde(default)]
    server: ServerConfig,
    #[serde(default)]
    local: LocalConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ServerConfig {
    #[serde(default = "default_url")]
    url: String,
    #[serde(default = "default_username")]
    username: String,
    #[serde(default = "default_password")]
    password: String,
    #[serde(default = "default_autorun")]
    autorun: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LocalConfig {
    #[serde(default = "default_port")]
    port: u16,
    #[serde(default = "default_identifier")]
    identifier: String,
}

#[derive(Debug, Clone, Serialize)]
struct ListenerStatus {
    running: bool,
    port: u16,
    device_count: usize,
}

#[derive(Debug, Clone, Serialize)]
struct DeviceInfo {
    number: String,
    info: String,
}

#[derive(Debug, Clone, Serialize)]
struct PatientOption {
    patient_name: String,
    patient_id: String,
    appointment_time: String,
    treatment_room: String,
    waiting_order: String,
    check_item: String,
    display: String,
}

#[derive(Debug, Clone, Serialize)]
struct IncomingCommand {
    server_ip: String,
    client_ip: String,
    command: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct BindPayload {
    #[serde(rename = "Number")]
    number: String,
    #[serde(rename = "Namepatient")]
    namepatient: String,
    #[serde(rename = "Patientvisitid")]
    patientvisitid: String,
    #[serde(rename = "Checkreportid")]
    checkreportid: String,
    #[serde(rename = "Rfidno")]
    rfidno: String,
    #[serde(rename = "Bindmirrostate")]
    bindmirrostate: String,
}

struct ListenerHandle {
    shutdown: oneshot::Sender<()>,
    join: JoinHandle<()>,
}

struct AppStateInner {
    config: Mutex<AppConfig>,
    devices: Mutex<HashMap<String, String>>,
    listener: Mutex<Option<ListenerHandle>>,
}

type AppState = Arc<AppStateInner>;

fn default_url() -> String {
    "http://192.168.1.11:8866/".to_string()
}

fn default_username() -> String {
    "admin".to_string()
}

fn default_password() -> String {
    "password".to_string()
}

fn default_autorun() -> bool {
    true
}

fn default_port() -> u16 {
    9000
}

fn default_identifier() -> String {
    "A1".to_string()
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            url: default_url(),
            username: default_username(),
            password: default_password(),
            autorun: default_autorun(),
        }
    }
}

impl Default for LocalConfig {
    fn default() -> Self {
        Self {
            port: default_port(),
            identifier: default_identifier(),
        }
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            server: ServerConfig::default(),
            local: LocalConfig::default(),
        }
    }
}

fn app_sidecar_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;

    #[cfg(target_os = "macos")]
    {
        for ancestor in exe.ancestors() {
            if ancestor.extension().and_then(|ext| ext.to_str()) == Some("app") {
                return ancestor.parent().map(PathBuf::from);
            }
        }
    }

    exe.parent().map(PathBuf::from)
}

fn config_path<R: Runtime>(app: &AppHandle<R>) -> PathBuf {
    let app_sidecar_config = app_sidecar_dir().map(|path| path.join("config.toml"));
    if let Some(path) = app_sidecar_config.as_ref().filter(|path| path.exists()) {
        return path.clone();
    }

    let cwd_config = std::env::current_dir()
        .ok()
        .map(|path| path.join("config.toml"))
        .filter(|path| path.exists());
    if let Some(path) = cwd_config {
        return path;
    }

    if let Some(path) = app_sidecar_config {
        return path;
    }

    app.path()
        .app_config_dir()
        .unwrap_or_else(|_| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
        .join("config.toml")
}

fn load_config_from_path(path: &PathBuf) -> AppConfig {
    fs::read_to_string(path)
        .ok()
        .and_then(|text| toml::from_str::<AppConfig>(&text).ok())
        .unwrap_or_default()
}

fn save_config_to_path(path: &PathBuf, config: &AppConfig) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, toml::to_string_pretty(config)?)?;
    Ok(())
}

fn api_url(config: &AppConfig, resource: &str) -> String {
    format!(
        "{}/{}",
        config.server.url.trim_end_matches('/'),
        resource.trim_start_matches('/')
    )
}

async fn api_get(config: AppConfig, resource: String) -> Result<Value> {
    let client = reqwest::Client::new();
    let res = client
        .get(api_url(&config, &resource))
        .send()
        .await
        .context("请求数据池服务失败")?;
    let text = res.text().await.context("读取数据池响应失败")?;
    if text.trim().is_empty() {
        return Err(anyhow!("数据池服务返回空内容"));
    }
    serde_json::from_str(&text).with_context(|| format!("数据池返回的 JSON 无效: {text}"))
}

async fn api_post<T: Serialize>(config: AppConfig, resource: String, body: T) -> Result<Value> {
    let client = reqwest::Client::new();
    let res = client
        .post(api_url(&config, &resource))
        .json(&body)
        .send()
        .await
        .context("请求数据池服务失败")?;
    let text = res.text().await.context("读取数据池响应失败")?;
    if text.trim().is_empty() {
        return Err(anyhow!("数据池服务返回空内容"));
    }
    serde_json::from_str(&text).with_context(|| format!("数据池返回的 JSON 无效: {text}"))
}

async fn load_devices_into_state(state: &AppState) -> Result<Vec<DeviceInfo>> {
    let config = state.config.lock().unwrap().clone();
    let value = api_get(config, "device".to_string()).await?;
    let devices = value
        .as_array()
        .ok_or_else(|| anyhow!("device 接口没有返回数组"))?;

    let mut mapped = HashMap::new();
    let mut list = Vec::new();
    for item in devices {
        let number = item
            .get("EndoscopeNumber")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if number.is_empty() {
            continue;
        }
        let device_type = item
            .get("EndoscopeType")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let info = item
            .get("EndoscopeInfo")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let display = format!("{device_type} {info}").trim().to_string();
        mapped.insert(number.clone(), display.clone());
        list.push(DeviceInfo {
            number,
            info: display,
        });
    }

    *state.devices.lock().unwrap() = mapped;
    Ok(list)
}

fn parse_command(data: &[u8]) -> String {
    if data.len() == 10 && data.first() == Some(&0x00) {
        let mut no: u32 = 0;
        for (offset, byte) in data[6..10].iter().enumerate() {
            no += (*byte as u32) << ((3 - offset) * 8);
        }
        return format!("{no:010}");
    }

    if data.len() == 5 {
        return data.iter().map(|byte| format!("{byte:02x}")).collect();
    }

    data.iter().map(|byte| format!("{byte:02X}")).collect()
}

async fn handle_client<R: Runtime>(
    app: AppHandle<R>,
    peer: SocketAddr,
    mut stream: tokio::net::TcpStream,
    server_ip: String,
) {
    loop {
        let mut buffer = [0u8; 1024];
        let len = match stream.read(&mut buffer).await {
            Ok(0) => return,
            Ok(len) => len,
            Err(error) => {
                eprintln!("client read error from {peer}: {error}");
                return;
            }
        };

        let command = parse_command(&buffer[..len]);
        if command.is_empty() {
            continue;
        }

        let payload = IncomingCommand {
            server_ip: server_ip.clone(),
            client_ip: peer.to_string(),
            command,
        };
        play_sound(config_path(&app), "ding.wav");
        show_window_for_incoming(&app);
        let _ = app.emit("ant://incoming-command", payload);
    }
}

async fn run_listener<R: Runtime>(
    app: AppHandle<R>,
    port: u16,
    mut shutdown_rx: oneshot::Receiver<()>,
) -> Result<()> {
    let listener = TcpListener::bind(("0.0.0.0", port))
        .await
        .with_context(|| format!("无法监听本地端口 {port}"))?;
    let server_ip = format!("0.0.0.0:{port}");

    loop {
        tokio::select! {
            _ = &mut shutdown_rx => {
                return Ok(());
            }
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, peer)) => {
                        let app_handle = app.clone();
                        let server_ip = server_ip.clone();
                        tokio::spawn(async move {
                            handle_client(app_handle, peer, stream, server_ip).await;
                        });
                    }
                    Err(error) => {
                        eprintln!("listener accept error: {error}");
                    }
                }
            }
        }
    }
}

#[tauri::command]
fn get_config<R: Runtime>(app: AppHandle<R>, state: State<'_, AppState>) -> Result<AppConfig, String> {
    let path = config_path(&app);
    let config = load_config_from_path(&path);
    *state.config.lock().unwrap() = config.clone();
    Ok(config)
}

#[tauri::command]
fn save_config<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    config: AppConfig,
) -> Result<(), String> {
    let path = config_path(&app);
    save_config_to_path(&path, &config).map_err(|error| error.to_string())?;
    *state.config.lock().unwrap() = config;
    Ok(())
}

#[tauri::command]
async fn refresh_devices(state: State<'_, AppState>) -> Result<Vec<DeviceInfo>, String> {
    load_devices_into_state(&state)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn start_listener<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
) -> Result<ListenerStatus, String> {
    if state.listener.lock().unwrap().is_some() {
        let config = state.config.lock().unwrap().clone();
        let device_count = state.devices.lock().unwrap().len();
        return Ok(ListenerStatus {
            running: true,
            port: config.local.port,
            device_count,
        });
    }

    load_devices_into_state(&state)
        .await
        .map_err(|error| error.to_string())?;

    let config = state.config.lock().unwrap().clone();
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let app_handle = app.clone();
    let port = config.local.port;
    let join = tokio::spawn(async move {
        if let Err(error) = run_listener(app_handle, port, shutdown_rx).await {
            eprintln!("{error}");
        }
    });

    *state.listener.lock().unwrap() = Some(ListenerHandle {
        shutdown: shutdown_tx,
        join,
    });

    Ok(ListenerStatus {
        running: true,
        port,
        device_count: state.devices.lock().unwrap().len(),
    })
}

#[tauri::command]
async fn stop_listener(state: State<'_, AppState>) -> Result<ListenerStatus, String> {
    if let Some(handle) = state.listener.lock().unwrap().take() {
        let _ = handle.shutdown.send(());
        handle.join.abort();
    }
    let config = state.config.lock().unwrap().clone();
    Ok(ListenerStatus {
        running: false,
        port: config.local.port,
        device_count: state.devices.lock().unwrap().len(),
    })
}

#[tauri::command]
fn get_listener_status(state: State<'_, AppState>) -> ListenerStatus {
    let config = state.config.lock().unwrap().clone();
    ListenerStatus {
        running: state.listener.lock().unwrap().is_some(),
        port: config.local.port,
        device_count: state.devices.lock().unwrap().len(),
    }
}

#[tauri::command]
fn get_device_info(state: State<'_, AppState>, enumber: String) -> Option<String> {
    state.devices.lock().unwrap().get(&enumber).cloned()
}

#[tauri::command]
async fn fetch_last_record(state: State<'_, AppState>, enumber: String) -> Result<Value, String> {
    let config = state.config.lock().unwrap().clone();
    api_get(config, format!("lastrecordbyeid/{enumber}"))
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn fetch_patient_names(state: State<'_, AppState>) -> Result<Vec<PatientOption>, String> {
    let config = state.config.lock().unwrap().clone();
    let identifier = config.local.identifier.clone();
    let value = api_post(config, "getPatientNameList".to_string(), json!({ "identifier": identifier }))
        .await
        .map_err(|error| error.to_string())?;
    if value
        .get("success")
        .and_then(Value::as_bool)
        .is_some_and(|success| !success)
    {
        return Ok(Vec::new());
    }
    if let Some(patients) = value.get("patients").and_then(Value::as_array) {
        return Ok(patients
            .iter()
            .map(|patient| {
                let patient_name = patient
                    .get("patient_name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let patient_id = patient
                    .get("patient_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let check_item = patient
                    .get("check_item")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let display = if patient_id.is_empty() && check_item.is_empty() {
                    patient_name.clone()
                } else {
                    format!("{patient_name}:{patient_id}:{check_item}")
                };
                PatientOption {
                    patient_name,
                    patient_id,
                    appointment_time: patient
                        .get("appointment_time")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    treatment_room: patient
                        .get("treatment_room")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    waiting_order: patient
                        .get("waiting_order")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    check_item,
                    display,
                }
            })
            .collect());
    }

    Ok(value
        .get("names")
        .and_then(Value::as_array)
        .map(|names| {
            names
                .iter()
                .filter_map(Value::as_str)
                .map(|name| PatientOption {
                    patient_name: name.split(':').next().unwrap_or(name).to_string(),
                    patient_id: name.split(':').nth(1).unwrap_or_default().to_string(),
                    appointment_time: String::new(),
                    treatment_room: String::new(),
                    waiting_order: String::new(),
                    check_item: name.split(':').nth(2).unwrap_or_default().to_string(),
                    display: name.to_string(),
                })
                .collect()
        })
        .unwrap_or_default())
}

#[tauri::command]
async fn bind_patient<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    payload: BindPayload,
) -> Result<Value, String> {
    let config = state.config.lock().unwrap().clone();
    let sound_file = match payload.bindmirrostate.as_str() {
        "0" => "bdjc.wav",
        "1" => "bdcg.wav",
        _ => "ding.wav",
    };
    let value = api_post(config, "writeback2".to_string(), payload)
        .await
        .map_err(|error| error.to_string())?;

    if value
        .get("success")
        .and_then(Value::as_bool)
        .is_some_and(|success| success)
    {
        play_sound(config_path(&app), sound_file);
    }

    Ok(value)
}

#[tauri::command]
async fn manual_read<R: Runtime>(
    app: AppHandle<R>,
    command: String,
) -> Result<(), String> {
    play_sound(config_path(&app), "ding.wav");
    show_window_for_incoming(&app);
    app.emit(
        "ant://incoming-command",
        IncomingCommand {
            server_ip: String::new(),
            client_ip: "manual".to_string(),
            command,
        },
    )
    .map_err(|error| error.to_string())
}

fn show_main_window<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn show_window_for_incoming<R: Runtime>(app: &AppHandle<R>) {
    #[cfg(target_os = "macos")]
    {
        let _ = app.set_activation_policy(tauri::ActivationPolicy::Regular);
    }

    show_main_window(app);
}

fn play_sound(config_path: PathBuf, file_name: &'static str) {
    let Some(path) = config_path
        .parent()
        .map(|path| path.join(file_name))
        .filter(|path| path.exists())
    else {
        return;
    };

    std::thread::spawn(move || {
        let Ok(stream) = rodio::OutputStreamBuilder::open_default_stream() else {
            return;
        };
        let Ok(file) = File::open(&path) else {
            return;
        };
        let Ok(sink) = rodio::play(stream.mixer(), BufReader::new(file)) else {
            return;
        };
        sink.sleep_until_end();
    });
}

fn build_tray<R: Runtime>(app: &tauri::App<R>) -> Result<()> {
    let show = MenuItem::with_id(app, "show", "显示主窗口", true, None::<&str>)?;
    let start = MenuItem::with_id(app, "start", "启动监听", true, None::<&str>)?;
    let stop = MenuItem::with_id(app, "stop", "停止监听", true, None::<&str>)?;
    let config = MenuItem::with_id(app, "config", "配置", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &start, &stop, &config, &quit])?;

    TrayIconBuilder::with_id("main-tray")
        .icon(TRAY_ICON)
        .icon_as_template(true)
        .tooltip("AntListener 2026")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(&tray.app_handle());
            }
        })
        .build(app)?;

    let app_handle = app.handle().clone();
    app.on_menu_event(move |app, event| match event.id().as_ref() {
        "show" => show_main_window(app),
        "config" => {
            show_main_window(app);
            let _ = app_handle.emit("ant://open-config", ());
        }
        "start" => {
            let _ = app_handle.emit("ant://tray-start", ());
        }
        "stop" => {
            let _ = app_handle.emit("ant://tray-stop", ());
        }
        "quit" => {
            std::process::exit(0);
        }
        _ => {}
    });

    Ok(())
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_main_window(app);
        }))
        .manage(Arc::new(AppStateInner {
            config: Mutex::new(AppConfig::default()),
            devices: Mutex::new(HashMap::new()),
            listener: Mutex::new(None),
        }))
        .invoke_handler(tauri::generate_handler![
            get_config,
            save_config,
            refresh_devices,
            start_listener,
            stop_listener,
            get_listener_status,
            get_device_info,
            fetch_last_record,
            fetch_patient_names,
            bind_patient,
            manual_read
        ])
        .setup(|app| {
            #[cfg(target_os = "macos")]
            {
                app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            }

            let path = config_path(app.handle());
            let config = load_config_from_path(&path);
            let state = app.state::<AppState>();
            *state.config.lock().unwrap() = config.clone();
            build_tray(app)?;
            if config.server.autorun {
                let app_handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    let app_for_state = app_handle.clone();
                    let state = app_for_state.state::<AppState>();
                    let _ = start_listener(app_handle, state).await;
                });
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running AntListener 2026");
}

#[cfg(test)]
mod tests {
    use super::parse_command;

    #[test]
    fn parses_legacy_ten_byte_card_number() {
        let packet = [0x00, 0x00, 0x00, 0x08, 0x04, 0x00, 0x02, 0xCB, 0x7A, 0xEE];
        assert_eq!(parse_command(&packet), "0046889710");
    }

    #[test]
    fn parses_five_byte_packet_as_lowercase_hex() {
        let packet = [0x12, 0xAB, 0x00, 0x7F, 0x09];
        assert_eq!(parse_command(&packet), "12ab007f09");
    }

    #[test]
    fn parses_other_packets_as_uppercase_hex() {
        let packet = [0x12, 0xAB, 0x00];
        assert_eq!(parse_command(&packet), "12AB00");
    }
}
