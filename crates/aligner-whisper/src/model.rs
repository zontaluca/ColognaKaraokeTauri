//! Whisper model loading and forced-alignment inference.
//!
//! Key design: the decoder runs a SINGLE forward pass over the full
//! teacher-forced sequence (with causal mask) so each token's cross-attention
//! has proper left-context.  Per-token single-step passes produce flat
//! attention and wrong timestamps.

use candle_core::{DType, Device, IndexOp, Module, Tensor};
use candle_nn::{
    conv1d, embedding, layer_norm, linear, linear_no_bias, ops::softmax, Conv1d, Conv1dConfig,
    Embedding, LayerNorm, Linear, VarBuilder,
};
use hf_hub::{api::tokio::ApiBuilder, Repo, RepoType};
use serde::Deserialize;
use tokenizers::Tokenizer;
use tracing::info;

use crate::AlignError;

// ─── Model config (from config.json) ─────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct WhisperJsonConfig {
    pub d_model: usize,
    pub encoder_attention_heads: usize,
    pub decoder_attention_heads: usize,
    pub encoder_ffn_dim: usize,
    pub decoder_ffn_dim: usize,
    pub encoder_layers: usize,
    pub decoder_layers: usize,
    pub vocab_size: usize,
    pub num_mel_bins: usize,
    pub max_source_positions: usize,
    pub max_target_positions: usize,
}

// ─── HuggingFace repo IDs ────────────────────────────────────────────────────

pub fn model_repo(model: &crate::WhisperModel) -> &'static str {
    match model {
        crate::WhisperModel::Small => "openai/whisper-small",
        crate::WhisperModel::Medium => "openai/whisper-medium",
        crate::WhisperModel::LargeV3Turbo => "openai/whisper-large-v3-turbo",
    }
}

// ─── Loaded resources ────────────────────────────────────────────────────────

pub struct WhisperResources {
    pub config: WhisperJsonConfig,
    pub tokenizer: Tokenizer,
    pub encoder: AudioEncoder,
    pub decoder: ForcedAlignDecoder,
    pub device: Device,
}

impl WhisperResources {
    pub async fn load(
        model: &crate::WhisperModel,
        _cache_dir: &std::path::Path,
    ) -> Result<Self, AlignError> {
        let repo_id = model_repo(model);
        info!("loading Whisper model from {}", repo_id);

        // hf-hub reads HF_ENDPOINT at construction time; an empty env var
        // (common in some shell configs) would produce a relative URL error.
        if std::env::var("HF_ENDPOINT").unwrap_or_default().is_empty() {
            std::env::set_var("HF_ENDPOINT", "https://huggingface.co");
        }
        if std::env::var("HF_HOME").unwrap_or_default().is_empty() {
            if let Some(home) = dirs_next::home_dir() {
                std::env::set_var("HF_HOME", home.join(".cache").join("huggingface"));
            }
        }

        let api = ApiBuilder::new()
            .build()
            .map_err(|e| AlignError::ModelDownload(e.to_string()))?;
        let repo = api.repo(Repo::new(repo_id.to_string(), RepoType::Model));

        let config_file = repo
            .get("config.json")
            .await
            .map_err(|e| AlignError::ModelDownload(e.to_string()))?;
        let tokenizer_file = repo
            .get("tokenizer.json")
            .await
            .map_err(|e| AlignError::ModelDownload(e.to_string()))?;
        let weights_file = repo
            .get("model.safetensors")
            .await
            .map_err(|e| AlignError::ModelDownload(e.to_string()))?;

        let config: WhisperJsonConfig = serde_json::from_str(
            &std::fs::read_to_string(&config_file)
                .map_err(|e| AlignError::ModelDownload(e.to_string()))?,
        )
        .map_err(|e| AlignError::ModelDownload(e.to_string()))?;

        let tokenizer = Tokenizer::from_file(&tokenizer_file)
            .map_err(|e| AlignError::Tokenization(e.to_string()))?;

        let device = best_device();
        let vb = unsafe {
            VarBuilder::from_mmaped_safetensors(&[weights_file], DType::F32, &device)
                .map_err(|e| AlignError::Inference(e.to_string()))?
        };

        let encoder = AudioEncoder::load(vb.pp("model.encoder"), &config)
            .map_err(|e| AlignError::Inference(e.to_string()))?;
        let decoder = ForcedAlignDecoder::load(vb.pp("model.decoder"), &config)
            .map_err(|e| AlignError::Inference(e.to_string()))?;

        Ok(Self { config, tokenizer, encoder, decoder, device })
    }
}

