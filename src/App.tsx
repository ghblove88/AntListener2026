import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  Ban,
  Check,
  ChevronLeft,
  ChevronRight,
  ChevronsLeft,
  ChevronsRight,
  CircleAlert,
  Cog,
  FileText,
  ListRestart,
  Loader2,
  MonitorCog,
  Play,
  Power,
  RadioTower,
  RefreshCw,
  Search,
  Server,
  ShieldCheck,
  Square,
  Trash2,
  X
} from "lucide-react";
import { pinyin } from "pinyin-pro";
import { type ReactNode, useEffect, useMemo, useRef, useState } from "react";

type AppConfig = {
  server: {
    url: string;
    autorun: boolean;
  };
  local: {
    port: number;
    identifier: string;
    allowed_ips: string[];
  };
};

type ListenerStatus = {
  running: boolean;
  port: number;
  device_count: number;
  active_connections: number;
  last_error: string | null;
  log_path: string | null;
};

type DeviceInfo = {
  number: string;
  info: string;
};

type IncomingCommand = {
  server_ip: string;
  client_ip: string;
  command: string;
};

type BindRecord = {
  id: string;
  time: string;
  enumber: string;
  einfo: string;
  number: string;
  operator: string;
  beginTime: string;
  totalTime: string;
  patientName: string;
};

type PatientOption = {
  patient_name: string;
  patient_id: string;
  appointment_time: string;
  treatment_room: string;
  waiting_order: string;
  check_item: string;
  display: string;
};

type AntRecord = {
  success?: boolean;
  msg?: string;
  ant?: {
    Number?: string;
    Operator?: string;
    BeginTime?: string;
    EndTime?: string;
    EndoscopeInfo?: string;
    EndoscopeNumber?: string;
    EndoscopeType?: string;
    PatientName?: string;
    TotalCostTime?: number;
  };
  step?: Array<{
    Step?: string;
    CostTime?: number;
    WashingMachine?: string;
  }>;
};

type BindDialogState = {
  incoming: IncomingCommand;
  deviceInfo: string;
  record: AntRecord | null;
  patients: PatientOption[];
  patientName: string;
  loading: boolean;
  saving: boolean;
  error: string;
};

const defaultConfig: AppConfig = {
  server: {
    url: "http://127.0.0.1:8866/",
    autorun: true
  },
  local: {
    port: 9000,
    identifier: "A1",
    allowed_ips: []
  }
};

const RECORD_STORAGE_KEY = "ant-listener-2026.records";
const MAX_LOCAL_RECORDS = 200;
const RECORD_PAGE_SIZE = 20;

function secToHms(value?: number): string {
  const duration = Number(value || 0);
  const hours = Math.floor(duration / 3600);
  const minutes = Math.floor((duration % 3600) / 60);
  const seconds = duration % 60;
  return [hours, minutes, seconds].map((part) => String(part).padStart(2, "0")).join(":");
}

function getInitials(name: string): string {
  try {
    return pinyin(name, { pattern: "first", toneType: "none" }).replace(/\s/g, "").toLowerCase();
  } catch {
    return name.toLowerCase();
  }
}

function filterPatients(patients: PatientOption[], keyword: string): PatientOption[] {
  const text = keyword.trim().toLowerCase();
  if (!text) return patients;
  const searchable = (patient: PatientOption) =>
    [patient.patient_name, patient.patient_id, patient.check_item, patient.display]
      .filter(Boolean)
      .join(" ")
      .toLowerCase();
  if (/^[a-z]+$/.test(text)) {
    return patients.filter((patient) => getInitials(patient.patient_name).startsWith(text) || searchable(patient).includes(text));
  }
  if (/[\u4e00-\u9fa5]/.test(text)) {
    return patients.filter((patient) => patient.patient_name.startsWith(text) || searchable(patient).includes(text));
  }
  return patients.filter((patient) => searchable(patient).includes(text));
}

function loadStoredRecords(): BindRecord[] {
  try {
    const value = JSON.parse(localStorage.getItem(RECORD_STORAGE_KEY) || "[]");
    return Array.isArray(value) ? value.slice(0, MAX_LOCAL_RECORDS) : [];
  } catch {
    return [];
  }
}

