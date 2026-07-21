use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File, OpenOptions},
    io::{BufReader, Write},
    net::{IpAddr, SocketAddr},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
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
    task::{JoinHandle, JoinSet},
};

const TRAY_ICON: tauri::image::Image<'_> = tauri::include_image!("./icons/icon.png");
const MAX_CLIENT_CONNECTIONS: usize = 32;
const MAX_LOG_FILE_SIZE: u64 = 5 * 1024 * 1024;
const HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const HTTP_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
    #[serde(default = "default_autorun")]
    autorun: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LocalConfig {
    #[serde(default = "default_port")]
    port: u16,
    #[serde(default = "default_identifier")]
    identifier: String,
    #[serde(default)]
    allowed_ips: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
struct ListenerStatus {
    running: bool,
    port: u16,
    device_count: usize,
    active_connections: usize,
    last_error: Option<String>,
    log_path: Option<String>,
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
    port: u16,
    shutdown: oneshot::Sender<()>,
    join: JoinHandle<()>,
}

struct ListenerStartingGuard<'a>(&'a AtomicBool);

impl Drop for ListenerStartingGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

struct AppStateInner {
    config: Mutex<AppConfig>,
    devices: Mutex<HashMap<String, String>>,
    listener: Mutex<Option<ListenerHandle>>,
    listener_starting: AtomicBool,
    active_connections: Arc<AtomicUsize>,
    last_error: Mutex<Option<String>>,
    log_path: Mutex<Option<PathBuf>>,
    http: reqwest::Client,
}

type AppState = Arc<AppStateInner>;

fn default_url() -> String {
    "http://127.0.0.1:8866/".to_string()
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
            autorun: default_autorun(),
        }
    }
}

impl Default for LocalConfig {
    fn default() -> Self {
        Self {
            port: default_port(),
            identifier: default_identifier(),
            allowed_ips: Vec::new(),
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

    app.path()
        .app_config_dir()
        .unwrap_or_else(|_| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
        .join("config.toml")
}

fn validate_config(config: &AppConfig) -> Result<HashSet<IpAddr>> {
    let url =
        reqwest::Url::parse(config.server.url.trim()).context("数据池服务地址不是有效 URL")?;
    if !matches!(url.scheme(), "http" | "https") || url.host().is_none() {
        return Err(anyhow!(
            "数据池服务地址必须是包含主机名的 http 或 https URL"
        ));
    }
    if config.local.port == 0 {
        return Err(anyhow!("本地监听端口必须在 1 到 65535 之间"));
    }
    if config.local.identifier.trim().is_empty() {
        return Err(anyhow!("本机编号不能为空"));
    }

    config
        .local
        .allowed_ips
        .iter()
        .map(|value| {
            let address = value
                .trim()
                .parse::<IpAddr>()
                .with_context(|| format!("允许的设备 IP 无效: {value}"))?;
            if !address.is_ipv4() {
                return Err(anyhow!("监听器当前仅支持 IPv4，不能使用: {value}"));
            }
            Ok(address)
        })
        .collect()
}

fn load_config_from_path(path: &PathBuf) -> Result<AppConfig> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("读取配置文件失败: {}", path.display()))?;
    let config = toml::from_str::<AppConfig>(&text)
        .with_context(|| format!("解析配置文件失败: {}", path.display()))?;
    validate_config(&config)?;
    Ok(config)
}

fn save_config_to_path(path: &PathBuf, config: &AppConfig) -> Result<()> {
    validate_config(config)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary_path = path.with_extension("toml.tmp");
    fs::write(&temporary_path, toml::to_string_pretty(config)?)?;
    fs::rename(&temporary_path, path)?;
    Ok(())
}

fn append_log(state: &AppState, level: &str, message: &str) {
    let log_path = state.log_path.lock().unwrap();
    let Some(path) = log_path.as_ref() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let timestamp = time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "unknown-time".to_string());
    if fs::metadata(path).is_ok_and(|metadata| metadata.len() >= MAX_LOG_FILE_SIZE) {
        let rotated_path = path.with_extension("log.1");
        let _ = fs::remove_file(&rotated_path);
        let _ = fs::rename(path, rotated_path);
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "[{timestamp}] [{level}] {message}");
    }
}

fn record_error(state: &AppState, message: impl Into<String>) -> String {
    let message = message.into();
    *state.last_error.lock().unwrap() = Some(message.clone());
    append_log(state, "ERROR", &message);
    message
}