fn best_device() -> Device {
    #[cfg(feature = "metal")]
    {
        Device::new_metal(0).unwrap_or(Device::Cpu)
    }
    #[cfg(not(feature = "metal"))]
    {
        Device::Cpu
    }
}

// ─── Special token IDs ───────────────────────────────────────────────────────

pub struct SpecialTokens {
    pub sot: u32,
    pub eot: u32,
    pub no_timestamps: u32,
    pub lang_id: u32,
    pub transcribe: u32,
}

impl SpecialTokens {
    pub fn for_language(tokenizer: &Tokenizer, language: &str) -> Result<Self, AlignError> {
        let lang_token = format!("<|{}|>", language);
        let lookup = |tok: &str| {
            tokenizer
                .token_to_id(tok)
                .ok_or_else(|| AlignError::Tokenization(format!("token not found: {}", tok)))
        };
        Ok(Self {
            sot: lookup("<|startoftranscript|>")?,
            eot: lookup("<|endoftext|>")?,
            no_timestamps: lookup("<|notimestamps|>")?,
            lang_id: lookup(&lang_token)?,
            transcribe: lookup("<|transcribe|>")?,
        })
    }
}

// ─── Multi-head attention helpers ────────────────────────────────────────────

/// Reshape [b, t, d] → [b, n_head, t, head_dim] for attention.
/// Returns contiguous tensor — Metal matmul requires contiguous inputs.
fn split_heads(x: &Tensor, n_head: usize, head_dim: usize) -> candle_core::Result<Tensor> {
    let (b, t, _) = x.dims3()?;
    x.reshape((b, t, n_head, head_dim))?.permute((0, 2, 1, 3))?.contiguous()
}

/// Merge [b, n_head, t, head_dim] → [b, t, d].
fn merge_heads(x: &Tensor) -> candle_core::Result<Tensor> {
    let (b, n_head, t, head_dim) = x.dims4()?;
    x.permute((0, 2, 1, 3))?.contiguous()?.reshape((b, t, n_head * head_dim))
}

// ─── Encoder ─────────────────────────────────────────────────────────────────

struct SelfAttention {
    q_proj: Linear,
    k_proj: Linear,
    v_proj: Linear,
    out_proj: Linear,
    n_head: usize,
    head_dim: usize,
    scale: f64,
}

impl SelfAttention {
    fn load(vb: VarBuilder, d: usize, n_head: usize) -> candle_core::Result<Self> {
        let head_dim = d / n_head;
        Ok(Self {
            q_proj: linear(d, d, vb.pp("q_proj"))?,
            k_proj: linear_no_bias(d, d, vb.pp("k_proj"))?,
            v_proj: linear(d, d, vb.pp("v_proj"))?,
            out_proj: linear(d, d, vb.pp("out_proj"))?,
            n_head,
            head_dim,
            scale: (head_dim as f64).powf(-0.5),
        })
    }

    /// `mask`: additive bias broadcastable to [b, n_head, t, t].
    fn forward(&self, x: &Tensor, mask: Option<&Tensor>) -> candle_core::Result<Tensor> {
        let q = split_heads(&self.q_proj.forward(x)?, self.n_head, self.head_dim)?;
        let k = split_heads(&self.k_proj.forward(x)?, self.n_head, self.head_dim)?;
        let v = split_heads(&self.v_proj.forward(x)?, self.n_head, self.head_dim)?;

        let attn = (q.matmul(&k.permute((0, 1, 3, 2))?.contiguous()?)? * self.scale)?;
        let attn = match mask {
            Some(m) => attn.broadcast_add(m)?,
            None => attn,
        };
        let attn = softmax(&attn, 3)?;
        let out = merge_heads(&attn.matmul(&v)?)?;
        self.out_proj.forward(&out)
    }
}

