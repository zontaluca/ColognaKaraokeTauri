use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::sync::Arc;
use std::thread;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use parking_lot::Mutex;
use tauri::{AppHandle, Manager, State};

/// Shared live audio ring buffer consumed by pitch analyzer.
#[derive(Default)]
pub struct MicBuffer {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
}

pub type MicBufferState = Arc<Mutex<MicBuffer>>;

/// Command channel to the dedicated recorder thread.
pub enum RecCmd {
    Start {
        song_dir: String,
        session_id: String,
        resp: Sender<Result<String, String>>,
    },
    Stop {
        resp: Sender<Result<String, String>>,
    },
}

pub type RecorderTx = Arc<Mutex<Option<Sender<RecCmd>>>>;

const BUFFER_WINDOW_SAMPLES: usize = 16000 * 4;

pub fn init(app: &AppHandle) {
    let buf: MicBufferState = Arc::new(Mutex::new(MicBuffer::default()));
    let tx_slot: RecorderTx = Arc::new(Mutex::new(None));
    app.manage(buf.clone());
    app.manage(tx_slot.clone());

    let buf_clone = buf;
    let tx_slot_clone = tx_slot;
    thread::spawn(move || recorder_thread(buf_clone, tx_slot_clone));
}

fn recorder_thread(buf: MicBufferState, tx_slot: RecorderTx) {
    let (tx, rx) = mpsc::channel::<RecCmd>();
    *tx_slot.lock() = Some(tx);

    let mut active: Option<(
        cpal::Stream,
        Arc<Mutex<Option<hound::WavWriter<std::io::BufWriter<std::fs::File>>>>>,
        PathBuf,
    )> = None;

    while let Ok(cmd) = rx.recv() {
        match cmd {
            RecCmd::Start { song_dir, session_id, resp } => {
                if active.is_some() {
                    let _ = resp.send(Err("Recorder already running".into()));
                    continue;
                }
                match start_stream(&buf, &song_dir, &session_id) {
                    Ok((stream, writer, path)) => {
                        let p = path.to_string_lossy().into_owned();
                        active = Some((stream, writer, path));
                        let _ = resp.send(Ok(p));
                    }
                    Err(e) => {
                        let _ = resp.send(Err(e));
                    }
                }
            }
            RecCmd::Stop { resp } => {
                if let Some((stream, writer, path)) = active.take() {
                    drop(stream);
                    if let Some(w) = writer.lock().take() {
                        let _ = w.finalize();
                    }
                    let _ = resp.send(Ok(path.to_string_lossy().into_owned()));
                } else {
                    let _ = resp.send(Ok(String::new()));
                }
            }
        }
    }
}

fn start_stream(
    buf: &MicBufferState,
    song_dir: &str,
    session_id: &str,
) -> Result<(
    cpal::Stream,
    Arc<Mutex<Option<hound::WavWriter<std::io::BufWriter<std::fs::File>>>>>,
    PathBuf,
), String> {
    let host = cpal::default_host();
    let device = host.default_input_device().ok_or("No default mic")?;
    let config = device.default_input_config().map_err(|e| e.to_string())?;
    let sample_rate = config.sample_rate().0;
    let channels = config.channels() as usize;

    let rec_dir = PathBuf::from(song_dir).join("recordings");
    std::fs::create_dir_all(&rec_dir).map_err(|e| e.to_string())?;
    let out_path = rec_dir.join(format!("{}.wav", session_id));
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let writer = hound::WavWriter::create(&out_path, spec).map_err(|e| e.to_string())?;
    let writer = Arc::new(Mutex::new(Some(writer)));

    {
        let mut b = buf.lock();
        b.samples.clear();
        b.sample_rate = sample_rate;
    }

    let buf_cb = buf.clone();
    let writer_cb = writer.clone();

    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => {
            let cfg: cpal::StreamConfig = config.into();
            device.build_input_stream(
                &cfg,
                move |data: &[f32], _| process_input(data, channels, &buf_cb, &writer_cb),
                |e| eprintln!("mic stream error: {}", e),
                None,
            )
        }
        cpal::SampleFormat::I16 => {
            let cfg: cpal::StreamConfig = config.into();
            device.build_input_stream(
                &cfg,
                move |data: &[i16], _| {
                    let f: Vec<f32> = data.iter().map(|s| *s as f32 / i16::MAX as f32).collect();
                    process_input(&f, channels, &buf_cb, &writer_cb);
                },
                |e| eprintln!("mic stream error: {}", e),
                None,
            )
        }
        cpal::SampleFormat::U16 => {
            let cfg: cpal::StreamConfig = config.into();
            device.build_input_stream(
                &cfg,
                move |data: &[u16], _| {
                    let f: Vec<f32> = data
                        .iter()
                        .map(|s| (*s as f32 - 32768.0) / 32768.0)
                        .collect();
                    process_input(&f, channels, &buf_cb, &writer_cb);
                },
                |e| eprintln!("mic stream error: {}", e),
                None,
            )
        }
        _ => return Err("Unsupported mic sample format".into()),
    }
    .map_err(|e| e.to_string())?;

    stream.play().map_err(|e| e.to_string())?;
    Ok((stream, writer, out_path))
}

fn process_input(
    data: &[f32],
    channels: usize,
    buf: &MicBufferState,
    writer: &Arc<Mutex<Option<hound::WavWriter<std::io::BufWriter<std::fs::File>>>>>,
) {
    let mono: Vec<f32> = if channels <= 1 {
        data.to_vec()
    } else {
        data.chunks(channels)
            .map(|c| c.iter().sum::<f32>() / channels as f32)
            .collect()
    };
    {
        let mut b = buf.lock();
        b.samples.extend_from_slice(&mono);
        if b.samples.len() > BUFFER_WINDOW_SAMPLES {
            let drop_n = b.samples.len() - BUFFER_WINDOW_SAMPLES;
            b.samples.drain(..drop_n);
        }
    }
    if let Some(w) = writer.lock().as_mut() {
        for s in mono {
            let clamped = s.clamp(-1.0, 1.0);
            let _ = w.write_sample((clamped * i16::MAX as f32) as i16);
        }
    }
}

fn send_cmd(tx_slot: &RecorderTx, cmd: RecCmd) -> Result<(), String> {
    let slot = tx_slot.lock();
    let tx = slot.as_ref().ok_or("Recorder thread not ready")?;
    tx.send(cmd).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn recorder_start(
    song_dir: String,
    session_id: String,
    tx: State<'_, RecorderTx>,
) -> Result<String, String> {
    let (resp, rx) = mpsc::channel();
    send_cmd(tx.inner(), RecCmd::Start { song_dir, session_id, resp })?;
    rx.recv().map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn recorder_stop(tx: State<'_, RecorderTx>) -> Result<String, String> {
    let (resp, rx) = mpsc::channel();
    send_cmd(tx.inner(), RecCmd::Stop { resp })?;
    rx.recv().map_err(|e| e.to_string())?
}