fn clear_error(state: &AppState) {
    *state.last_error.lock().unwrap() = None;
}

fn api_url(config: &AppConfig, resource: &str) -> Result<reqwest::Url> {
    let base = format!("{}/", config.server.url.trim_end_matches('/'));
    reqwest::Url::parse(&base)?
        .join(resource.trim_start_matches('/'))
        .context("构造数据池请求地址失败")
}

async fn api_get(state: &AppState, resource: &str) -> Result<Value> {
    let config = state.config.lock().unwrap().clone();
    let url = api_url(&config, resource)?;
    let mut attempt = 0;
    let res = loop {
        match state.http.get(url.clone()).send().await {
            Ok(response) => break response,
            Err(error) if attempt == 0 && (error.is_connect() || error.is_timeout()) => {
                attempt += 1;
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            Err(error) => return Err(error).context("请求数据池服务失败"),
        }
    }
    .error_for_status()
    .context("数据池服务返回失败状态")?;
    res.json::<Value>()
        .await
        .context("数据池服务返回的 JSON 无效")
}

async fn api_post<T: Serialize>(state: &AppState, resource: &str, body: T) -> Result<Value> {
    let config = state.config.lock().unwrap().clone();
    let res = state
        .http
        .post(api_url(&config, resource)?)
        .json(&body)
        .send()
        .await
        .context("请求数据池服务失败")?
        .error_for_status()
        .context("数据池服务返回失败状态")?;
    res.json::<Value>()
        .await
        .context("数据池服务返回的 JSON 无效")
}

fn verify_binding_state(
    value: &Value,
    expected_number: &str,
    expected_patient: &str,
) -> Result<()> {
    if value.get("success").and_then(Value::as_bool) != Some(true) {
        return Err(anyhow!("服务端未返回本次洗消记录"));
    }
    let ant = value
        .get("ant")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("服务端复核响应缺少洗消记录"))?;
    let actual_number = ant
        .get("number")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    let actual_patient = ant
        .get("patient")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    if actual_number != expected_number.trim() {
        return Err(anyhow!(
            "服务端复核洗消编号不一致，期望 {}，实际 {}",
            expected_number,
            actual_number
        ));
    }
    if actual_patient != expected_patient.trim() {
        return Err(anyhow!(
            "服务端复核病人姓名不一致，期望 {}，实际 {}",
            expected_patient,
            actual_patient
        ));
    }
    Ok(())
}

