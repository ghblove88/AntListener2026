import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  Ban,
  Check,
  CircleAlert,
  Cog,
  ListRestart,
  Loader2,
  Play,
  RefreshCw,
  Square,
  Trash2,
  X
} from "lucide-react";
import { pinyin } from "pinyin-pro";
import { useEffect, useMemo, useState } from "react";

type AppConfig = {
  server: {
    url: string;
    username: string;
    password: string;
    autorun: boolean;
  };
  local: {
    port: number;
    identifier: string;
  };
};

type ListenerStatus = {
  running: boolean;
  port: number;
  device_count: number;
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
    username: "admin",
    password: "password",
    autorun: true
  },
  local: {
    port: 9000,
    identifier: "A1"
  }
};

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

function App() {
  const [config, setConfig] = useState<AppConfig>(defaultConfig);
  const [status, setStatus] = useState<ListenerStatus>({ running: false, port: 9000, device_count: 0 });
  const [devices, setDevices] = useState<DeviceInfo[]>([]);
  const [records, setRecords] = useState<BindRecord[]>([]);
  const [selectedId, setSelectedId] = useState("");
  const [manualNumber, setManualNumber] = useState("");
  const [message, setMessage] = useState("");
  const [busy, setBusy] = useState(false);
  const [showConfig, setShowConfig] = useState(false);
  const [queue, setQueue] = useState<IncomingCommand[]>([]);
  const [dialog, setDialog] = useState<BindDialogState | null>(null);

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
      setDevices(list);
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
    try {
      await invoke("save_config", { config: next });
      setConfig(next);
      setShowConfig(false);
      await refreshStatus();
      setMessage("配置已保存。端口修改后请重新启动监听。");
    } catch (error) {
      setMessage(String(error));
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
    await invoke("manual_read", { command });
    setManualNumber("");
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
    setQueue((current) => {
      const exists =
        dialog?.incoming.command === incoming.command ||
        current.some((item) => item.command === incoming.command && item.client_ip === incoming.client_ip);
      return exists ? current : [...current, incoming];
    });
  }

  async function openBindDialog(incoming: IncomingCommand) {
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
    try {
      const [deviceInfo, record, patients] = await Promise.all([
        invoke<string | null>("get_device_info", { enumber: incoming.command }),
        invoke<AntRecord>("fetch_last_record", { enumber: incoming.command }),
        invoke<PatientOption[]>("fetch_patient_names")
      ]);
      if (record.success === false) {
        throw new Error(record.msg || `未找到内窥镜 ${incoming.command} 的洗消记录。`);
      }
      setDialog((current) =>
        current
          ? {
              ...current,
              deviceInfo: deviceInfo || "",
              record,
              patients,
              patientName: patients[0]?.patient_name || "",
              loading: false,
              error: ""
            }
          : current
      );
    } catch (error) {
      setDialog((current) =>
        current
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
    setDialog(null);
  }

  async function confirmBind(patientName: string) {
    if (!dialog?.record?.ant?.Number) return;
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
      setRecords((current) => [nextRecord, ...current]);
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
    const unlisteners: Array<() => void> = [];

    listen<IncomingCommand>("ant://incoming-command", (event) => enqueueIncoming(event.payload)).then((unlisten) =>
      unlisteners.push(unlisten)
    );
    listen("ant://open-config", () => setShowConfig(true)).then((unlisten) => unlisteners.push(unlisten));
    listen("ant://tray-start", () => startListener()).then((unlisten) => unlisteners.push(unlisten));
    listen("ant://tray-stop", () => stopListener()).then((unlisten) => unlisteners.push(unlisten));

    return () => {
      unlisteners.forEach((unlisten) => unlisten());
    };
  }, []);

  useEffect(() => {
    if (!dialog && queue.length > 0) {
      const [next, ...rest] = queue;
      setQueue(rest);
      openBindDialog(next);
    }
  }, [dialog, queue]);

  const selectedRecord = records.find((item) => item.id === selectedId);

  return (
    <main className="app-shell">
      <header className="topbar">
        <div>
          <h1>AntListener 2026</h1>
          <p>内镜洗消数据监听与病人绑定</p>
        </div>
        <div className="status-strip">
          <StatusPill active={status.running} label={status.running ? "监听中" : "已停止"} />
          <span>端口 {status.port}</span>
          <span>设备 {status.device_count || devices.length}</span>
          {queue.length > 0 && <span>队列 {queue.length}</span>}
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
        <button onClick={() => setShowConfig(true)}>
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

      {message && (
        <div className="message">
          <CircleAlert size={18} />
          <span>{message}</span>
        </div>
      )}

      <section className="content-grid">
        <div className="table-panel">
          <div className="panel-header">
            <h2>绑定记录</h2>
            <button className="danger" onClick={unbindSelected} disabled={!selectedRecord || busy}>
              <Ban size={17} /> 解除绑定
            </button>
          </div>
          <div className="table-wrap">
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
                {records.length === 0 ? (
                  <tr>
                    <td className="empty" colSpan={8}>
                      暂无绑定记录。启动监听后，刷卡数据会在这里显示。
                    </td>
                  </tr>
                ) : (
                  records.map((record) => (
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
        </div>

        <aside className="side-panel">
          <h2>当前配置</h2>
          <dl>
            <dt>服务地址</dt>
            <dd>{config.server.url}</dd>
            <dt>本地端口</dt>
            <dd>{config.local.port}</dd>
            <dt>本机编号</dt>
            <dd>{config.local.identifier}</dd>
            <dt>自动监听</dt>
            <dd>{config.server.autorun ? "开启" : "关闭"}</dd>
          </dl>
          <div className="note">关闭主窗口会隐藏到任务栏托盘。需要完全退出时，请使用托盘菜单中的退出。</div>
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

  return (
    <div className="modal-backdrop">
      <section className="modal config-modal">
        <div className="modal-header">
          <h2>系统配置</h2>
          <button className="icon-button" onClick={onCancel}>
            <X size={18} />
          </button>
        </div>
        <label>
          <span>数据池服务地址</span>
          <input
            value={draft.server.url}
            onChange={(event) => setDraft({ ...draft, server: { ...draft.server, url: event.target.value } })}
          />
        </label>
        <div className="form-row">
          <label>
            <span>本地监听端口</span>
            <input
              type="number"
              value={draft.local.port}
              onChange={(event) =>
                setDraft({ ...draft, local: { ...draft.local, port: Number(event.target.value || 0) } })
              }
            />
          </label>
          <label>
            <span>本机编号</span>
            <input
              value={draft.local.identifier}
              onChange={(event) => setDraft({ ...draft, local: { ...draft.local, identifier: event.target.value } })}
            />
          </label>
        </div>
        <div className="form-row">
          <label>
            <span>用户名</span>
            <input
              value={draft.server.username}
              onChange={(event) => setDraft({ ...draft, server: { ...draft.server, username: event.target.value } })}
            />
          </label>
          <label>
            <span>密码</span>
            <input
              type="password"
              value={draft.server.password}
              onChange={(event) => setDraft({ ...draft, server: { ...draft.server, password: event.target.value } })}
            />
          </label>
        </div>
        <label className="checkbox">
          <input
            type="checkbox"
            checked={draft.server.autorun}
            onChange={(event) => setDraft({ ...draft, server: { ...draft.server, autorun: event.target.checked } })}
          />
          <span>启动程序后自动监听</span>
        </label>
        <div className="modal-actions">
          <button onClick={onCancel}>取消</button>
          <button className="primary" onClick={() => onSave(draft)}>
            <Check size={17} /> 保存
          </button>
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
          <button className="icon-button" onClick={onCancel}>
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
          <button onClick={onCancel}>取消</button>
          <button className="primary" onClick={() => onConfirm(state.patientName)} disabled={state.loading || state.saving}>
            {state.saving ? <Loader2 className="spin" size={17} /> : <Check size={17} />} 确定绑定
          </button>
        </div>
      </section>
    </div>
  );
}

export default App;