function App() {
  const [config, setConfig] = useState<AppConfig>(defaultConfig);
  const [status, setStatus] = useState<ListenerStatus>({
    running: false,
    port: 9000,
    device_count: 0,
    active_connections: 0,
    last_error: null,
    log_path: null
  });
  const [records, setRecords] = useState<BindRecord[]>(loadStoredRecords);
  const [recordKeyword, setRecordKeyword] = useState("");
  const [recordPage, setRecordPage] = useState(1);
  const [selectedId, setSelectedId] = useState("");
  const [manualNumber, setManualNumber] = useState("");
  const [message, setMessage] = useState("");
  const [busy, setBusy] = useState(false);
  const [showConfig, setShowConfig] = useState(false);
  const [queue, setQueue] = useState<IncomingCommand[]>([]);
  const [dialog, setDialog] = useState<BindDialogState | null>(null);
  const pendingCommands = useRef(new Set<string>());
  const dialogRequestId = useRef(0);

  async function refreshStatus() {
    const next = await invoke<ListenerStatus>("get_listener_status");
    setStatus(next);
  }

  async function loadConfig() {
    const next = await invoke<AppConfig>("get_config");
    setConfig(next);
    setStatus((current) => ({ ...current, port: next.local.port }));
  }

  async function refreshDevices() {
    setBusy(true);
    setMessage("");
    try {
      const list = await invoke<DeviceInfo[]>("refresh_devices");
      await refreshStatus();
      setMessage(`已刷新 ${list.length} 台内镜设备。`);
    } catch (error) {
      setMessage(String(error));
    } finally {
      setBusy(false);
    }
  }

  async function startListener() {
    setBusy(true);
    setMessage("");
    try {
      const next = await invoke<ListenerStatus>("start_listener");
      setStatus(next);
      setMessage(`监听已启动：0.0.0.0:${next.port}`);
    } catch (error) {
      setMessage(String(error));
    } finally {
      setBusy(false);
    }
  }

  async function stopListener() {
    setBusy(true);
    setMessage("");
    try {
      const next = await invoke<ListenerStatus>("stop_listener");
      setStatus(next);
      setMessage("监听已停止。");
    } catch (error) {
      setMessage(String(error));
    } finally {
      setBusy(false);
    }
  }

  async function saveConfig(next: AppConfig) {
    setBusy(true);
    setMessage("");
    let saved = false;
    try {
      await invoke("save_config", { config: next });
      saved = true;
      const listenerNeedsRestart =
        status.running &&
        (config.local.port !== next.local.port ||
          config.local.allowed_ips.join(",") !== next.local.allowed_ips.join(","));
      setConfig(next);
      setShowConfig(false);
      if (listenerNeedsRestart) {
        await invoke("stop_listener");
        await invoke("start_listener");
      }
      await refreshStatus();
      setMessage(listenerNeedsRestart ? "配置已保存，监听已按新配置重新启动。" : "配置已保存。");
    } catch (error) {
      await refreshStatus().catch(() => undefined);
      setMessage(saved ? `配置已保存，但重新启动监听失败：${String(error)}` : String(error));
    } finally {
      setBusy(false);
    }
  }

  async function triggerManualRead() {
    const command = manualNumber.trim();
    if (!command) {
      setMessage("请输入正确的设备编号。");
      return;
    }
    try {
      await invoke("manual_read", { command });
      setManualNumber("");
    } catch (error) {
      setMessage(String(error));
    }
  }

  async function unbindSelected() {
    const record = records.find((item) => item.id === selectedId);
    if (!record) {
      setMessage("请先选择一条要解除绑定的记录。");
      return;
    }
    setBusy(true);
    setMessage("");
    try {
      const result = await invoke<{ success?: boolean; msg?: string }>("bind_patient", {
        payload: {
          Number: record.number,
          Namepatient: record.patientName,
          Patientvisitid: "",
          Checkreportid: "",
          Rfidno: record.enumber,
          Bindmirrostate: "0"
        }
      });
      if (result.success !== true) {
        setMessage(`解除绑定失败：${result.msg || "接口未返回成功状态"}`);
        return;
      }
      setRecords((current) => current.filter((item) => item.id !== record.id));
      setSelectedId("");
      setMessage("已解除绑定。");
    } catch (error) {
      setMessage(String(error));
    } finally {
      setBusy(false);
    }
  }

  function enqueueIncoming(incoming: IncomingCommand) {
    if (pendingCommands.current.has(incoming.command)) return;
    pendingCommands.current.add(incoming.command);
    setQueue((current) => {
      if (current.length >= 100) {
        pendingCommands.current.delete(incoming.command);
        setMessage("待读取队列已达到 100 条，已拒绝新的刷卡数据。请先处理当前队列。");
        return current;
      }
      return [...current, incoming];
    });
  }

  async function openBindDialog(incoming: IncomingCommand) {
    const requestId = ++dialogRequestId.current;
    setDialog({
      incoming,
      deviceInfo: "",
      record: null,
      patients: [],
      patientName: "",
      loading: true,
      saving: false,
      error: ""
    });
    const [deviceResult, recordResult, patientResult] = await Promise.allSettled([
        invoke<string | null>("get_device_info", { enumber: incoming.command }),
        invoke<AntRecord>("fetch_last_record", { enumber: incoming.command }),
        invoke<PatientOption[]>("fetch_patient_names")
      ] as const);
    try {
      if (recordResult.status === "rejected") throw recordResult.reason;
      const record = recordResult.value;
      if (record.success === false) {
        throw new Error(record.msg || `未找到内窥镜 ${incoming.command} 的洗消记录。`);
      }
      if (!record.ant?.Number) {
        throw new Error(`内窥镜 ${incoming.command} 的洗消记录缺少洗消编号。`);
      }
      const deviceInfo = deviceResult.status === "fulfilled" ? deviceResult.value : null;
      const patients = patientResult.status === "fulfilled" ? patientResult.value : [];
      const patientError =
        patientResult.status === "rejected" ? `候诊病人列表读取失败，仍可手动输入姓名：${String(patientResult.reason)}` : "";
      setDialog((current) =>
        current && dialogRequestId.current === requestId
          ? {
              ...current,
              deviceInfo: deviceInfo || "",
              record,
              patients,
              patientName: patients[0]?.patient_name || "",
              loading: false,
              error: patientError
            }
          : current
      );
    } catch (error) {
      setDialog((current) =>
        current && dialogRequestId.current === requestId
          ? {
              ...current,
              loading: false,
              error: String(error)
            }
          : current
      );
    }
  }

  function closeBindDialog() {
    dialogRequestId.current += 1;
    if (dialog) pendingCommands.current.delete(dialog.incoming.command);
    setDialog(null);
  }

  async function confirmBind(patientName: string) {
    if (!dialog?.record?.ant?.Number || dialog.saving) return;
    if (!patientName.trim()) {
      setDialog((current) => (current ? { ...current, error: "请输入病人名字，与当前洗消记录绑定。" } : current));
      return;
    }
    setDialog((current) => (current ? { ...current, saving: true, error: "" } : current));
    try {
      const result = await invoke<{ success?: boolean; msg?: string }>("bind_patient", {
        payload: {
          Number: dialog.record.ant.Number,
          Namepatient: patientName.trim(),
          Patientvisitid: "",
          Checkreportid: "",
          Rfidno: dialog.incoming.command,
          Bindmirrostate: "1"
        }
      });
      if (result.success !== true) {
        throw new Error(result.msg || "接口未返回成功状态");
      }
      const nextRecord: BindRecord = {
        id: `${Date.now()}-${dialog.incoming.command}`,
        time: new Date().toLocaleString("zh-CN", { hour12: false }),
        enumber: dialog.incoming.command,
        einfo: dialog.deviceInfo,
        number: dialog.record.ant.Number || "",
        operator: dialog.record.ant.Operator || "",
        beginTime: dialog.record.ant.BeginTime || "",
        totalTime: secToHms(dialog.record.ant.TotalCostTime),
        patientName: patientName.trim()
      };
      setRecords((current) => [nextRecord, ...current].slice(0, MAX_LOCAL_RECORDS));
      setRecordKeyword("");
      setRecordPage(1);
      setSelectedId(nextRecord.id);
      closeBindDialog();
    } catch (error) {
      setDialog((current) =>
        current
          ? {
              ...current,
              saving: false,
              error: `病人姓名绑定操作错误：${String(error)}`
            }
          : current
      );
    }
  }

  useEffect(() => {
    loadConfig().then(refreshStatus).catch((error) => setMessage(String(error)));
    const statusTimer = window.setInterval(() => {
      refreshStatus().catch(() => undefined);
    }, 2000);
    let disposed = false;
    const unlisteners: Array<() => void> = [];

    function register(unlistenPromise: Promise<() => void>) {
      unlistenPromise.then((unlisten) => {
        if (disposed) unlisten();
        else unlisteners.push(unlisten);
      });
    }

    register(listen<IncomingCommand>("ant://incoming-command", (event) => enqueueIncoming(event.payload)));
    register(listen("ant://open-config", () => setShowConfig(true)));
    register(listen("ant://tray-start", () => startListener()));
    register(listen("ant://tray-stop", () => stopListener()));

    return () => {
      disposed = true;
      window.clearInterval(statusTimer);
      unlisteners.forEach((unlisten) => unlisten());
    };
  }, []);

  useEffect(() => {
    try {
      localStorage.setItem(RECORD_STORAGE_KEY, JSON.stringify(records.slice(0, MAX_LOCAL_RECORDS)));
    } catch (error) {
      setMessage(`本机绑定记录保存失败：${String(error)}`);
    }
  }, [records]);

  useEffect(() => {
    if (!dialog && queue.length > 0) {
      const [next, ...rest] = queue;
      setQueue(rest);
      openBindDialog(next);
    }
  }, [dialog, queue]);

  const filteredRecords = useMemo(() => {
    const keyword = recordKeyword.trim().toLowerCase();
    if (!keyword) return records;
    return records.filter((record) =>
      [
        record.time,
        record.enumber,
        record.einfo,
        record.number,
        record.operator,
        record.beginTime,
        record.patientName
      ]
        .join(" ")
        .toLowerCase()
        .includes(keyword)
    );
  }, [recordKeyword, records]);

  const recordTotalPages = Math.max(1, Math.ceil(filteredRecords.length / RECORD_PAGE_SIZE));
  const safeRecordPage = Math.min(recordPage, recordTotalPages);
  const pageRecords = filteredRecords.slice(
    (safeRecordPage - 1) * RECORD_PAGE_SIZE,
    safeRecordPage * RECORD_PAGE_SIZE
  );

  useEffect(() => {
    if (recordPage > recordTotalPages) {
      setRecordPage(recordTotalPages);
      setSelectedId("");
    }
  }, [recordPage, recordTotalPages]);

  function goToRecordPage(page: number) {
    const nextPage = Math.max(1, Math.min(page, recordTotalPages));
    setRecordPage(nextPage);
    setSelectedId("");
  }

  const selectedRecord = records.find((item) => item.id === selectedId);
  const visibleMessage = status.last_error || message || "";

  return (
    <main className="app-shell">
      <header className="topbar">
        <div className="brand-lockup">
          <span className="brand-mark" aria-hidden="true">
            <RadioTower size={21} />
          </span>
          <h1>AntListener 2026</h1>
        </div>
        <div className="status-strip">
          <StatusPill active={status.running} label={status.running ? "监听中" : "已停止"} />
          <span className="metric-pill metric-port">
            端口 <strong>{status.port}</strong>
          </span>
          <span className="metric-pill metric-device">
            设备 <strong>{status.device_count}</strong>
          </span>
          <span className="metric-pill metric-connection">
            连接 <strong>{status.active_connections}</strong>
          </span>
          {queue.length > 0 && (
            <span className="metric-pill metric-queue">
              队列 <strong>{queue.length}</strong>
            </span>
          )}
        </div>
      </header>

      <section className="toolbar">
        <button className="primary" onClick={startListener} disabled={busy || status.running}>
          <Play size={17} /> 启动监听
        </button>
        <button onClick={stopListener} disabled={busy || !status.running}>
          <Square size={17} /> 停止监听
        </button>
        <button onClick={refreshDevices} disabled={busy}>
          {busy ? <Loader2 className="spin" size={17} /> : <RefreshCw size={17} />} 刷新设备
        </button>
        <button className="config-trigger" onClick={() => setShowConfig(true)}>
          <Cog size={17} /> 配置
        </button>
        <div className="manual-read">
          <input
            value={manualNumber}
            onChange={(event) => setManualNumber(event.target.value)}
            placeholder="手动输入设备编号"
            onKeyDown={(event) => {
              if (event.key === "Enter") triggerManualRead();
            }}
          />
          <button onClick={triggerManualRead}>
            <ListRestart size={17} /> 手动读取
          </button>
        </div>
      </section>

      {visibleMessage && (
        <div className="message">
          <CircleAlert size={18} />
          <span>{visibleMessage}</span>
        </div>
      )}

      <section className="content-grid">
        <div className="table-panel">
          <div className="panel-header">
            <div className="panel-heading">
              <span className="panel-heading-icon" aria-hidden="true">
                <ListRestart size={18} />
              </span>
              <div>
                <h2>本机绑定记录</h2>
                <small>仅保留最近 {MAX_LOCAL_RECORDS} 条，可手动清空</small>
              </div>
            </div>
            <div className="panel-actions">
              <button
                onClick={() => {
                  if (!window.confirm("仅清空本机保存的绑定记录，不会解除服务器上的绑定。确定继续吗？")) return;
                  setRecords([]);
                  setRecordKeyword("");
                  setRecordPage(1);
                  setSelectedId("");
                }}
                disabled={records.length === 0 || busy}
              >
                <Trash2 size={17} /> 清空记录
              </button>
              <button className="danger" onClick={unbindSelected} disabled={!selectedRecord || busy}>
                <Ban size={17} /> 解除绑定
              </button>
            </div>
          </div>
          <div className="record-filter-bar">
            <div className="record-search">
              <Search size={16} aria-hidden="true" />
              <input
                value={recordKeyword}
                aria-label="搜索本机绑定记录"
                placeholder="搜索内镜、病人、洗消编号或操作员"
                onChange={(event) => {
                  setRecordKeyword(event.target.value);
                  setRecordPage(1);
                  setSelectedId("");
                }}
              />
              {recordKeyword && (
                <button
                  className="record-search-clear"
                  aria-label="清除搜索条件"
                  title="清除搜索"
                  onClick={() => {
                    setRecordKeyword("");
                    setRecordPage(1);
                    setSelectedId("");
                  }}
                >
                  <X size={14} />
                </button>
              )}
            </div>
            <span className="record-filter-summary">
              {recordKeyword ? `筛选 ${filteredRecords.length} / ${records.length} 条` : `共 ${records.length} 条`}
            </span>
          </div>

          <div className={pageRecords.length === 0 ? "table-wrap is-empty" : "table-wrap"}>
            <table>
              <thead>
                <tr>
                  <th>读取时间</th>
                  <th>内镜编号</th>
                  <th>内镜信息</th>
                  <th>洗消编号</th>
                  <th>操作员</th>
                  <th>开始时间</th>
                  <th>总时长</th>
                  <th>病人姓名</th>
                </tr>
              </thead>
              <tbody>
                {pageRecords.length === 0 ? (
                  <tr>
                    <td className="empty" colSpan={8}>
                      <div className="empty-state">
                        <span aria-hidden="true">
                          <ListRestart size={22} />
                        </span>
                        <strong>{records.length === 0 ? "暂无绑定记录" : "未找到匹配记录"}</strong>
                        <small>
                          {records.length === 0 ? "启动监听后，刷卡数据会自动显示在这里" : "请调整或清除搜索条件"}
                        </small>
                      </div>
                    </td>
                  </tr>
                ) : (
                  pageRecords.map((record) => (
                    <tr
                      key={record.id}
                      className={record.id === selectedId ? "selected" : ""}
                      onClick={() => setSelectedId(record.id)}
                    >
                      <td>{record.time}</td>
                      <td>{record.enumber}</td>
                      <td>{record.einfo}</td>
                      <td>{record.number}</td>
                      <td>{record.operator}</td>
                      <td>{record.beginTime}</td>
                      <td>{record.totalTime}</td>
                      <td>{record.patientName}</td>
                    </tr>
                  ))
                )}
              </tbody>
            </table>
          </div>

          <div className="record-pagination">
            <span>
              共 <strong>{filteredRecords.length}</strong> 条，每页 {RECORD_PAGE_SIZE} 条
            </span>
            <div className="pagination-controls">
              <button
                className="page-icon-button"
                aria-label="第一页"
                title="第一页"
                disabled={safeRecordPage <= 1}
                onClick={() => goToRecordPage(1)}
              >
                <ChevronsLeft size={16} />
              </button>
              <button disabled={safeRecordPage <= 1} onClick={() => goToRecordPage(safeRecordPage - 1)}>
                <ChevronLeft size={16} /> 上一页
              </button>
              <span className="page-indicator">
                第 <strong>{safeRecordPage}</strong> / {recordTotalPages} 页
              </span>
              <button
                disabled={safeRecordPage >= recordTotalPages}
                onClick={() => goToRecordPage(safeRecordPage + 1)}
              >
                下一页 <ChevronRight size={16} />
              </button>
              <button
                className="page-icon-button"
                aria-label="最后一页"
                title="最后一页"
                disabled={safeRecordPage >= recordTotalPages}
                onClick={() => goToRecordPage(recordTotalPages)}
              >
                <ChevronsRight size={16} />
              </button>
            </div>
          </div>
        </div>

        <aside className="side-panel config-panel">
          <div className="config-panel-header">
            <span className="config-panel-mark" aria-hidden="true">
              <Cog size={19} />
            </span>
            <div>
              <h2>当前配置</h2>
              <p>运行参数与服务状态</p>
            </div>
            <span className={status.running ? "config-health online" : "config-health"}>
              {status.running ? "运行中" : "已停止"}
            </span>
          </div>

          <div className="config-body">
            <section className="config-group">
              <h3>服务连接</h3>
              <ConfigItem icon={<Server size={17} />} label="数据服务地址" value={config.server.url} tone="indigo" mono />
              <ConfigItem
                icon={<RadioTower size={17} />}
                label="本地监听端口"
                value={`${config.local.port}`}
                tone="teal"
                mono
              />
            </section>

            <section className="config-group">
              <h3>终端策略</h3>
              <ConfigItem icon={<MonitorCog size={17} />} label="本机编号" value={config.local.identifier} tone="blue" />
              <ConfigItem
                icon={<Power size={17} />}
                label="程序启动后"
                value={config.server.autorun ? "自动启动监听" : "保持监听关闭"}
                tone={config.server.autorun ? "emerald" : "slate"}
              />
              <ConfigItem
                icon={<ShieldCheck size={17} />}
                label="允许设备 IP"
                value={config.local.allowed_ips.length > 0 ? config.local.allowed_ips.join(", ") : "全部内网设备"}
                tone="amber"
                mono={config.local.allowed_ips.length > 0}
              />
            </section>

            <section className="config-group">
              <h3>运行诊断</h3>
              <ConfigItem
                icon={<FileText size={17} />}
                label="日志文件"
                value={status.log_path || "正在初始化"}
                tone="slate"
                mono
              />
            </section>
          </div>

          <div className="config-note">
            <CircleAlert size={16} />
            <span>关闭主窗口仅隐藏到任务栏托盘；完全退出请使用托盘菜单。</span>
          </div>
        </aside>
      </section>

      {showConfig && <ConfigDialog config={config} onCancel={() => setShowConfig(false)} onSave={saveConfig} />}
      {dialog && (
        <BindDialog
          state={dialog}
          onCancel={closeBindDialog}
          onConfirm={confirmBind}
          onPatientNameChange={(patientName) => setDialog((current) => (current ? { ...current, patientName } : current))}
        />
      )}
    </main>
  );
}