async fn load_devices_into_state(state: &AppState) -> Result<Vec<DeviceInfo>> {
    let value = api_get(state, "device").await?;
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

#[derive(Default)]
struct PacketDecoder {
    pending: Vec<u8>,
}

impl PacketDecoder {
    fn push(&mut self, data: &[u8]) -> Vec<Vec<u8>> {
        self.pending.extend_from_slice(data);
        let mut packets = Vec::new();
        while let Some(first) = self.pending.first() {
            let packet_length = if *first == 0x00 { 10 } else { 5 };
            if self.pending.len() < packet_length {
                break;
            }
            packets.push(self.pending.drain(..packet_length).collect());
        }
        packets
    }

    fn pending_len(&self) -> usize {
        self.pending.len()
    }
}

fn parse_command(data: &[u8]) -> Result<String> {
    if data.len() == 10 && data.first() == Some(&0x00) {
        let mut no: u32 = 0;
        for (offset, byte) in data[6..10].iter().enumerate() {
            no += (*byte as u32) << ((3 - offset) * 8);
        }
        return Ok(format!("{no:010}"));
    }

    if data.len() == 5 {
        return Ok(data.iter().map(|byte| format!("{byte:02x}")).collect());
    }

    Err(anyhow!("不支持的读卡器数据帧长度: {}", data.len()))
}

struct ConnectionGuard(Arc<AtomicUsize>);

impl ConnectionGuard {
    fn new(counter: Arc<AtomicUsize>) -> Self {
        counter.fetch_add(1, Ordering::Relaxed);
        Self(counter)
    }
}

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

async fn handle_client<R: Runtime>(
    app: AppHandle<R>,
    state: AppState,
    peer: SocketAddr,
    mut stream: tokio::net::TcpStream,
    server_ip: String,
) {
    let _connection_guard = ConnectionGuard::new(state.active_connections.clone());
    append_log(&state, "INFO", &format!("设备连接已建立: {peer}"));
    let mut decoder = PacketDecoder::default();
    loop {
        let mut buffer = [0u8; 512];
        let len = match stream.read(&mut buffer).await {
            Ok(0) => {
                if decoder.pending_len() > 0 {
                    append_log(
                        &state,
                        "WARN",
                        &format!(
                            "设备 {peer} 断开时留下 {} 个未完成字节",
                            decoder.pending_len()
                        ),
                    );
                }
                append_log(&state, "INFO", &format!("设备连接已断开: {peer}"));
                return;
            }
            Ok(len) => len,
            Err(error) => {
                append_log(
                    &state,
                    "ERROR",
                    &format!("读取设备 {peer} 数据失败: {error}"),
                );
                return;
            }
        };

        for packet in decoder.push(&buffer[..len]) {
            match parse_command(&packet) {
                Ok(command) => {
                    let payload = IncomingCommand {
                        server_ip: server_ip.clone(),
                        client_ip: peer.to_string(),
                        command,
                    };
                    play_sound(config_path(&app), "ding.wav");
                    show_window_for_incoming(&app);
                    if let Err(error) = app.emit("ant://incoming-command", payload) {
                        append_log(&state, "ERROR", &format!("发送刷卡事件失败: {error}"));
                    }
                }
                Err(error) => {
                    append_log(
                        &state,
                        "WARN",
                        &format!("忽略设备 {peer} 的无效数据: {error}"),
                    );
                }
            }
        }
    }
}

async fn run_listener<R: Runtime>(
    app: AppHandle<R>,
    state: AppState,
    listener: TcpListener,
    allowed_ips: HashSet<IpAddr>,
    mut shutdown_rx: oneshot::Receiver<()>,
) -> Result<()> {
    let server_ip = listener.local_addr()?.to_string();
    let mut clients = JoinSet::new();

    loop {
        tokio::select! {
            _ = &mut shutdown_rx => {
                clients.abort_all();
                while clients.join_next().await.is_some() {}
                return Ok(());
            }
            Some(result) = clients.join_next(), if !clients.is_empty() => {
                if let Err(error) = result {
                    append_log(&state, "ERROR", &format!("设备连接任务异常结束: {error}"));
                }
            }
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, peer)) => {
                        if !allowed_ips.is_empty() && !allowed_ips.contains(&peer.ip()) {
                            append_log(&state, "WARN", &format!("拒绝未授权设备 IP: {}", peer.ip()));
                            continue;
                        }
                        if clients.len() >= MAX_CLIENT_CONNECTIONS {
                            append_log(&state, "WARN", &format!("连接数已达到上限，拒绝设备: {peer}"));
                            continue;
                        }
                        let app_handle = app.clone();
                        let state_for_client = state.clone();
                        let server_ip = server_ip.clone();
                        clients.spawn(async move {
                            handle_client(app_handle, state_for_client, peer, stream, server_ip).await;
                        });
                    }
                    Err(error) => {
                        return Err(error).context("接受设备连接失败");
                    }
                }
            }
        }
    }
}

fn active_listener_port(state: &AppState) -> Option<u16> {
    let mut listener = state.listener.lock().unwrap();
    if listener
        .as_ref()
        .is_some_and(|handle| handle.join.is_finished())
    {
        listener.take();
    }
    listener.as_ref().map(|handle| handle.port)
}

fn listener_status(state: &AppState) -> ListenerStatus {
    let config = state.config.lock().unwrap().clone();
    let active_port = active_listener_port(state);
    ListenerStatus {
        running: active_port.is_some(),
        port: active_port.unwrap_or(config.local.port),
        device_count: state.devices.lock().unwrap().len(),
        active_connections: state.active_connections.load(Ordering::Relaxed),
        last_error: state.last_error.lock().unwrap().clone(),
        log_path: state
            .log_path
            .lock()
            .unwrap()
            .as_ref()
            .map(|path| path.display().to_string()),
    }
}

fn validate_device_number(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 64
        || !value
            .chars()
            .all(|character| character.is_ascii_alphanumeric())
    {
        return Err(anyhow!("设备编号只能包含 1 到 64 个数字或英文字母"));
    }
    Ok(())
}

