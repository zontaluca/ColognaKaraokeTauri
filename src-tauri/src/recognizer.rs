/// Song recognition via Shazam API, using the fingerprinting algorithm from SongRec
/// (https://github.com/marin-m/SongRec, GPL-3.0).
/// The fingerprinting logic is adapted from SongRec's core; the HTTP transport is
/// replaced with reqwest so no GLib/libsoup dependency is required.
use std::collections::HashMap;
use std::io::{Cursor, Seek, SeekFrom, Write};
use std::path::Path;
use std::time::SystemTime;

use base64::Engine;
use byteorder::{LittleEndian, WriteBytesExt};
use chfft::RFft1D;
use crc32fast::Hasher as Crc32Hasher;
use once_cell::sync::Lazy;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Hanning window (2048 entries, computed once)
// ---------------------------------------------------------------------------

static HANNING: Lazy<[f32; 2048]> = Lazy::new(|| {
    let mut w = [0.0f32; 2048];
    for (i, v) in w.iter_mut().enumerate() {
        *v = 0.5 * (1.0 - (2.0 * std::f64::consts::PI * i as f64 / 2047.0).cos()) as f32;
    }
    w
});

// ---------------------------------------------------------------------------
// Shazam fingerprint types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord)]
enum FrequencyBand {
    B250_520 = 0,
    B520_1450 = 1,
    B1450_3500 = 2,
    B3500_5500 = 3,
}

struct FrequencyPeak {
    fft_pass_number: u32,
    peak_magnitude: u16,
    corrected_peak_frequency_bin: u16,
}

struct DecodedSignature {
    sample_rate_hz: u32,
    number_samples: u32,
    frequency_band_to_sound_peaks: HashMap<FrequencyBand, Vec<FrequencyPeak>>,
}

impl DecodedSignature {
    fn encode_to_binary(&self) -> Vec<u8> {
        let mut cursor = Cursor::new(vec![]);
        cursor.write_u32::<LittleEndian>(0xcafe2580).unwrap(); // magic1
        cursor.write_u32::<LittleEndian>(0).unwrap(); // crc32 placeholder
        cursor.write_u32::<LittleEndian>(0).unwrap(); // size_minus_header placeholder
        cursor.write_u32::<LittleEndian>(0x94119c00).unwrap(); // magic2
        cursor.write_u32::<LittleEndian>(0).unwrap();
        cursor.write_u32::<LittleEndian>(0).unwrap();
        cursor.write_u32::<LittleEndian>(0).unwrap();
        let rate_id: u32 = match self.sample_rate_hz {
            8000 => 1, 11025 => 2, 16000 => 3, 32000 => 4, 44100 => 5, 48000 => 6, _ => 3,
        };
        cursor.write_u32::<LittleEndian>(rate_id << 27).unwrap();
        cursor.write_u32::<LittleEndian>(0).unwrap();
        cursor.write_u32::<LittleEndian>(0).unwrap();
        cursor
            .write_u32::<LittleEndian>(
                self.number_samples + (self.sample_rate_hz as f32 * 0.24) as u32,
            )
            .unwrap();
        cursor.write_u32::<LittleEndian>((15 << 19) + 0x40000).unwrap();
        cursor.write_u32::<LittleEndian>(0x40000000).unwrap();
        cursor.write_u32::<LittleEndian>(0).unwrap(); // size_minus_header placeholder

        let mut sorted: Vec<_> = self.frequency_band_to_sound_peaks.iter().collect();
        sorted.sort_by_key(|(b, _)| **b);

        for (band, peaks) in &sorted {
            let mut peaks_buf = Cursor::new(vec![]);
            let mut fft_pass = 0u32;
            for p in *peaks {
                if p.fft_pass_number - fft_pass >= 255 {
                    peaks_buf.write_u8(0xff).unwrap();
                    peaks_buf.write_u32::<LittleEndian>(p.fft_pass_number).unwrap();
                    fft_pass = p.fft_pass_number;
                }
                peaks_buf
                    .write_u8((p.fft_pass_number - fft_pass) as u8)
                    .unwrap();
                peaks_buf.write_u16::<LittleEndian>(p.peak_magnitude).unwrap();
                peaks_buf
                    .write_u16::<LittleEndian>(p.corrected_peak_frequency_bin)
                    .unwrap();
                fft_pass = p.fft_pass_number;
            }
            let pb = peaks_buf.into_inner();
            cursor
                .write_u32::<LittleEndian>(0x60030040 + **band as u32)
                .unwrap();
            cursor.write_u32::<LittleEndian>(pb.len() as u32).unwrap();
            cursor.write_all(&pb).unwrap();
            let pad = (4 - pb.len() % 4) % 4;
            for _ in 0..pad {
                cursor.write_u8(0).unwrap();
            }
        }

        let total = cursor.position() as u32;
        // patch size_minus_header at offset 8
        cursor.seek(SeekFrom::Start(8)).unwrap();
        cursor.write_u32::<LittleEndian>(total - 48).unwrap();
        // patch second size_minus_header at offset 52
        cursor.seek(SeekFrom::Start(52)).unwrap();
        cursor.write_u32::<LittleEndian>(total - 48).unwrap();
        // patch crc32 at offset 4
        let buf = cursor.into_inner();
        let mut hasher = Crc32Hasher::new();
        hasher.update(&buf[8..]);
        let crc = hasher.finalize();
        let mut buf = buf;
        buf[4..8].copy_from_slice(&crc.to_le_bytes());

        buf
    }