struct EncoderLayer {
    self_attn: SelfAttention,
    self_attn_ln: LayerNorm,
    fc1: Linear,
    fc2: Linear,
    final_ln: LayerNorm,
}

impl EncoderLayer {
    fn load(vb: VarBuilder, cfg: &WhisperJsonConfig) -> candle_core::Result<Self> {
        let d = cfg.d_model;
        Ok(Self {
            self_attn: SelfAttention::load(vb.pp("self_attn"), d, cfg.encoder_attention_heads)?,
            self_attn_ln: layer_norm(d, 1e-5, vb.pp("self_attn_layer_norm"))?,
            fc1: linear(d, cfg.encoder_ffn_dim, vb.pp("fc1"))?,
            fc2: linear(cfg.encoder_ffn_dim, d, vb.pp("fc2"))?,
            final_ln: layer_norm(d, 1e-5, vb.pp("final_layer_norm"))?,
        })
    }

    fn forward(&self, x: &Tensor) -> candle_core::Result<Tensor> {
        let residual = x;
        let x = (residual + self.self_attn.forward(&self.self_attn_ln.forward(x)?, None)?)?;
        let residual = &x;
        let x_ff = self
            .fc2
            .forward(&self.fc1.forward(&self.final_ln.forward(&x)?)?.gelu()?)?;
        residual + x_ff
    }
}

pub struct AudioEncoder {
    conv1: Conv1d,
    conv2: Conv1d,
    positional_embedding: Tensor,
    layers: Vec<EncoderLayer>,
    ln_post: LayerNorm,
}

impl AudioEncoder {
    pub fn load(vb: VarBuilder, cfg: &WhisperJsonConfig) -> candle_core::Result<Self> {
        let conv1_cfg = Conv1dConfig { padding: 1, stride: 1, ..Default::default() };
        let conv2_cfg = Conv1dConfig { padding: 1, stride: 2, ..Default::default() };
        let layers = (0..cfg.encoder_layers)
            .map(|i| EncoderLayer::load(vb.pp(format!("layers.{}", i)), cfg))
            .collect::<candle_core::Result<Vec<_>>>()?;
        Ok(Self {
            conv1: conv1d(cfg.num_mel_bins, cfg.d_model, 3, conv1_cfg, vb.pp("conv1"))?,
            conv2: conv1d(cfg.d_model, cfg.d_model, 3, conv2_cfg, vb.pp("conv2"))?,
            positional_embedding: vb.get(
                (cfg.max_source_positions, cfg.d_model),
                "embed_positions.weight",
            )?,
            layers,
            ln_post: layer_norm(cfg.d_model, 1e-5, vb.pp("layer_norm"))?,
        })
    }

    /// `mel`: [1, n_mels, T_frames]  → returns [1, T_enc, d_model]
    pub fn forward(&self, mel: &Tensor) -> candle_core::Result<Tensor> {
        let x = self.conv1.forward(mel)?.gelu()?;
        let x = self.conv2.forward(&x)?.gelu()?;
        let x = x.permute((0, 2, 1))?;   // [1, T_enc, d]
        let t_enc = x.dim(1)?;
        // Slice positional embedding to match actual encoder length.
        let pos = self.positional_embedding.i(..t_enc)?.unsqueeze(0)?;
        let x = x.broadcast_add(&pos)?;
        let x = self.layers.iter().try_fold(x, |acc, l| l.forward(&acc))?;
        self.ln_post.forward(&x)
    }
}

// ─── Decoder ─────────────────────────────────────────────────────────────────