#[tauri::command]
fn get_config<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
) -> Result<AppConfig, String> {
    let path = config_path(&app);
    if !path.exists() {
        let default_config = AppConfig::default();
        save_config_to_path(&path, &default_config)
            .map_err(|error| record_error(&state, error.to_string()))?;
    }
    let config =
        load_config_from_path(&path).map_err(|error| record_error(&state, error.to_string()))?;
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
    save_config_to_path(&path, &config).map_err(|error| record_error(&state, error.to_string()))?;
    *state.config.lock().unwrap() = config;
    clear_error(&state);
    append_log(&state, "INFO", &format!("配置已保存: {}", path.display()));
    Ok(())
}

#[tauri::command]
async fn refresh_devices(state: State<'_, AppState>) -> Result<Vec<DeviceInfo>, String> {
    match load_devices_into_state(&state).await {
        Ok(devices) => {
            clear_error(&state);
            append_log(
                &state,
                "INFO",
                &format!("设备列表已刷新，共 {} 台", devices.len()),
            );
            Ok(devices)
        }
        Err(error) => Err(record_error(&state, error.to_string())),
    }
}

#[tauri::command]
async fn start_listener<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
) -> Result<ListenerStatus, String> {
    let state_arc = state.inner().clone();
    if active_listener_port(&state_arc).is_some() {
        return Ok(listener_status(&state_arc));
    }
    state_arc
        .listener_starting
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .map_err(|_| "监听正在启动，请稍候".to_string())?;
    let _starting_guard = ListenerStartingGuard(&state_arc.listener_starting);

    let config = state.config.lock().unwrap().clone();
    let allowed_ips =
        validate_config(&config).map_err(|error| record_error(&state_arc, error.to_string()))?;
    let port = config.local.port;
    let listener = TcpListener::bind(("0.0.0.0", port))
        .await
        .with_context(|| format!("无法监听本地端口 {port}"))
        .map_err(|error| record_error(&state_arc, error.to_string()))?;

    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let app_handle = app.clone();
    let state_for_listener = state_arc.clone();
    let join = tokio::spawn(async move {
        if let Err(error) = run_listener(
            app_handle,
            state_for_listener.clone(),
            listener,
            allowed_ips,
            shutdown_rx,
        )
        .await
        {
            record_error(&state_for_listener, format!("监听服务异常停止: {error}"));
        }
    });

    *state_arc.listener.lock().unwrap() = Some(ListenerHandle {
        port,
        shutdown: shutdown_tx,
        join,
    });
    clear_error(&state_arc);
    append_log(&state_arc, "INFO", &format!("监听已启动: 0.0.0.0:{port}"));

    let state_for_devices = state_arc.clone();
    tauri::async_runtime::spawn(async move {
        match load_devices_into_state(&state_for_devices).await {
            Ok(devices) => append_log(
                &state_for_devices,
                "INFO",
                &format!("后台刷新设备列表成功，共 {} 台", devices.len()),
            ),
            Err(error) => {
                record_error(&state_for_devices, format!("后台刷新设备列表失败: {error}"));
            }
        }
    });
    Ok(listener_status(&state_arc))
}

#[tauri::command]
async fn stop_listener(state: State<'_, AppState>) -> Result<ListenerStatus, String> {
    let state_arc = state.inner().clone();
    stop_listener_inner(&state_arc).await?;
    Ok(listener_status(&state_arc))
}

async fn stop_listener_inner(state: &AppState) -> Result<(), String> {
    let handle = state.listener.lock().unwrap().take();
    if let Some(handle) = handle {
        let _ = handle.shutdown.send(());
        if let Err(error) = handle.join.await {
            return Err(record_error(state, format!("停止监听失败: {error}")));
        }
    }
    append_log(state, "INFO", "监听已停止，现有设备连接已关闭");
    Ok(())
}

#[tauri::command]
fn get_listener_status(state: State<'_, AppState>) -> ListenerStatus {
    listener_status(&state)
}

#[tauri::command]
fn get_device_info(state: State<'_, AppState>, enumber: String) -> Option<String> {
    state.devices.lock().unwrap().get(&enumber).cloned()
}

#[tauri::command]
async fn fetch_last_record(state: State<'_, AppState>, enumber: String) -> Result<Value, String> {
    validate_device_number(&enumber).map_err(|error| error.to_string())?;
    api_get(&state, &format!("lastrecordbyeid/{enumber}"))
        .await
        .map_err(|error| record_error(&state, error.to_string()))
}

