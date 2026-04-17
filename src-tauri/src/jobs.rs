use std::collections::VecDeque;
use std::sync::Arc;

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
            let on_progress = move |step: usize, status: &str, message: &str, progress: f32| {
                let mut q = state_prog.lock();
                if let Some(j) = q.jobs.iter_mut().find(|j| j.id == id_prog) {
                    j.current_step = step;
                    j.status = if status == "error" { "error".into() } else if status == "done" && progress >= 1.0 { "done".into() } else { "active".into() };
                    j.message = message.to_string();
                    j.progress = progress;
                    let j2 = j.clone();
                    emit_job(&app_prog, &j2);
                }
            };

            let res = pipeline::run_pipeline(app.clone(), url, on_progress).await;

            let mut q = state.lock();
            if let Some(j) = q.jobs.iter_mut().find(|j| j.id == id) {
                match res {
                    Ok(meta) => {
                        j.status = "done".into();
                        j.progress = 1.0;
                        j.message = "Done".into();
                        j.result = Some(meta);
                    }
                    Err(e) => {
                        j.status = "error".into();
                        j.error = Some(e.clone());
                        j.message = e;
                    }
                }
                let j2 = j.clone();
                emit_job(&app, &j2);
                emit_list(&app, &q);
            }
        }
    });
}

pub fn init(app: &AppHandle) {
    let state: JobQueueState = Arc::new(Mutex::new(JobQueue::default()));
    app.manage(state);
}
