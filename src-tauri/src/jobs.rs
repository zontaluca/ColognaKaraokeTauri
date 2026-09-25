use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};
use uuid::Uuid;

use crate::pipeline;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub url: String,
    pub status: String, // queued | active | done | error | canceled
    pub current_step: usize,
    pub progress: f32,
    pub message: String,
    pub result: Option<serde_json::Value>,
    pub error: Option<String>,
}

#[derive(Default)]
pub struct JobQueue {
    pub jobs: Vec<Job>,
    pub pending: VecDeque<String>,
    pub worker_alive: bool,
    pub cancel_ids: Vec<String>,
}

pub type JobQueueState = Arc<Mutex<JobQueue>>;

/// True while the queue worker is processing jobs.
pub fn worker_alive(app: &AppHandle) -> bool {
    app.try_state::<JobQueueState>()
        .map_or(false, |s| s.lock().worker_alive)
}

/// Rate limiter for `karaoke://jobs` progress events. yt-dlp and demucs print
/// progress many times per second and every event re-renders the frontend.
/// Stage changes, status changes and new kinds of message (the text with its
/// numbers removed) always go through; plain percentage ticks are capped.
struct ProgressThrottle {
    last_emit: Option<Instant>,
    last_step: usize,
    last_kind: String,
}

impl ProgressThrottle {
    const MIN_INTERVAL: Duration = Duration::from_millis(200);

    fn new() -> Self {
        Self { last_emit: None, last_step: usize::MAX, last_kind: String::new() }
    }

    fn should_emit(&mut self, step: usize, status: &str, message: &str) -> bool {
        let kind: String = message.chars().filter(|c| !c.is_ascii_digit()).collect();
        let now = Instant::now();
        let due = self
            .last_emit
            .map_or(true, |t| now.duration_since(t) >= Self::MIN_INTERVAL);
        let emit = due || status != "active" || step != self.last_step || kind != self.last_kind;
        if emit {
            self.last_emit = Some(now);
            self.last_step = step;
            self.last_kind = kind;
        }
        emit
    }
}

fn emit_job(app: &AppHandle, job: &Job) {
    let _ = app.emit("karaoke://jobs", job);
}

fn emit_list(app: &AppHandle, q: &JobQueue) {
    let _ = app.emit("karaoke://jobs-list", &q.jobs);
}

#[tauri::command]
pub fn jobs_enqueue(app: AppHandle, url: String, state: State<'_, JobQueueState>) -> Result<String, String> {
    let id = Uuid::new_v4().to_string();
    let job = Job {
        id: id.clone(),
        url,
        status: "queued".into(),
        current_step: 0,
        progress: 0.0,
        message: "Queued".into(),
        result: None,
        error: None,
    };
    let mut q = state.lock();
    q.jobs.push(job.clone());
    q.pending.push_back(id.clone());
    let should_start = !q.worker_alive;
    if should_start {
        q.worker_alive = true;
    }
    emit_job(&app, &job);
    emit_list(&app, &q);
    drop(q);

    if should_start {
        spawn_worker(app.clone(), state.inner().clone());
    }
    Ok(id)
}

#[tauri::command]
pub fn jobs_list(state: State<'_, JobQueueState>) -> Vec<Job> {
    state.lock().jobs.clone()
}

#[tauri::command]
pub fn jobs_cancel(app: AppHandle, id: String, state: State<'_, JobQueueState>) -> Result<(), String> {
    let mut q = state.lock();
    q.cancel_ids.push(id.clone());
    if let Some(j) = q.jobs.iter_mut().find(|j| j.id == id) {
        if j.status == "queued" {
            j.status = "canceled".into();
            j.message = "Canceled".into();
            let j2 = j.clone();
            emit_job(&app, &j2);
        }
    }
    q.pending.retain(|x| x != &id);
    emit_list(&app, &q);
    Ok(())
}