#[tauri::command]
async fn fetch_patient_names(state: State<'_, AppState>) -> Result<Vec<PatientOption>, String> {
    let identifier = state.config.lock().unwrap().local.identifier.clone();
    let value = api_post(
        &state,
        "getPatientNameList",
        json!({ "identifier": identifier }),
    )
    .await
    .map_err(|error| record_error(&state, error.to_string()))?;
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
    let verify_binding = payload.bindmirrostate == "1";
    let expected_number = payload.number.clone();
    let expected_patient = payload.namepatient.clone();
    let sound_file = match payload.bindmirrostate.as_str() {
        "0" => "bdjc.wav",
        "1" => "bdcg.wav",
        _ => "ding.wav",
    };
    let value = api_post(&state, "writeback2", payload)
        .await
        .map_err(|error| record_error(&state, error.to_string()))?;

    if value
        .get("success")
        .and_then(Value::as_bool)
        .is_some_and(|success| success)
    {
        if verify_binding {
            let verification = api_get(&state, &format!("data/v1/recordByNo/{expected_number}"))
                .await
                .map_err(|error| {
                    record_error(
                        &state,
                        format!("绑定接口成功，但服务端状态复核请求失败: {error}"),
                    )
                })?;
            verify_binding_state(&verification, &expected_number, &expected_patient).map_err(
                |error| {
                    record_error(
                        &state,
                        format!("绑定接口成功，但服务端状态复核失败: {error}"),
                    )
                },
            )?;
        }
        clear_error(&state);
        play_sound(config_path(&app), sound_file);
    } else if let Some(message) = value.get("msg").and_then(Value::as_str) {
        record_error(&state, format!("绑定接口返回失败: {message}"));
    }

    Ok(value)
}

#[tauri::command]
async fn manual_read<R: Runtime>(app: AppHandle<R>, command: String) -> Result<(), String> {
    let command = command.trim().to_string();
    validate_device_number(&command).map_err(|error| error.to_string())?;
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
    let path = config_path
        .parent()
        .map(|path| path.join(file_name))
        .filter(|path| path.exists());

    std::thread::spawn(move || {
        use rodio::Source as _;

        let Ok(stream) = rodio::OutputStreamBuilder::open_default_stream() else {
            return;
        };
        let sink = if let Some(path) = path {
            let Ok(file) = File::open(path) else {
                return;
            };
            let Ok(sink) = rodio::play(stream.mixer(), BufReader::new(file)) else {
                return;
            };
            sink
        } else {
            let frequency = match file_name {
                "bdcg.wav" => 880.0,
                "bdjc.wav" => 440.0,
                _ => 660.0,
            };
            let sink = rodio::Sink::connect_new(stream.mixer());
            sink.append(
                rodio::source::SineWave::new(frequency)
                    .take_duration(Duration::from_millis(180))
                    .amplify(0.18),
            );
            sink
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
                show_main_window(tray.app_handle());
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
            let quit_app = app.clone();
            let state = quit_app.state::<AppState>().inner().clone();
            tauri::async_runtime::spawn(async move {
                let _ = stop_listener_inner(&state).await;
                append_log(&state, "INFO", "AntListener 2026 已退出");
                quit_app.exit(0);
            });
        }
        _ => {}
    });

    Ok(())
}