/// Cross-attention block: returns (output [b,t,d], weights [b, n_head, t, T_enc]).
struct MultiHeadCrossAttention {
    q_proj: Linear,
    k_proj: Linear,
    v_proj: Linear,
    out_proj: Linear,
    n_head: usize,
    head_dim: usize,
    scale: f64,
}

impl MultiHeadCrossAttention {
    fn load(vb: VarBuilder, d: usize, n_head: usize) -> candle_core::Result<Self> {
        let head_dim = d / n_head;
        Ok(Self {
            q_proj: linear(d, d, vb.pp("q_proj"))?,
            k_proj: linear_no_bias(d, d, vb.pp("k_proj"))?,
            v_proj: linear(d, d, vb.pp("v_proj"))?,
            out_proj: linear(d, d, vb.pp("out_proj"))?,
            n_head,
            head_dim,
            scale: (head_dim as f64).powf(-0.5),
        })
    }

    /// `x`:  [b, t_dec, d]  — decoder hidden states
    /// `xa`: [b, T_enc, d]  — encoder output
    ///
    /// Returns (output [b, t_dec, d], weights [b, n_head, t_dec, T_enc]).
    fn forward(&self, x: &Tensor, xa: &Tensor) -> candle_core::Result<(Tensor, Tensor)> {
        let (_, t_enc, _) = xa.dims3()?;
        let q = split_heads(&self.q_proj.forward(x)?, self.n_head, self.head_dim)?;
        let k = split_heads(&self.k_proj.forward(xa)?, self.n_head, self.head_dim)?;
        let v = split_heads(&self.v_proj.forward(xa)?, self.n_head, self.head_dim)?;

        let _ = t_enc; // used implicitly in matmul
        let attn = (q.matmul(&k.permute((0, 1, 3, 2))?.contiguous()?)? * self.scale)?;
        let weights = softmax(&attn, 3)?;  // [b, n_head, t_dec, T_enc]
        let out = merge_heads(&weights.matmul(&v)?)?;
        let out = self.out_proj.forward(&out)?;
        Ok((out, weights))
    }
}

struct DecoderLayer {
    self_attn: SelfAttention,
    self_attn_ln: LayerNorm,
    cross_attn: MultiHeadCrossAttention,
    cross_attn_ln: LayerNorm,
    final_ln: LayerNorm,
    fc1: Linear,
    fc2: Linear,
}

impl DecoderLayer {
    fn load(vb: VarBuilder, cfg: &WhisperJsonConfig) -> candle_core::Result<Self> {
        let d = cfg.d_model;
        Ok(Self {
            self_attn: SelfAttention::load(vb.pp("self_attn"), d, cfg.decoder_attention_heads)?,
            self_attn_ln: layer_norm(d, 1e-5, vb.pp("self_attn_layer_norm"))?,
            cross_attn: MultiHeadCrossAttention::load(
                vb.pp("encoder_attn"),
                d,
                cfg.decoder_attention_heads,
            )?,
            cross_attn_ln: layer_norm(d, 1e-5, vb.pp("encoder_attn_layer_norm"))?,
            final_ln: layer_norm(d, 1e-5, vb.pp("final_layer_norm"))?,
            fc1: linear(d, cfg.decoder_ffn_dim, vb.pp("fc1"))?,
            fc2: linear(cfg.decoder_ffn_dim, d, vb.pp("fc2"))?,
        })
    }

    /// Full sequence forward.
    /// `x`:  [1, t_dec, d]
    /// `xa`: [1, T_enc, d]
    /// `mask`: additive causal mask [t_dec, t_dec] (−∞ for future positions)
    ///
    /// Returns (hidden [1,t_dec,d], cross_attn_weights [1, n_head, t_dec, T_enc]).
    fn forward(
        &self,
        x: &Tensor,
        xa: &Tensor,
        causal_mask: &Tensor,
    ) -> candle_core::Result<(Tensor, Tensor)> {
        // Masked self-attention.
        let residual = x;
        let x_sa =
            self.self_attn.forward(&self.self_attn_ln.forward(x)?, Some(causal_mask))?;
        let x = (residual + x_sa)?;

        // Cross-attention.
        let residual = &x;
        let (x_ca, weights) = self.cross_attn.forward(&self.cross_attn_ln.forward(&x)?, xa)?;
        let x = (residual + x_ca)?;

        // FFN.
        let residual = &x;
        let x_ff = self
            .fc2
            .forward(&self.fc1.forward(&self.final_ln.forward(&x)?)?.gelu()?)?;
        let x = (residual + x_ff)?;

        Ok((x, weights))
    }
}

