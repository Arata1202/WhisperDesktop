import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import "./App.css";

type MeetingSummary = {
  id: string;
  date: string;
  roomId: string;
  meetingTime: string;
  speakerCount: number;
  trackCount: number;
};

type JobStatus = {
  state: string;
  completed: number;
  total: number;
  outputPath?: string;
  error?: string;
};

function App() {
  const [status, setStatus] = useState("");
  const [dates, setDates] = useState<string[]>([]);
  const [meetings, setMeetings] = useState<MeetingSummary[]>([]);
  const [selectedDate, setSelectedDate] = useState<string | null>(null);
  const [selectedMeeting, setSelectedMeeting] = useState<MeetingSummary | null>(
    null,
  );
  const [jobId, setJobId] = useState<string | null>(null);
  const [jobStatus, setJobStatus] = useState<JobStatus | null>(null);
  const [datesLoading, setDatesLoading] = useState(false);
  const [meetingsLoading, setMeetingsLoading] = useState(false);
  const [transcribeLoading, setTranscribeLoading] = useState(false);

  const pollingRef = useRef<number | null>(null);

  const hasDates = dates.length > 0;
  const hasMeetings = meetings.length > 0;

  const selectedMeetingId = useMemo(
    () => selectedMeeting?.id ?? null,
    [selectedMeeting],
  );

  const refreshDates = async () => {
    setDatesLoading(true);
    try {
      const result = await invoke<string[]>("list_dates");
      setDates(result);
      setStatus(`日付取得: ${result.join(", ") || "none"}`);
      if (result.length > 0) {
        setSelectedDate(result[0]);
      } else {
        setSelectedDate(null);
        setMeetings([]);
        setSelectedMeeting(null);
      }
    } catch (err) {
      setStatus(`日付一覧の取得に失敗しました: ${String(err)}`);
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
      setSelectedMeeting(result[0] ?? null);
      setStatus("会議一覧を更新しました。");
    } catch (err) {
      setStatus(`会議一覧の取得に失敗しました: ${String(err)}`);
    } finally {
      setMeetingsLoading(false);
    }
  };

  const startTranscribe = async () => {
    if (!selectedMeetingId) return;
    setTranscribeLoading(true);
    try {
      const createdJobId = await invoke<string>("start_transcribe", {
        meetingId: selectedMeetingId,
      });
      setJobId(createdJobId);
      setJobStatus(null);
      setStatus("文字起こしを開始しました。");
    } catch (err) {
      setStatus(`文字起こしの開始に失敗しました: ${String(err)}`);
    } finally {
      setTranscribeLoading(false);
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
        setStatus(`進捗の取得に失敗しました: ${String(err)}`);
        stopPolling();
      }
    }, 1500);
  };

  useEffect(() => {
    refreshDates();
    return () => {
      stopPolling();
    };
  }, []);

  useEffect(() => {
    if (selectedDate) {
      refreshMeetings(selectedDate);
    }
  }, [selectedDate]);

  useEffect(() => {
    if (!jobId) return;
    startPolling();
    return () => {
      stopPolling();
    };
  }, [jobId]);

  return (
    <main className="app-shell">
      <h1>WhisperDesktop</h1>
      <div className="grid two">
        <section className="section">
          <div className="row">
            <h2>日付一覧</h2>
            <div className="spacer" />
            <button
              type="button"
              className="ghost"
              onClick={refreshDates}
              disabled={datesLoading}
            >
              更新
            </button>
          </div>
          <ul className="list">
            {hasDates ? (
              dates.map((date) => (
                <li key={date}>
                  <button
                    type="button"
                    className={date === selectedDate ? "selected" : ""}
                    onClick={() => setSelectedDate(date)}
                  >
                    {date}
                  </button>
                </li>
              ))
            ) : (
              <li className="notice">日付がありません</li>
            )}
          </ul>
        </section>

        <section className="section">
          <div className="row">
            <h2>会議一覧</h2>
            <div className="spacer" />
            <button
              type="button"
              className="ghost"
              onClick={() => refreshMeetings()}
              disabled={meetingsLoading}
            >
              更新
            </button>
          </div>
          <ul className="list">
            {hasMeetings ? (
              meetings.map((meeting) => (
                <li key={meeting.id}>
                  <button
                    type="button"
                    className={meeting.id === selectedMeetingId ? "selected" : ""}
                    onClick={() => setSelectedMeeting(meeting)}
                  >
                    {meeting.meetingTime} | {meeting.roomId} (
                    {meeting.speakerCount}人/{meeting.trackCount}件)
                  </button>
                </li>
              ))
            ) : (
              <li className="notice">会議がありません</li>
            )}
          </ul>
        </section>
      </div>

      <section className="section" style={{ marginTop: 20 }}>
        <div className="row">
          <h2>文字起こし</h2>
          <div className="spacer" />
          <button
            type="button"
            onClick={startTranscribe}
            disabled={!selectedMeetingId || transcribeLoading}
          >
            開始
          </button>
        </div>
        {jobStatus ? (
          <div className="notice">
            状態: {jobStatus.state} | {jobStatus.completed}/{jobStatus.total}
            {jobStatus.outputPath ? (
              <div className="mono">{jobStatus.outputPath}</div>
            ) : null}
            {jobStatus.error ? (
              <div className="mono">{jobStatus.error}</div>
            ) : null}
          </div>
        ) : (
          <div className="notice">会議を選択して開始してください。</div>
        )}
      </section>

      {status ? <p className="notice status">{status}</p> : null}
    </main>
  );
}

export default App;
