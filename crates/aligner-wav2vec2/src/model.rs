use std::path::{Path, PathBuf};

use hf_hub::api::tokio::ApiBuilder;
use hf_hub::{Repo, RepoType};
use ndarray::{Array2, ArrayView2};
use ort::session::{builder::GraphOptimizationLevel, Session};
use ort::value::Tensor;

use crate::AlignError;

/// Where to look for the ONNX weights and `vocab.json`.
#[derive(Debug, Clone)]
pub enum ModelSource {
    /// Pre-staged directory containing `model.onnx` and `vocab.json`.
    LocalDir(PathBuf),
    /// HuggingFace Hub repo. Files downloaded into the user's HF cache and
    /// reused on subsequent runs.
    HfHub {
        repo: String,
        revision: Option<String>,
    },
}

/// Resolved on-disk paths after [`resolve_model_paths`].
pub struct ResolvedModelPaths {
    pub onnx: PathBuf,
    pub vocab: PathBuf,
}

/// Materialise the ONNX + vocab files locally. For [`ModelSource::HfHub`] this
/// uses the standard hf-hub cache (`~/.cache/huggingface/hub`).
pub async fn resolve_model_paths(source: &ModelSource) -> Result<ResolvedModelPaths, AlignError> {
    match source {
        ModelSource::LocalDir(dir) => {
            let onnx = dir.join("model.onnx");
            let vocab = dir.join("vocab.json");
            ensure_exists(&onnx)?;
            ensure_exists(&vocab)?;
            Ok(ResolvedModelPaths { onnx, vocab })
        }
        ModelSource::HfHub { repo, revision } => {
            let api = ApiBuilder::new()
                .build()
                .map_err(|e| AlignError::ModelLoad(format!("hf api init: {}", e)))?;
            let repo_handle = match revision {
                Some(r) => api.repo(Repo::with_revision(
                    repo.clone(),
                    RepoType::Model,
                    r.clone(),
                )),
                None => api.repo(Repo::model(repo.clone())),
            };
            let onnx = repo_handle
                .get("model.onnx")
                .await
                .map_err(|e| AlignError::ModelLoad(format!("download model.onnx: {}", e)))?;
            let vocab = repo_handle
                .get("vocab.json")
                .await
                .map_err(|e| AlignError::ModelLoad(format!("download vocab.json: {}", e)))?;
            Ok(ResolvedModelPaths { onnx, vocab })
        }
    }
}

fn ensure_exists(p: &Path) -> Result<(), AlignError> {
    if !p.exists() {
        return Err(AlignError::ModelLoad(format!("missing file: {}", p.display())));
    }
    Ok(())
}

/// Loaded ONNX session ready for inference.
pub struct OnnxSession {
    session: Session,
    input_name: String,
    output_name: String,
}

impl OnnxSession {
    pub fn load(onnx_path: &Path) -> Result<Self, AlignError> {
        let mut builder = Session::builder()
            .map_err(|e| AlignError::ModelLoad(format!("session builder: {}", e)))?
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(|e| AlignError::ModelLoad(format!("opt level: {}", e)))?;

        // Explicitly register execution providers so ORT doesn't auto-pick whatever
        // is compiled into the downloaded binary (on macOS this includes CoreML,
        // which triggers "Context leak / CoreAnalytics returned false" errors and
        // very slow first-run model compilation when the CoreML cache is cold).
        #[cfg(feature = "coreml")]
        {
            use ort::execution_providers::CoreMLExecutionProvider;
            builder = builder
                .with_execution_providers([CoreMLExecutionProvider::default().build()])
                .map_err(|e| AlignError::ModelLoad(format!("coreml ep: {}", e)))?;
        }
        #[cfg(feature = "cuda")]
        {
            use ort::execution_providers::CUDAExecutionProvider;
            builder = builder
                .with_execution_providers([CUDAExecutionProvider::default().build()])
                .map_err(|e| AlignError::ModelLoad(format!("cuda ep: {}", e)))?;
        }
        // When no accelerated EP is requested, lock explicitly to CPU so the ORT
        // runtime cannot fall back to auto-detected platform EPs (CoreML on macOS).
        #[cfg(not(any(feature = "coreml", feature = "cuda")))]
        {
            use ort::execution_providers::CPUExecutionProvider;
            builder = builder
                .with_execution_providers([CPUExecutionProvider::default().build()])
                .map_err(|e| AlignError::ModelLoad(format!("cpu ep: {}", e)))?;
        }

        let session = builder
            .commit_from_file(onnx_path)
            .map_err(|e| AlignError::ModelLoad(format!("commit_from_file: {}", e)))?;

        let input_name = session
            .inputs()
            .first()
            .map(|i| i.name().to_string())
            .unwrap_or_else(|| "input_values".to_string());
        let output_name = session
            .outputs()
            .first()
            .map(|o| o.name().to_string())
            .unwrap_or_else(|| "logits".to_string());

        Ok(Self {
            session,
            input_name,
            output_name,
        })
    }

