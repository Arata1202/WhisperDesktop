import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import "./App.css";

type MeetingSummary = {
  id: string;
  date: string;
  roomId: string;
  meetingTime: string;
};

type JobStatus = {
  state: string;
  completed: number;
  total: number;
  outputPath?: string;
  error?: string;
  log?: string;
};

type AppConfig = {
  minio: {
    url: string;
    region: string;
    accessKey: string;
    secretKey: string;
    bucket: string;
  };
  whisper: {
    binaryPath: string;
    ffmpegPath: string;
    modelPath: string;
    outputDir: string;
    includeTimestamps: boolean;
    includeSpeaker: boolean;
  };
};

const defaultConfig: AppConfig = {
  minio: {
    url: "",
    region: "",
    accessKey: "",
    secretKey: "",
    bucket: "",
  },
  whisper: {
    binaryPath: "",
    ffmpegPath: "",
    modelPath: "ggml-large-v3.bin",
    outputDir: "",
    includeTimestamps: false,
    includeSpeaker: true,
  },
};

function App() {
  const [activeTab, setActiveTab] = useState<"meetings" | "settings">(
    "meetings",
  );
  const [dates, setDates] = useState<string[]>([]);
  const [meetings, setMeetings] = useState<MeetingSummary[]>([]);
  const [selectedDate, setSelectedDate] = useState<string | null>(null);
  const [selectedRoom, setSelectedRoom] = useState<string | null>(null);
  const [selectedMeetingId, setSelectedMeetingId] = useState<string | null>(
    null,
  );
  const [jobId, setJobId] = useState<string | null>(null);
  const [jobStatus, setJobStatus] = useState<JobStatus | null>(null);
  const [datesLoading, setDatesLoading] = useState(false);
  const [meetingsLoading, setMeetingsLoading] = useState(false);
  const [transcribeLoading, setTranscribeLoading] = useState(false);
  const [config, setConfig] = useState<AppConfig>(defaultConfig);
  const [configLoaded, setConfigLoaded] = useState(false);
  const [minioCheckLoading, setMinioCheckLoading] = useState(false);
  const [minioCheckStatus, setMinioCheckStatus] = useState<
    "idle" | "ok" | "ng"
  >("ng");

  const pollingRef = useRef<number | null>(null);
  const saveInFlightRef = useRef(false);
  const pendingConfigRef = useRef<AppConfig | null>(null);
  const lastSavedRef = useRef<string>("");
  const logRef = useRef<HTMLPreElement | null>(null);

  const hasDates = dates.length > 0;
  const hasMeetings = meetings.length > 0;

  const rooms = useMemo(() => {
    if (!hasMeetings) return [];
    const roomSet = new Set(meetings.map((meeting) => meeting.roomId));
    return Array.from(roomSet).sort();
  }, [hasMeetings, meetings]);

  const filteredMeetings = useMemo(() => {
    if (!selectedRoom) return meetings;
    return meetings.filter((meeting) => meeting.roomId === selectedRoom);
  }, [meetings, selectedRoom]);

  const refreshDates = async () => {
    setDatesLoading(true);
    try {
      const result = await invoke<string[]>("list_dates");
      const sorted = [...result].sort().reverse();
      setDates(sorted);
      if (result.length > 0) {
        setSelectedDate(null);
      } else {
        setSelectedDate(null);
        setMeetings([]);
        setSelectedMeetingId(null);
      }
    } catch (err) {
      console.error(err);
    } finally {
      setDatesLoading(false);
    }
  };

  const refreshMeetings = async (dateValue?: string | null) => {
    const targetDate = dateValue ?? selectedDate;
    if (!targetDate) return;
    setMeetingsLoading(true);
    try {
      const result = await invoke<MeetingSummary[]>("list_meetings", {
        date: targetDate,
      });
      setMeetings(result);
      setSelectedRoom(null);
      setSelectedMeetingId(null);
    } catch (err) {
      console.error(err);
    } finally {
      setMeetingsLoading(false);
    }
  };

  const refreshAll = async () => {
    await refreshDates();
    if (selectedDate) {
      await refreshMeetings(selectedDate);
    }
  };

  const startTranscribe = async () => {
    if (!selectedMeetingId) return;
    setJobStatus({ state: "running", completed: 0, total: 0, log: "" });
    setTranscribeLoading(true);
    try {
      const createdJobId = await invoke<string>("start_transcribe", {
        meetingId: selectedMeetingId,
      });
      setJobId(createdJobId);
    } catch (err) {
      console.error(err);
    } finally {
      setTranscribeLoading(false);
    }
  };

  const mergeConfig = (incoming: AppConfig) => ({
    ...defaultConfig,
    ...incoming,
    minio: { ...defaultConfig.minio, ...incoming.minio },
    whisper: { ...defaultConfig.whisper, ...incoming.whisper },
  });

  const loadConfig = async () => {
    try {
      const result = await invoke<AppConfig>("get_config");
      const merged = mergeConfig(result);
      if (merged.whisper.modelPath.endsWith(".en.bin")) {
        merged.whisper.modelPath = merged.whisper.modelPath.replace(
          ".en.bin",
          ".bin",
        );
      }
      if (!merged.whisper.modelPath.trim()) {
        merged.whisper.modelPath = "ggml-large-v3.bin";
      }
      if (!merged.whisper.binaryPath.trim()) {
        const binaryPath = await invoke<string | null>(
          "get_default_whisper_binary",
        );
        if (binaryPath) {
          merged.whisper.binaryPath = binaryPath;
        }
      }
      if (!merged.whisper.ffmpegPath.trim()) {
        const ffmpegPath = await invoke<string | null>(
          "get_default_ffmpeg_binary",
        );
        if (ffmpegPath) {
          merged.whisper.ffmpegPath = ffmpegPath;
        }
      }
      if (!merged.whisper.outputDir.trim()) {
        const outputDir = await invoke<string>("get_default_output_dir");
        if (outputDir) {
          merged.whisper.outputDir = outputDir;
        }
      }
      lastSavedRef.current = JSON.stringify(merged);
      setConfig(merged);
    } catch (err) {
      console.error(err);
    } finally {
      setConfigLoaded(true);
    }
  };

  const saveConfig = async (targetConfig: AppConfig) => {
    try {
      await invoke("set_config", { config: targetConfig });
      lastSavedRef.current = JSON.stringify(targetConfig);
    } catch (err) {
      console.error(err);
    }
  };

  const queueSave = (targetConfig: AppConfig) => {
    if (saveInFlightRef.current) {
      pendingConfigRef.current = targetConfig;
      return;
    }
    saveInFlightRef.current = true;
    saveConfig(targetConfig)
      .catch(() => {})
      .finally(() => {
        saveInFlightRef.current = false;
        if (pendingConfigRef.current) {
          const pending = pendingConfigRef.current;
          pendingConfigRef.current = null;
          queueSave(pending);
        }
      });
  };

  const checkMinioConnection = async () => {
    setMinioCheckLoading(true);
    try {
      await invoke("check_minio");
      setMinioCheckStatus("ok");
    } catch (err) {
      console.error(err);
      setMinioCheckStatus("ng");
    } finally {
      setMinioCheckLoading(false);
    }
  };

  const stopPolling = () => {
    if (pollingRef.current !== null) {
      window.clearInterval(pollingRef.current);
      pollingRef.current = null;
    }
  };

  const startPolling = () => {
    stopPolling();
    if (!jobId) return;
    pollingRef.current = window.setInterval(async () => {
      try {
        const status = await invoke<JobStatus>("get_transcribe_status", {
          jobId,
        });
        setJobStatus(status);
        if (status.state !== "running" && status.state !== "downloading") {
          stopPolling();
        }
      } catch (err) {
        console.error(err);
        stopPolling();
      }
    }, 1500);
  };

  useEffect(() => {
    refreshDates();
    loadConfig();
    checkMinioConnection();
    return () => {
      stopPolling();
    };
  }, []);

  useEffect(() => {
    if (selectedDate) {
      refreshMeetings(selectedDate);
    } else {
      setMeetings([]);
      setSelectedRoom(null);
      setSelectedMeetingId(null);
    }
  }, [selectedDate]);

  useEffect(() => {
    if (!jobId) return;
    startPolling();
    return () => {
      stopPolling();
    };
  }, [jobId]);

  useEffect(() => {
    if (!logRef.current) return;
    logRef.current.scrollTop = logRef.current.scrollHeight;
  }, [jobStatus?.log]);

  useEffect(() => {
    if (!configLoaded) return;
    const serialized = JSON.stringify(config);
    if (serialized === lastSavedRef.current) return;
    queueSave(config);
  }, [config, configLoaded]);

  return (
    <main className="app-shell">
      <div className="row header">
        <h1>WhisperDesktop</h1>
        <button
          type="button"
          className={`status-indicator status-${minioCheckStatus} ${
            minioCheckLoading ? "is-loading" : ""
          }`}
          aria-live="polite"
          onClick={checkMinioConnection}
          disabled={minioCheckLoading}
          aria-label="Check connection"
          title="Check connection"
        >
          <span
            className={`status-dot status-${minioCheckStatus}`}
            aria-hidden="true"
          />
          <span className="status-text">
            {minioCheckStatus === "ok" ? "SUCCESS" : "ERROR"}
          </span>
          <span className="status-icon" aria-hidden="true">
            ↻
          </span>
        </button>
      </div>
      <div className="row header-controls">
        <div className="tabs">
          <button
            type="button"
            className={activeTab === "meetings" ? "tab active" : "tab"}
            onClick={() => setActiveTab("meetings")}
          >
            Meetings
          </button>
          <button
            type="button"
            className={activeTab === "settings" ? "tab active" : "tab"}
            onClick={() => setActiveTab("settings")}
          >
            Settings
          </button>
        </div>
        {activeTab === "meetings" ? (
          <button
            type="button"
            className="ghost"
            onClick={refreshAll}
            disabled={minioCheckStatus === "ng" || datesLoading || meetingsLoading}
          >
            Refresh
          </button>
        ) : null}
      </div>

      {activeTab === "meetings" ? (
        <>
          <div className="grid">
            <section className="section">
              <div className="row">
                <h2>Select meeting</h2>
              </div>
              <div className="form-stack">
                <label className="field">
                  <span>Choose Date</span>
                  <select
                    value={selectedDate ?? ""}
                    onChange={(event) =>
                      setSelectedDate(event.target.value || null)
                    }
                    disabled={
                      minioCheckStatus === "ng" || !hasDates || datesLoading
                    }
                  >
                    <option value="">Select...</option>
                    {dates.map((date) => (
                      <option key={date} value={date}>
                        {date}
                      </option>
                    ))}
                  </select>
                </label>
                <label className="field">
                  <span>Choose Room</span>
                  <select
                    value={selectedRoom ?? ""}
                    onChange={(event) => {
                      const value = event.target.value;
                      setSelectedRoom(value || null);
                      setSelectedMeetingId(null);
                    }}
                    disabled={
                      minioCheckStatus === "ng" ||
                      !selectedDate ||
                      !rooms.length ||
                      meetingsLoading
                    }
                  >
                    <option value="">Select...</option>
                    {rooms.map((room) => (
                      <option key={room} value={room}>
                        {room}
                      </option>
                    ))}
                  </select>
                </label>
                <label className="field">
                  <span>3. Choose Time</span>
                  <select
                    value={selectedMeetingId ?? ""}
                    onChange={(event) => {
                      const value = event.target.value;
                      if (!value) {
                        setSelectedMeetingId(null);
                        return;
                      }
                      setSelectedMeetingId(value);
                    }}
                    disabled={
                      minioCheckStatus === "ng" ||
                      !selectedRoom ||
                      !filteredMeetings.length ||
                      meetingsLoading
                    }
                  >
                    <option value="">Select...</option>
                    {filteredMeetings.map((meeting) => (
                      <option key={meeting.id} value={meeting.id}>
                        {meeting.meetingTime}
                      </option>
                    ))}
                  </select>
                </label>
              </div>
            </section>
          </div>

          <section className="section" style={{ marginTop: 20 }}>
            <div className="row">
              <h2>Settings</h2>
            </div>
            <label className="field">
              <span>Show timestamps</span>
              <select
                value={config.whisper.includeTimestamps ? "on" : "off"}
                onChange={(event) =>
                  setConfig((prev) => ({
                    ...prev,
                    whisper: {
                      ...prev.whisper,
                      includeTimestamps: event.target.value === "on",
                    },
                  }))
                }
              >
                <option value="on">Show</option>
                <option value="off">Hide</option>
              </select>
            </label>
            <label className="field field-spaced">
              <span>Show speakers</span>
              <select
                value={config.whisper.includeSpeaker ? "on" : "off"}
                onChange={(event) =>
                  setConfig((prev) => ({
                    ...prev,
                    whisper: {
                      ...prev.whisper,
                      includeSpeaker: event.target.value === "on",
                    },
                  }))
                }
              >
                <option value="on">Show</option>
                <option value="off">Hide</option>
              </select>
            </label>
          </section>
          <section className="section section-gap">
            <div className="row row-log">
              <h2>Log</h2>
            </div>
            <pre ref={logRef} className="mono output-log">
              {jobStatus?.log ?? ""}
            </pre>
          </section>

          <button
            type="button"
            className="ghost button-full section-gap"
            onClick={startTranscribe}
            disabled={!selectedMeetingId || transcribeLoading}
          >
            Start
          </button>

        </>
      ) : (
        <>
          <section className="section">
            <div className="row">
              <h2>MinIO</h2>
            </div>
            <div className="form-grid">
              <label className="field">
                <span className="required-label">
                  MINIO_URL <span className="required-mark">*</span>
                </span>
                <input
                  type="text"
                  value={config.minio.url}
                  onChange={(event) =>
                    setConfig((prev) => ({
                      ...prev,
                      minio: { ...prev.minio, url: event.target.value },
                    }))
                  }
                  placeholder="https://..."
                />
              </label>
              <label className="field">
                <span className="required-label">
                  MINIO_REGION <span className="required-mark">*</span>
                </span>
                <input
                  type="text"
                  value={config.minio.region}
                  onChange={(event) =>
                    setConfig((prev) => ({
                      ...prev,
                      minio: { ...prev.minio, region: event.target.value },
                    }))
                  }
                  placeholder="ap-northeast-1"
                />
              </label>
              <label className="field">
                <span className="required-label">
                  MINIO_ACCESS_KEY <span className="required-mark">*</span>
                </span>
                <input
                  type="password"
                  value={config.minio.accessKey}
                  onChange={(event) =>
                    setConfig((prev) => ({
                      ...prev,
                      minio: { ...prev.minio, accessKey: event.target.value },
                    }))
                  }
                  autoComplete="off"
                />
              </label>
              <label className="field">
                <span className="required-label">
                  MINIO_SECRET_KEY <span className="required-mark">*</span>
                </span>
                <input
                  type="password"
                  value={config.minio.secretKey}
                  onChange={(event) =>
                    setConfig((prev) => ({
                      ...prev,
                      minio: { ...prev.minio, secretKey: event.target.value },
                    }))
                  }
                  autoComplete="off"
                />
              </label>
              <label className="field">
                <span className="required-label">
                  MINIO_BUCKET <span className="required-mark">*</span>
                </span>
                <input
                  type="text"
                  value={config.minio.bucket}
                  onChange={(event) =>
                    setConfig((prev) => ({
                      ...prev,
                      minio: { ...prev.minio, bucket: event.target.value },
                    }))
                  }
                />
              </label>
            </div>
          </section>

          <section className="section" style={{ marginTop: 20 }}>
            <h2>Whisper</h2>
            <div className="form-grid">
              <label className="field">
                <span>WHISPER_MODEL</span>
                <select
                  value={config.whisper.modelPath}
                  onChange={(event) => {
                    const value = event.target.value;
                    setConfig((prev) => ({
                      ...prev,
                      whisper: { ...prev.whisper, modelPath: value },
                    }));
                  }}
                >
                  <option value="ggml-tiny.bin">tiny</option>
                  <option value="ggml-base.bin">base</option>
                  <option value="ggml-small.bin">small</option>
                  <option value="ggml-medium.bin">medium</option>
                  <option value="ggml-large-v2.bin">large-v2</option>
                  <option value="ggml-large-v3.bin">large-v3</option>
                </select>
              </label>
              <label className="field">
                <span>WHISPER_OUTPUT_DIR</span>
                <input
                  type="text"
                  value={config.whisper.outputDir}
                  onChange={(event) =>
                    setConfig((prev) => ({
                      ...prev,
                      whisper: { ...prev.whisper, outputDir: event.target.value },
                    }))
                  }
                />
              </label>
              <label className="field">
                <span>WHISPER_BINARY</span>
                <input
                  type="text"
                  value={config.whisper.binaryPath}
                  onChange={(event) =>
                    setConfig((prev) => ({
                      ...prev,
                      whisper: { ...prev.whisper, binaryPath: event.target.value },
                    }))
                  }
                  placeholder="/opt/homebrew/bin/whisper-cli"
                />
              </label>
              <label className="field">
                <span>FFMPEG_BINARY</span>
                <input
                  type="text"
                  value={config.whisper.ffmpegPath}
                  onChange={(event) =>
                    setConfig((prev) => ({
                      ...prev,
                      whisper: { ...prev.whisper, ffmpegPath: event.target.value },
                    }))
                  }
                  placeholder="/opt/homebrew/bin/ffmpeg"
                />
              </label>
            </div>
          </section>
        </>
      )}
    </main>
  );
}

export default App;