fn spawn_worker(app: AppHandle, state: JobQueueState) {
    tauri::async_runtime::spawn(async move {
        loop {
            let next_id = {
                let mut q = state.lock();
                q.pending.pop_front()
            };
            let id = match next_id {
                Some(x) => x,
                None => {
                    let mut q = state.lock();
                    q.worker_alive = false;
                    // Queue drained: free the wav2vec2 session kept between jobs.
                    // Done under the queue lock so a worker spawned by a new
                    // enqueue can't have its freshly loaded model dropped.
                    crate::aligner::release_model_cache();
                    break;
                }
            };

            // Check canceled
            {
                let mut q = state.lock();
                if q.cancel_ids.contains(&id) {
                    if let Some(j) = q.jobs.iter_mut().find(|j| j.id == id) {
                        j.status = "canceled".into();
                        j.message = "Canceled".into();
                        let j2 = j.clone();
                        emit_job(&app, &j2);
                    }
                    continue;
                }
            }

            // Mark active
            let url: String = {
                let mut q = state.lock();
                let j = q.jobs.iter_mut().find(|j| j.id == id);
                if let Some(j) = j {
                    j.status = "active".into();
                    j.message = "Starting...".into();
                    let j2 = j.clone();
                    emit_job(&app, &j2);
                    j.url.clone()
                } else {
                    continue;
                }
            };

            let app_prog = app.clone();
            let state_prog = state.clone();
            let id_prog = id.clone();
            let throttle = Arc::new(Mutex::new(ProgressThrottle::new()));
            let on_progress = move |step: usize, status: &str, message: &str, progress: f32| {
                let mut q = state_prog.lock();
                if let Some(j) = q.jobs.iter_mut().find(|j| j.id == id_prog) {
                    j.current_step = step;
                    // "done" is set only once the pipeline returns (below), so the
                    // frontend sees a single done event per job.
                    j.status = if status == "error" { "error".into() } else { "active".into() };
                    j.message = message.to_string();
                    j.progress = progress;
                    if throttle.lock().should_emit(step, status, message) {
                        let j2 = j.clone();
                        emit_job(&app_prog, &j2);
                    }
                }
            };

            let res = pipeline::run_pipeline(app.clone(), url, on_progress).await;

            let maybe_song_dir: Option<String> = {
                let mut q = state.lock();
                let maybe = if let Some(j) = q.jobs.iter_mut().find(|j| j.id == id) {
                    let dir = match &res {
                        Ok(meta) => {
                            let d = meta
                                .get("_pipeline_dir")
                                .and_then(|v| v.as_str())
                                .map(String::from);
                            j.status = "done".into();
                            j.progress = 1.0;
                            j.message = "Done".into();
                            j.result = Some(meta.clone());
                            d
                        }
                        Err(e) => {
                            j.status = "error".into();
                            j.error = Some(e.clone());
                            j.message = e.clone();
                            None
                        }
                    };
                    let j2 = j.clone();
                    emit_job(&app, &j2);
                    dir
                } else {
                    None
                };
                emit_list(&app, &q);
                maybe
            };

            // Auto-sync outside the lock so we don't hold it during network I/O
            if let Some(song_dir) = maybe_song_dir {
                let settings = crate::settings::load_settings(&app);
                if settings.mega.auto_sync && settings.mega.email.is_some() {
                    let app2 = app.clone();
                    tauri::async_runtime::spawn(async move {
                        if crate::cloud::is_network_available().await {
                            let _ = crate::cloud::upload_song(&app2, song_dir).await;
                        }
                    });
                }
            }
        }
    });
}

pub fn init(app: &AppHandle) {
    let state: JobQueueState = Arc::new(Mutex::new(JobQueue::default()));
    app.manage(state);
}

#[cfg(test)]
mod tests {
    use super::ProgressThrottle;

    #[test]
    fn throttle_passes_new_stages_and_messages() {
        let mut t = ProgressThrottle::new();
        assert!(t.should_emit(0, "active", "Downloading... 1%"));
        // Same kind of message right away: suppressed.
        assert!(!t.should_emit(0, "active", "Downloading... 2%"));
        // Status and message-kind changes always pass.
        assert!(t.should_emit(0, "done", "Audio downloaded"));
        assert!(t.should_emit(3, "active", "Separating... 5%"));
        assert!(t.should_emit(3, "active", "Encoding audio..."));
    }
}