    fn encode_to_uri(&self) -> String {
        format!(
            "data:audio/vnd.shazam.sig;base64,{}",
            base64::prelude::BASE64_STANDARD.encode(self.encode_to_binary())
        )
    }
}

// ---------------------------------------------------------------------------
// Fingerprint generator (adapted from SongRec algorithm.rs)
// ---------------------------------------------------------------------------

struct SignatureGenerator {
    ring_buffer: Vec<i16>,
    ring_buffer_idx: usize,
    reordered: Vec<f32>,
    fft_outputs: Vec<Vec<f32>>,
    fft_outputs_idx: usize,
    fft: RFft1D<f32>,
    spread_fft_outputs: Vec<Vec<f32>>,
    spread_fft_outputs_idx: usize,
    num_spread_ffts_done: u32,
    signature: DecodedSignature,
}

impl SignatureGenerator {
    fn make_signature_from_buffer(f32_mono_16khz: &[f32]) -> DecodedSignature {
        let mut gen = SignatureGenerator {
            ring_buffer: vec![0i16; 2048],
            ring_buffer_idx: 0,
            reordered: vec![0.0f32; 2048],
            fft_outputs: vec![vec![0.0f32; 1025]; 256],
            fft_outputs_idx: 0,
            fft: RFft1D::<f32>::new(2048),
            spread_fft_outputs: vec![vec![0.0f32; 1025]; 256],
            spread_fft_outputs_idx: 0,
            num_spread_ffts_done: 0,
            signature: DecodedSignature {
                sample_rate_hz: 16000,
                number_samples: f32_mono_16khz.len() as u32,
                frequency_band_to_sound_peaks: HashMap::new(),
            },
        };

        // Convert f32 → i16
        let s16: Vec<i16> = f32_mono_16khz
            .iter()
            .map(|&s| (s.clamp(-1.0, 1.0) * 32767.0) as i16)
            .collect();

        for chunk in s16.chunks_exact(128) {
            gen.do_fft(chunk);
            gen.do_peak_spreading();
            gen.num_spread_ffts_done += 1;
            if gen.num_spread_ffts_done >= 46 {
                gen.do_peak_recognition();
            }
        }

        gen.signature
    }

    fn do_fft(&mut self, chunk: &[i16]) {
        self.ring_buffer[self.ring_buffer_idx..self.ring_buffer_idx + 128].copy_from_slice(chunk);
        self.ring_buffer_idx = (self.ring_buffer_idx + 128) & 2047;

        let hanning = &*HANNING;
        for (i, mult) in hanning.iter().enumerate() {
            self.reordered[i] =
                self.ring_buffer[(i + self.ring_buffer_idx) & 2047] as f32 * mult;
        }

        let fft_result = self.fft.forward(&self.reordered);
        let real_fft = &mut self.fft_outputs[self.fft_outputs_idx];
        for i in 0..=1024 {
            real_fft[i] = ((fft_result[i].re.powi(2) + fft_result[i].im.powi(2))
                / ((1 << 17) as f32))
                .max(0.0000000001);
        }
        self.fft_outputs_idx = (self.fft_outputs_idx + 1) & 255;
    }

    fn do_peak_spreading(&mut self) {
        let prev = ((self.fft_outputs_idx as i32 - 1) & 255) as usize;
        let real_fft = self.fft_outputs[prev].clone();
        let spread = &mut self.spread_fft_outputs[self.spread_fft_outputs_idx];
        spread.copy_from_slice(&real_fft);
        for p in 0..=1022 {
            spread[p] = spread[p].max(spread[p + 1]).max(spread[p + 2]);
        }
        let spread_copy = spread.clone();
        for p in 0..=1024 {
            for &offset in &[1i32, 3, 6] {
                let idx = ((self.spread_fft_outputs_idx as i32 - offset) & 255) as usize;
                self.spread_fft_outputs[idx][p] =
                    self.spread_fft_outputs[idx][p].max(spread_copy[p]);
            }
        }
        self.spread_fft_outputs_idx = (self.spread_fft_outputs_idx + 1) & 255;
    }