function StatusPill({ active, label }: { active: boolean; label: string }) {
  return (
    <span className={active ? "status active" : "status"}>
      <span />
      {label}
    </span>
  );
}

function ConfigItem({
  icon,
  label,
  value,
  tone,
  mono = false
}: {
  icon: ReactNode;
  label: string;
  value: string;
  tone: "teal" | "indigo" | "blue" | "emerald" | "amber" | "slate";
  mono?: boolean;
}) {
  return (
    <div className={`config-item tone-${tone}${mono ? " mono" : ""}`}>
      <span className="config-item-icon" aria-hidden="true">
        {icon}
      </span>
      <div>
        <span>{label}</span>
        <strong title={value}>{value}</strong>
      </div>
    </div>
  );
}

function ConfigDialog({
  config,
  onCancel,
  onSave
}: {
  config: AppConfig;
  onCancel: () => void;
  onSave: (config: AppConfig) => void;
}) {
  const [draft, setDraft] = useState<AppConfig>(config);
  const [allowedIps, setAllowedIps] = useState(config.local.allowed_ips.join(", "));

  return (
    <div className="modal-backdrop">
      <section className="modal config-modal">
        <div className="modal-header config-modal-header">
          <div className="modal-title-block">
            <span aria-hidden="true">
              <Cog size={20} />
            </span>
            <div>
              <h2>系统配置</h2>
              <p>配置数据服务、监听终端与设备访问策略</p>
            </div>
          </div>
          <button className="icon-button" onClick={onCancel}>
            <X size={18} />
          </button>
        </div>

        <div className="config-form">
          <section className="form-section">
            <div className="form-section-heading">
              <span className="tone-indigo" aria-hidden="true">
                <Server size={18} />
              </span>
              <div>
                <h3>数据服务</h3>
                <p>病人列表、洗消记录和绑定请求统一使用此地址</p>
              </div>
            </div>
            <label className="field-card">
              <span className="field-label">数据池服务地址</span>
              <input
                type="url"
                value={draft.server.url}
                placeholder="例如 http://127.0.0.1:8866/"
                onChange={(event) => setDraft({ ...draft, server: { ...draft.server, url: event.target.value } })}
              />
            </label>
          </section>

          <section className="form-section">
            <div className="form-section-heading">
              <span className="tone-teal" aria-hidden="true">
                <RadioTower size={18} />
              </span>
              <div>
                <h3>本机监听</h3>
                <p>设置读卡器接入端口和当前工作站编号</p>
              </div>
            </div>
            <div className="form-row config-field-row">
              <label className="field-card">
                <span className="field-label">本地监听端口</span>
                <input
                  type="number"
                  min={1}
                  max={65535}
                  value={draft.local.port}
                  onChange={(event) =>
                    setDraft({ ...draft, local: { ...draft.local, port: Number(event.target.value || 0) } })
                  }
                />
                <small>有效范围 1–65535</small>
              </label>
              <label className="field-card">
                <span className="field-label">本机编号</span>
                <input
                  value={draft.local.identifier}
                  placeholder="例如 A1"
                  onChange={(event) => setDraft({ ...draft, local: { ...draft.local, identifier: event.target.value } })}
                />
                <small>用于区分医院内不同监听工作站</small>
              </label>
            </div>

            <label className="toggle-card">
              <span className="toggle-icon" aria-hidden="true">
                <Power size={17} />
              </span>
              <span className="toggle-copy">
                <strong>启动程序后自动监听</strong>
                <small>开启后无需操作员再次点击“启动监听”</small>
              </span>
              <input
                className="toggle-input"
                type="checkbox"
                checked={draft.server.autorun}
                onChange={(event) => setDraft({ ...draft, server: { ...draft.server, autorun: event.target.checked } })}
              />
              <span className="toggle-visual" aria-hidden="true">
                <i />
              </span>
            </label>
          </section>

          <section className="form-section">
            <div className="form-section-heading">
              <span className="tone-amber" aria-hidden="true">
                <ShieldCheck size={18} />
              </span>
              <div>
                <h3>设备访问策略</h3>
                <p>限制哪些设备 IP 可以向本机发送刷卡数据</p>
              </div>
            </div>
            <label className="field-card">
              <span className="field-label">允许连接的设备 IP</span>
              <input
                value={allowedIps}
                placeholder="留空允许全部，例如 192.168.1.20, 192.168.1.21"
                onChange={(event) => setAllowedIps(event.target.value)}
              />
              <small>多个 IP 使用逗号或空格分隔；留空时允许全部内网设备。</small>
            </label>
          </section>
        </div>

        <div className="config-save-bar">
          <div className="config-restart-note">
            <RefreshCw size={15} />
            <span>监听运行时，修改端口或设备 IP 会自动重启监听。</span>
          </div>
          <div className="modal-actions config-actions">
            <button onClick={onCancel}>取消</button>
            <button
              className="primary"
              onClick={() =>
                onSave({
                  ...draft,
                  local: {
                    ...draft.local,
                    allowed_ips: allowedIps
                      .split(/[,，\s]+/)
                      .map((value) => value.trim())
                      .filter(Boolean)
                  }
                })
              }
            >
              <Check size={17} /> 保存配置
            </button>
          </div>
        </div>
      </section>
    </div>
  );
}