pub struct ForcedAlignDecoder {
    token_embedding: Embedding,
    positional_embedding: Tensor,
    layers: Vec<DecoderLayer>,
    n_layer: usize,
}

impl ForcedAlignDecoder {
    pub fn load(vb: VarBuilder, cfg: &WhisperJsonConfig) -> candle_core::Result<Self> {
        let n_layer = cfg.decoder_layers;
        let layers = (0..n_layer)
            .map(|i| DecoderLayer::load(vb.pp(format!("layers.{}", i)), cfg))
            .collect::<candle_core::Result<Vec<_>>>()?;
        Ok(Self {
            token_embedding: embedding(cfg.vocab_size, cfg.d_model, vb.pp("embed_tokens"))?,
            positional_embedding: vb.get(
                (cfg.max_target_positions, cfg.d_model),
                "embed_positions.weight",
            )?,
            layers,
            n_layer,
        })
    }

    /// Run the full token sequence through the decoder in a single pass with
    /// causal masking, then return averaged cross-attention from the last
    /// `attention_layers` decoder layers as `[n_tokens, T_enc]`.
    pub fn forced_attention(
        &self,
        encoder_out: &Tensor, // [1, T_enc, d]
        tokens: &[u32],
        attention_layers: usize,
        device: &Device,
    ) -> candle_core::Result<Vec<Vec<f32>>> {
        let n = tokens.len();

        // Token + positional embeddings.
        let tok_t = Tensor::new(tokens, device)?.unsqueeze(0)?;  // [1, n]
        let emb = self.token_embedding.forward(&tok_t)?;          // [1, n, d]
        let pos = self.positional_embedding.i(..n)?.unsqueeze(0)?; // [1, n, d]
        let mut x = emb.broadcast_add(&pos)?;                     // [1, n, d]

        // Build causal mask [1, 1, n, n] broadcastable to [1, n_head, n, n].
        let causal: Vec<f32> = (0..n)
            .flat_map(|i| (0..n).map(move |j| if j <= i { 0.0f32 } else { f32::NEG_INFINITY }))
            .collect();
        let causal_mask =
            Tensor::from_vec(causal, (1, 1, n, n), device)?.to_dtype(DType::F32)?;

        let eff_layers = if attention_layers == 0 { self.n_layer } else { attention_layers };
        let last_layer_start = self.n_layer.saturating_sub(eff_layers);
        let mut sum_attn: Option<Tensor> = None; // [1, n_head, n, T_enc]
        let mut n_acc = 0usize;

        for (li, layer) in self.layers.iter().enumerate() {
            let (x_new, weights) = layer.forward(&x, encoder_out, &causal_mask)?;
            x = x_new;
            if li >= last_layer_start {
                sum_attn = Some(match sum_attn {
                    None => weights,
                    Some(prev) => (prev + weights)?,
                });
                n_acc += 1;
            }
        }

        let avg = sum_attn
            .ok_or_else(|| candle_core::Error::Msg("no attention layers collected".into()))?;
        // avg: [1, n_head, n, T_enc] → average over heads → [1, n, T_enc]
        let avg = (avg / n_acc as f64)?.mean(1)?; // [1, n, T_enc]
        let avg = avg.squeeze(0)?;                 // [n, T_enc]

        // Convert to Vec<Vec<f32>>.
        let flat = avg.to_vec2::<f32>()?;
        Ok(flat)
    }
}