    fn do_peak_recognition(&mut self) {
        let fft_m46 =
            self.fft_outputs[((self.fft_outputs_idx as i32 - 46) & 255) as usize].clone();
        let spread_m49 = self.spread_fft_outputs
            [((self.spread_fft_outputs_idx as i32 - 49) & 255) as usize]
            .clone();

        for bin in 10..=1014usize {
            if fft_m46[bin] < 1.0 / 64.0 || fft_m46[bin] < spread_m49[bin - 1] {
                continue;
            }
            let mut max_neighbor: f32 = 0.0;
            for &off in &[-10i32, -7, -4, -3, 1, 2, 5, 8] {
                max_neighbor = max_neighbor.max(spread_m49[(bin as i32 + off) as usize]);
            }
            if fft_m46[bin] <= max_neighbor {
                continue;
            }
            let mut max_other: f32 = max_neighbor;
            for &off in &[
                -53i32, -45, 165, 172, 179, 186, 193, 200, 214, 221, 228, 235, 242, 249,
            ] {
                let idx = ((self.spread_fft_outputs_idx as i32 + off) & 255) as usize;
                max_other = max_other.max(self.spread_fft_outputs[idx][bin - 1]);
            }
            if fft_m46[bin] <= max_other {
                continue;
            }

            let fft_pass_number = self.num_spread_ffts_done - 46;
            let pm = |v: f32| (v.ln().max(1.0 / 64.0) * 1477.3 + 6144.0) as f32;
            let mag = pm(fft_m46[bin]);
            let mag_before = pm(fft_m46[bin - 1]);
            let mag_after = pm(fft_m46[bin + 1]);
            let var1 = mag * 2.0 - mag_before - mag_after;
            let var2 = (mag_after - mag_before) * 32.0 / var1;
            let corrected_bin = ((bin as i32 * 64) + var2 as i32) as u16;

            let freq_hz = corrected_bin as f32 * (16000.0 / 2.0 / 1024.0 / 64.0);
            let band = match freq_hz as i32 {
                250..=519 => FrequencyBand::B250_520,
                520..=1449 => FrequencyBand::B520_1450,
                1450..=3499 => FrequencyBand::B1450_3500,
                3500..=5500 => FrequencyBand::B3500_5500,
                _ => continue,
            };

            self.signature
                .frequency_band_to_sound_peaks
                .entry(band)
                .or_default()
                .push(FrequencyPeak {
                    fft_pass_number,
                    peak_magnitude: mag as u16,
                    corrected_peak_frequency_bin: corrected_bin,
                });
        }
    }
}

// ---------------------------------------------------------------------------
// Public interface
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct SongInfo {
    pub title: String,
    pub artist: String,
}

/// Recognize a song from an audio file via Shazam.
/// Returns None if recognition fails or no match is found (non-fatal).
pub async fn recognize_song(audio_path: &Path) -> Option<SongInfo> {
    let path = audio_path.to_owned();
    // Load + fingerprint in a blocking thread (CPU-intensive)
    let signature = tokio::task::spawn_blocking(move || -> Result<DecodedSignature, String> {
        // Take 12 seconds from the middle (same strategy as SongRec). The excerpt
        // is cut at the native rate so only those seconds go through the
        // resampler, instead of converting the whole song to 16 kHz first.
        let (excerpt, sample_rate) = crate::audio::load_mono_middle(&path, 12.0)?;
        let samples = crate::audio::resample_to(&excerpt, sample_rate, 16000)?;
        if samples.is_empty() {
            return Err("audio decoded to 0 samples".into());
        }
        Ok(SignatureGenerator::make_signature_from_buffer(&samples))
    })
    .await
    .ok()?
    .ok()?;

    // POST to Shazam API
    let result = query_shazam(&signature).await.ok()?;

    let track = result.get("track")?;
    let title = track.get("title")?.as_str()?.to_string();
    let artist = track.get("subtitle")?.as_str()?.to_string();

    if title.is_empty() || artist.is_empty() {
        return None;
    }

    eprintln!("[recognizer] Shazam identified: {} — {}", artist, title);
    Some(SongInfo { title, artist })
}

async fn query_shazam(
    signature: &DecodedSignature,
) -> Result<serde_json::Value, String> {
    let timestamp_ms = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    let uuid1 = Uuid::new_v4().hyphenated().to_string().to_uppercase();
    let uuid2 = Uuid::new_v4().hyphenated().to_string();

    let url = format!(
        "https://amp.shazam.com/discovery/v5/en/US/android/-/tag/{}/{}?sync=true&webv3=true&sampling=true&connected=&shazamapiversion=v3&sharehub=true&video=v3",
        uuid1, uuid2
    );

    let sample_ms =
        (signature.number_samples as f32 / signature.sample_rate_hz as f32 * 1000.0) as u32;

    let body = serde_json::json!({
        "geolocation": { "altitude": 300, "latitude": 45, "longitude": 2 },
        "signature": {
            "samplems": sample_ms,
            "timestamp": timestamp_ms as u32,
            "uri": signature.encode_to_uri()
        },
        "timestamp": timestamp_ms as u32,
        "timezone": "Europe/Paris"
    });

    let client = crate::http::client().ok_or("HTTP client unavailable")?;

    let resp = client
        .post(&url)
        .header(reqwest::header::USER_AGENT, "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36")
        .header("Content-Language", "en_US")
        .header("Content-Type", "application/json")
        .body(body.to_string())
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if !resp.status().is_success() {
        return Err(format!("Shazam returned HTTP {}", resp.status()));
    }

    resp.json::<serde_json::Value>()
        .await
        .map_err(|e| e.to_string())
}