function BindDialog({
  state,
  onCancel,
  onConfirm,
  onPatientNameChange
}: {
  state: BindDialogState;
  onCancel: () => void;
  onConfirm: (patientName: string) => void;
  onPatientNameChange: (patientName: string) => void;
}) {
  const [filterText, setFilterText] = useState("");
  const filteredPatients = useMemo(() => filterPatients(state.patients, filterText), [state.patients, filterText]);
  const ant = state.record?.ant;
  const steps = state.record?.step || [];
  const totalStepTime = steps.reduce((sum, step) => sum + Number(step.CostTime || 0), 0);

  useEffect(() => {
    setFilterText("");
  }, [state.incoming.command]);

  return (
    <div className="modal-backdrop">
      <section className="modal bind-modal">
        <div className="modal-header">
          <h2>绑定病人姓名</h2>
          <button className="icon-button" onClick={onCancel} disabled={state.saving}>
            <X size={18} />
          </button>
        </div>

        <div className="bind-summary">
          <div>
            <span>内镜编号</span>
            <strong>{state.incoming.command}</strong>
          </div>
          <div>
            <span>内镜信息</span>
            <strong>{state.deviceInfo || "未加载"}</strong>
          </div>
        </div>

        {state.loading ? (
          <div className="loading-state">
            <Loader2 className="spin" size={24} />
            正在读取洗消记录与病人列表...
          </div>
        ) : state.error && !state.record ? (
          <div className="error-state">{state.error}</div>
        ) : (
          <div className="bind-workspace">
            <section className="bind-left">
              <div className="wash-card">
                <div className="wash-card-main">
                  <div>
                    <span>洗消编号</span>
                    <strong>{ant?.Number || "-"}</strong>
                  </div>
                  <div>
                    <span>内镜类型</span>
                    <strong>{ant?.EndoscopeType || state.deviceInfo.split(" ")[0] || "-"}</strong>
                  </div>
                  <div>
                    <span>操作员</span>
                    <strong>{ant?.Operator || "-"}</strong>
                  </div>
                  <div>
                    <span>总时长</span>
                    <strong>{secToHms(ant?.TotalCostTime)}</strong>
                  </div>
                </div>
                <div className="wash-time-range">
                  <span>{ant?.BeginTime || "-"}</span>
                  <span>{ant?.EndTime || "-"}</span>
                </div>
              </div>

              <div className="steps-section">
                <div className="section-title">
                  <h3>洗消步骤</h3>
                  <span>{steps.length} 个步骤 / 累计 {secToHms(totalStepTime)}</span>
                </div>
                <div className="step-timeline">
                  {steps.map((step, index) => {
                    const ratio = Math.max(6, Math.min(100, (Number(step.CostTime || 0) / Math.max(totalStepTime, 1)) * 100));
                    return (
                      <div className="step-card" key={`${step.Step}-${index}`}>
                        <div className="step-index">{String(index + 1).padStart(2, "0")}</div>
                        <div className="step-content">
                          <div className="step-head">
                            <strong>{step.Step || "未命名步骤"}</strong>
                            <span>{secToHms(step.CostTime)}</span>
                          </div>
                          <div className="step-machine">{step.WashingMachine || "人工处理 / 未记录设备"}</div>
                          <div className="step-bar">
                            <i style={{ width: `${ratio}%` }} />
                          </div>
                        </div>
                      </div>
                    );
                  })}
                </div>
              </div>
            </section>

            <section className="patient-section bind-right">
              <div className="section-title">
                <h3>绑定病人姓名</h3>
                <span>队列 {state.patients.length} 人</span>
              </div>
              <div className="patient-picker">
                <label className="patient-input-card">
                  <span>当前绑定姓名</span>
                  <input
                    autoFocus
                    value={state.patientName}
                    onChange={(event) => {
                      onPatientNameChange(event.target.value);
                      setFilterText(event.target.value);
                    }}
                    onKeyDown={(event) => {
                      if (event.key === "Enter") onConfirm(state.patientName);
                    }}
                  />
                  <small>可输入姓名、拼音首字母、病人号或检查项目过滤队列。</small>
                </label>
                <div className="patient-queue">
                  {filteredPatients.length === 0 ? (
                    <div className="empty-name-list">没有匹配的病人姓名</div>
                  ) : (
                    filteredPatients.map((patient, index) => (
                      <button
                        className={patient.patient_name === state.patientName ? "patient-row selected" : "patient-row"}
                        key={`${patient.patient_id}-${patient.display}-${index}`}
                        onClick={() => {
                          onPatientNameChange(patient.patient_name || patient.display);
                          setFilterText("");
                        }}
                      >
                        <span className="queue-no">{patient.waiting_order || index + 1}</span>
                        <span className="patient-main">
                          <strong>{patient.patient_name || patient.display}</strong>
                          <em>{patient.check_item || "未记录检查项目"}</em>
                        </span>
                        <span className="patient-meta">
                          <em>{patient.patient_id || "-"}</em>
                          <em>{patient.appointment_time || "-"}</em>
                        </span>
                      </button>
                    ))
                  )}
                </div>
              </div>
            </section>
            {state.error && <div className="inline-error">{state.error}</div>}
          </div>
        )}

        <div className="modal-actions">
          <button onClick={onCancel} disabled={state.saving}>取消</button>
          <button className="primary" onClick={() => onConfirm(state.patientName)} disabled={state.loading || state.saving}>
            {state.saving ? <Loader2 className="spin" size={17} /> : <Check size={17} />} 确定绑定
          </button>
        </div>
      </section>
    </div>
  );
}

export default App;