    /// Run the encoder on `samples` (16 kHz mono, already preprocessed).
    /// Returns logits shaped `[T_frames, vocab_size]`.
    pub fn infer(&mut self, samples: &[f32]) -> Result<Array2<f32>, AlignError> {
        let n = samples.len();
        if n == 0 {
            return Err(AlignError::Inference("empty input samples".into()));
        }
        let input = Array2::from_shape_vec((1, n), samples.to_vec())
            .map_err(|e| AlignError::Inference(format!("reshape input: {}", e)))?;
        let tensor = Tensor::from_array(input)
            .map_err(|e| AlignError::Inference(format!("input tensor: {}", e)))?;

        let outputs = self
            .session
            .run(ort::inputs![self.input_name.as_str() => tensor])
            .map_err(|e| AlignError::Inference(format!("session run: {}", e)))?;

        let output = outputs
            .get(self.output_name.as_str())
            .ok_or_else(|| {
                AlignError::Inference(format!("output '{}' missing", self.output_name))
            })?;
        let view = output
            .try_extract_array::<f32>()
            .map_err(|e| AlignError::Inference(format!("extract logits: {}", e)))?;

        // Expected shape: [1, T, V]. Drop the batch dim.
        let shape = view.shape();
        if shape.len() != 3 || shape[0] != 1 {
            return Err(AlignError::Inference(format!(
                "unexpected logits shape: {:?}",
                shape
            )));
        }
        let (t, v) = (shape[1], shape[2]);
        let slice = view
            .as_slice()
            .ok_or_else(|| AlignError::Inference("non-contiguous logits".into()))?;
        Array2::from_shape_vec((t, v), slice.to_vec())
            .map_err(|e| AlignError::Inference(format!("reshape logits: {}", e)))
    }
}

/// Apply log-softmax across the vocab axis of `logits` ([T, V] → [T, V]).
pub fn log_softmax_rows(logits: ArrayView2<f32>) -> Array2<f32> {
    let (t, v) = (logits.shape()[0], logits.shape()[1]);
    let mut out = Array2::<f32>::zeros((t, v));
    for (row_in, mut row_out) in logits.outer_iter().zip(out.outer_iter_mut()) {
        let max = row_in.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        let mut sum = 0.0_f32;
        for (&x, y) in row_in.iter().zip(row_out.iter_mut()) {
            let e = (x - max).exp();
            *y = e;
            sum += e;
        }
        let log_sum = sum.ln();
        for (&x, y) in row_in.iter().zip(row_out.iter_mut()) {
            *y = x - max - log_sum;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ndarray::Array2;

    #[test]
    fn log_softmax_sums_to_zero_in_logspace() {
        let logits = Array2::from_shape_vec((1, 3), vec![1.0, 2.0, 3.0]).unwrap();
        let lp = log_softmax_rows(logits.view());
        let row: f32 = lp.row(0).iter().map(|x| x.exp()).sum();
        assert!((row - 1.0).abs() < 1e-5);
    }
}