pub fn run() {
    let http = reqwest::Client::builder()
        .connect_timeout(HTTP_CONNECT_TIMEOUT)
        .timeout(HTTP_REQUEST_TIMEOUT)
        .pool_idle_timeout(Duration::from_secs(60))
        .build()
        .expect("failed to create HTTP client");

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_main_window(app);
        }))
        .manage(Arc::new(AppStateInner {
            config: Mutex::new(AppConfig::default()),
            devices: Mutex::new(HashMap::new()),
            listener: Mutex::new(None),
            listener_starting: AtomicBool::new(false),
            active_connections: Arc::new(AtomicUsize::new(0)),
            last_error: Mutex::new(None),
            log_path: Mutex::new(None),
            http,
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
            let state = app.state::<AppState>();
            let log_path = app
                .path()
                .app_log_dir()
                .ok()
                .map(|directory| directory.join("ant-listener.log"));
            *state.log_path.lock().unwrap() = log_path;

            let mut config_valid = true;
            let config = if path.exists() {
                match load_config_from_path(&path) {
                    Ok(config) => config,
                    Err(error) => {
                        config_valid = false;
                        record_error(&state, error.to_string());
                        AppConfig::default()
                    }
                }
            } else {
                let config = AppConfig::default();
                if let Err(error) = save_config_to_path(&path, &config) {
                    config_valid = false;
                    record_error(&state, format!("创建默认配置失败: {error}"));
                }
                config
            };
            *state.config.lock().unwrap() = config.clone();
            build_tray(app)?;
            append_log(&state, "INFO", "AntListener 2026 已启动");
            if config_valid && config.server.autorun {
                let app_handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    let app_for_state = app_handle.clone();
                    let state = app_for_state.state::<AppState>();
                    if let Err(error) = start_listener(app_handle, state).await {
                        let state = app_for_state.state::<AppState>();
                        record_error(&state, format!("自动启动监听失败: {error}"));
                    }
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
    use super::{parse_command, validate_config, verify_binding_state, AppConfig, PacketDecoder};
    use serde_json::json;

    #[test]
    fn binding_verification_requires_matching_bound_record() {
        let response = json!({
            "success": true,
            "ant": {"number": "ANT001", "patient": "张三"}
        });
        verify_binding_state(&response, "ANT001", "张三").unwrap();
    }

    #[test]
    fn binding_verification_rejects_mismatched_record() {
        let response = json!({
            "success": true,
            "ant": {"number": "ANT000", "patient": "李四"}
        });
        assert!(verify_binding_state(&response, "ANT001", "张三").is_err());
    }

    #[test]
    fn parses_legacy_ten_byte_card_number() {
        let packet = [0x00, 0x00, 0x00, 0x08, 0x04, 0x00, 0x02, 0xCB, 0x7A, 0xEE];
        assert_eq!(parse_command(&packet).unwrap(), "0046889710");
    }

    #[test]
    fn parses_five_byte_packet_as_lowercase_hex() {
        let packet = [0x12, 0xAB, 0x00, 0x7F, 0x09];
        assert_eq!(parse_command(&packet).unwrap(), "12ab007f09");
    }

    #[test]
    fn rejects_incomplete_packet() {
        let packet = [0x12, 0xAB, 0x00];
        assert!(parse_command(&packet).is_err());
    }

    #[test]
    fn decoder_reassembles_fragmented_ten_byte_packet() {
        let mut decoder = PacketDecoder::default();
        assert!(decoder.push(&[0x00, 0x00, 0x00, 0x08]).is_empty());
        let packets = decoder.push(&[0x04, 0x00, 0x02, 0xCB, 0x7A, 0xEE]);
        assert_eq!(packets.len(), 1);
        assert_eq!(parse_command(&packets[0]).unwrap(), "0046889710");
        assert_eq!(decoder.pending_len(), 0);
    }

    #[test]
    fn decoder_splits_coalesced_five_byte_packets() {
        let mut decoder = PacketDecoder::default();
        let packets = decoder.push(&[0x12, 0xAB, 0x00, 0x7F, 0x09, 0x34, 0xCD, 0x01, 0x02, 0x03]);
        assert_eq!(packets.len(), 2);
        assert_eq!(parse_command(&packets[0]).unwrap(), "12ab007f09");
        assert_eq!(parse_command(&packets[1]).unwrap(), "34cd010203");
    }

    #[test]
    fn decoder_handles_mixed_packet_types() {
        let mut decoder = PacketDecoder::default();
        let packets = decoder.push(&[
            0x12, 0xAB, 0x00, 0x7F, 0x09, 0x00, 0x00, 0x00, 0x08, 0x04, 0x00, 0x02, 0xCB, 0x7A,
            0xEE,
        ]);
        assert_eq!(packets.len(), 2);
        assert_eq!(parse_command(&packets[0]).unwrap(), "12ab007f09");
        assert_eq!(parse_command(&packets[1]).unwrap(), "0046889710");
    }

    #[test]
    fn config_rejects_zero_port_and_invalid_allowed_ip() {
        let mut config = AppConfig::default();
        config.local.port = 0;
        assert!(validate_config(&config).is_err());

        config.local.port = 9000;
        config.local.allowed_ips = vec!["not-an-ip".to_string()];
        assert!(validate_config(&config).is_err());
    }

    #[test]
    fn example_config_is_valid() {
        let config: AppConfig = toml::from_str(include_str!("../../config.example.toml")).unwrap();
        validate_config(&config).unwrap();
    }

    #[tokio::test]
    async fn binding_an_occupied_port_fails() {
        let first = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let address = first.local_addr().unwrap();
        assert!(tokio::net::TcpListener::bind(address).await.is_err());
    }
}
