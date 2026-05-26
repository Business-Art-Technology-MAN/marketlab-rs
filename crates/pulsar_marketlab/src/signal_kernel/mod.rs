//! Structural core for Layer 3 [`SignalKernel`] (SRD 3 vector calculation surface).
//!
//! Responsibilities modeled here:
//! - **SIMD indicator array wrappers** — `vector-ta` MACD / ADX with explicit kernel dispatch.
//! - **Covariance distance closures** — trailing-window statistical distances between series.
//! - **Grade-channel hyper-rotors** — Clifford/GA multivector decomposition and per-grade rotation.
//! - **Signal projection matrix** — dense per-tick downstream operator ledger (`ndarray::Array2`).

use std::collections::BTreeMap;
use std::fmt;

use ndarray::Array2;
use serde::{Deserialize, Serialize};
use vector_ta::indicators::adx::{adx_with_kernel, AdxInput, AdxParams};
use vector_ta::indicators::macd::{macd_with_kernel, MacdInput, MacdParams};
use vector_ta::utilities::enums::Kernel;

use crate::execution_engine::ExecutionEngine;

// -----------------------------------------------------------------------------
// Layer 2 abstraction
// -----------------------------------------------------------------------------

/// Read-only surface Layer 2 exposes to the signal kernel.
pub trait ExecutionEngineFeed {
    fn tracking_matrix(&self) -> &Array2<f64>;
    fn master_timeline_len(&self) -> usize;
}

impl ExecutionEngineFeed for ExecutionEngine {
    fn tracking_matrix(&self) -> &Array2<f64> {
        self.tracking().values()
    }

    fn master_timeline_len(&self) -> usize {
        self.master_len()
    }
}

// -----------------------------------------------------------------------------
// Errors
// -----------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum SignalKernelError {
    EmptyMasterTimeline,
    EmptySeries,
    SeriesLengthMismatch { expected: usize, got: usize },
    WindowTooShort { window: usize, available: usize },
    TimeIndexOutOfRange { t: usize, len: usize },
    SingularCovariance,
    InvalidEmbeddingDim { dim: usize },
    EmptyGradeDecomposition,
    IndicatorMacd(String),
    IndicatorAdx(String),
    MatrixConstructionFailed,
}

impl fmt::Display for SignalKernelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SignalKernelError::EmptyMasterTimeline => {
                write!(f, "master timeline must contain at least one tick")
            }
            SignalKernelError::EmptySeries => write!(f, "input series must be non-empty"),
            SignalKernelError::SeriesLengthMismatch { expected, got } => write!(
                f,
                "series length {got} does not match master timeline {expected}"
            ),
            SignalKernelError::WindowTooShort { window, available } => write!(
                f,
                "covariance window {window} exceeds available samples {available}"
            ),
            SignalKernelError::TimeIndexOutOfRange { t, len } => {
                write!(f, "time index {t} out of range for length {len}")
            }
            SignalKernelError::SingularCovariance => {
                write!(f, "covariance matrix is singular or ill-conditioned")
            }
            SignalKernelError::InvalidEmbeddingDim { dim } => {
                write!(f, "embedding dimension must be >= 1, got {dim}")
            }
            SignalKernelError::EmptyGradeDecomposition => {
                write!(f, "grade decomposition produced no channels")
            }
            SignalKernelError::IndicatorMacd(msg) => write!(f, "MACD indicator failed: {msg}"),
            SignalKernelError::IndicatorAdx(msg) => write!(f, "ADX indicator failed: {msg}"),
            SignalKernelError::MatrixConstructionFailed => {
                write!(f, "ndarray could not assume row-major shape")
            }
        }
    }
}

impl std::error::Error for SignalKernelError {}

// -----------------------------------------------------------------------------
// SIMD kernel dispatch
// -----------------------------------------------------------------------------

/// Explicit SIMD backend selection forwarded to `vector-ta`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SimdKernelChoice {
    #[default]
    Auto,
    Scalar,
    Avx2,
    Avx512,
}

impl SimdKernelChoice {
    pub fn to_vector_ta_kernel(self) -> Kernel {
        match self {
            SimdKernelChoice::Auto => Kernel::Auto,
            SimdKernelChoice::Scalar => Kernel::Scalar,
            SimdKernelChoice::Avx2 => Kernel::Avx2,
            SimdKernelChoice::Avx512 => Kernel::Avx512,
        }
    }
}

// -----------------------------------------------------------------------------
// vector-ta SIMD indicator array wrappers
// -----------------------------------------------------------------------------

/// MACD triple output aligned to the master timeline length.
#[derive(Clone, Debug, PartialEq)]
pub struct MacdIndicatorArray {
    pub macd: Vec<f64>,
    pub signal: Vec<f64>,
    pub hist: Vec<f64>,
}

impl MacdIndicatorArray {
    /// Compute MACD via `vector-ta` with explicit SIMD kernel dispatch.
    pub fn compute(
        close: &[f64],
        params: MacdParams,
        kernel: SimdKernelChoice,
    ) -> Result<Self, SignalKernelError> {
        if close.is_empty() {
            return Err(SignalKernelError::EmptySeries);
        }
        let input = MacdInput::from_slice(close, params);
        let output = macd_with_kernel(&input, kernel.to_vector_ta_kernel())
            .map_err(|e| SignalKernelError::IndicatorMacd(e.to_string()))?;
        Ok(Self {
            macd: output.macd,
            signal: output.signal,
            hist: output.hist,
        })
    }

    pub fn len(&self) -> usize {
        self.macd.len()
    }

    pub fn is_empty(&self) -> bool {
        self.macd.is_empty()
    }
}

/// ADX output aligned to the master timeline length.
#[derive(Clone, Debug, PartialEq)]
pub struct AdxIndicatorArray {
    pub values: Vec<f64>,
}

impl AdxIndicatorArray {
    /// Compute ADX from HLC slices via `vector-ta` with explicit SIMD kernel dispatch.
    pub fn compute(
        high: &[f64],
        low: &[f64],
        close: &[f64],
        params: AdxParams,
        kernel: SimdKernelChoice,
    ) -> Result<Self, SignalKernelError> {
        if high.is_empty() || low.is_empty() || close.is_empty() {
            return Err(SignalKernelError::EmptySeries);
        }
        if high.len() != low.len() || high.len() != close.len() {
            return Err(SignalKernelError::SeriesLengthMismatch {
                expected: high.len(),
                got: low.len(),
            });
        }
        let input = AdxInput::from_slices(high, low, close, params);
        let output = adx_with_kernel(&input, kernel.to_vector_ta_kernel())
            .map_err(|e| SignalKernelError::IndicatorAdx(e.to_string()))?;
        Ok(Self {
            values: output.values,
        })
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

// -----------------------------------------------------------------------------
// Statistical covariance distance closures
// -----------------------------------------------------------------------------

/// Distance metric applied inside a trailing covariance window.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CovarianceMetric {
    /// Mahalanobis distance between the current bivariate sample and the window mean.
    Mahalanobis,
    /// Frobenius norm of the normalized covariance residual matrix.
    NormalizedFrobenius,
    /// Symmetric KL divergence between two window Gaussians (univariate proxy).
    SymmetricKl,
}

/// Specialized closure: `(series_a, series_b, t) -> distance` under a trailing covariance window.
#[derive(Clone, Debug, PartialEq)]
pub struct CovarianceDistanceClosure {
    window: usize,
    metric: CovarianceMetric,
    /// Minimum absolute denominator guard for matrix inversion.
    epsilon: f64,
}

impl CovarianceDistanceClosure {
    pub fn new(window: usize, metric: CovarianceMetric) -> Result<Self, SignalKernelError> {
        if window < 2 {
            return Err(SignalKernelError::WindowTooShort {
                window,
                available: window,
            });
        }
        Ok(Self {
            window,
            metric,
            epsilon: 1e-12,
        })
    }

    pub fn window(&self) -> usize {
        self.window
    }

    pub fn metric(&self) -> CovarianceMetric {
        self.metric
    }

    /// Evaluate the closure at master index `t` (uses samples `[t-window+1 ..= t]`).
    pub fn evaluate(&self, series_a: &[f64], series_b: &[f64], t: usize) -> Result<f64, SignalKernelError> {
        if series_a.len() != series_b.len() {
            return Err(SignalKernelError::SeriesLengthMismatch {
                expected: series_a.len(),
                got: series_b.len(),
            });
        }
        if t >= series_a.len() {
            return Err(SignalKernelError::TimeIndexOutOfRange {
                t,
                len: series_a.len(),
            });
        }
        if t + 1 < self.window {
            return Err(SignalKernelError::WindowTooShort {
                window: self.window,
                available: t + 1,
            });
        }

        let start = t + 1 - self.window;
        let slice_a = &series_a[start..=t];
        let slice_b = &series_b[start..=t];

        match self.metric {
            CovarianceMetric::Mahalanobis => {
                mahalanobis_bivariate(slice_a, slice_b, self.epsilon)
            }
            CovarianceMetric::NormalizedFrobenius => {
                Ok(normalized_frobenius_covariance_distance(slice_a, slice_b))
            }
            CovarianceMetric::SymmetricKl => Ok(symmetric_kl_univariate(slice_a, slice_b, self.epsilon)),
        }
    }
}

fn mean(xs: &[f64]) -> f64 {
    xs.iter().sum::<f64>() / xs.len() as f64
}

fn variance(xs: &[f64], mu: f64) -> f64 {
    xs.iter().map(|x| (x - mu).powi(2)).sum::<f64>() / xs.len() as f64
}

fn covariance(xs: &[f64], ys: &[f64], mu_x: f64, mu_y: f64) -> f64 {
    xs.iter()
        .zip(ys.iter())
        .map(|(x, y)| (x - mu_x) * (y - mu_y))
        .sum::<f64>()
        / xs.len() as f64
}

fn mahalanobis_bivariate(a: &[f64], b: &[f64], epsilon: f64) -> Result<f64, SignalKernelError> {
    let mu_a = mean(a);
    let mu_b = mean(b);
    let var_a = variance(a, mu_a).max(epsilon);
    let var_b = variance(b, mu_b).max(epsilon);
    let cov = covariance(a, b, mu_a, mu_b);

    let det = (var_a * var_b - cov * cov).abs();
    if det < epsilon {
        return Err(SignalKernelError::SingularCovariance);
    }

    let delta_a = *a.last().expect("non-empty window") - mu_a;
    let delta_b = *b.last().expect("non-empty window") - mu_b;

    let inv_00 = var_b / det;
    let inv_01 = -cov / det;
    let inv_11 = var_a / det;

    let d0 = inv_00 * delta_a + inv_01 * delta_b;
    let d1 = inv_01 * delta_a + inv_11 * delta_b;
    Ok((delta_a * d0 + delta_b * d1).max(0.0).sqrt())
}

fn normalized_frobenius_covariance_distance(a: &[f64], b: &[f64]) -> f64 {
    let mu_a = mean(a);
    let mu_b = mean(b);
    let var_a = variance(a, mu_a);
    let var_b = variance(b, mu_b);
    let cov = covariance(a, b, mu_a, mu_b);

    let norm = (var_a.abs() + var_b.abs() + cov.abs() + 1e-12).sqrt();
    let res_aa = var_a - 1.0;
    let res_bb = var_b - 1.0;
    let res_ab = cov;
    ((res_aa * res_aa + res_bb * res_bb + 2.0 * res_ab * res_ab) / norm).sqrt()
}

fn symmetric_kl_univariate(a: &[f64], b: &[f64], epsilon: f64) -> f64 {
    let mu_a = mean(a);
    let mu_b = mean(b);
    let var_a = variance(a, mu_a).max(epsilon);
    let var_b = variance(b, mu_b).max(epsilon);

    let kl_ab = (var_b / var_a).ln() + (var_a + (mu_a - mu_b).powi(2)) / var_b - 1.0;
    let kl_ba = (var_a / var_b).ln() + (var_b + (mu_a - mu_b).powi(2)) / var_a - 1.0;
    0.5 * (kl_ab + kl_ba)
}

// -----------------------------------------------------------------------------
// Clifford / GA grade-channel decomposition & hyper-rotors
// -----------------------------------------------------------------------------

/// Algebra grade channel — multivector components are never mixed across grades.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GradeChannel {
    /// Grade-0 scalar (1 component).
    Scalar,
    /// Grade-1 vector (`embedding_dim` components).
    Vector,
    /// Grade-2 pairwise channel (`C(n,2)` components).
    GradeTwo,
    /// Grade-3 trivector (`C(n,3)` components, present when `n >= 3`).
    Trivector,
    /// Grade-n pseudoscalar (1 component).
    Pseudoscalar,
}

impl GradeChannel {
    /// Component count for this grade in `Cl(n)`.
    pub fn component_count(self, embedding_dim: usize) -> usize {
        match self {
            GradeChannel::Scalar | GradeChannel::Pseudoscalar => 1,
            GradeChannel::Vector => embedding_dim,
            GradeChannel::GradeTwo => binomial(embedding_dim, 2),
            GradeChannel::Trivector => binomial(embedding_dim, 3),
        }
    }

    /// Active grade channels for a given embedding dimension.
    pub fn channels_for_dim(embedding_dim: usize) -> Vec<GradeChannel> {
        if embedding_dim == 0 {
            return Vec::new();
        }
        let mut channels = vec![GradeChannel::Scalar, GradeChannel::Vector];
        if embedding_dim >= 2 {
            channels.push(GradeChannel::GradeTwo);
        }
        if embedding_dim >= 3 {
            channels.push(GradeChannel::Trivector);
        }
        if embedding_dim >= 1 {
            channels.push(GradeChannel::Pseudoscalar);
        }
        channels
    }
}

fn binomial(n: usize, k: usize) -> usize {
    if k > n {
        return 0;
    }
    if k == 0 || k == n {
        return 1;
    }
    let k = k.min(n - k);
    let mut result = 1usize;
    for i in 0..k {
        result = result * (n - i) / (i + 1);
    }
    result
}

/// Hyper-dimensional multivector stored strictly by grade channel.
#[derive(Clone, Debug, PartialEq)]
pub struct GradeChannelDecomposition {
    pub embedding_dim: usize,
    pub channels: BTreeMap<GradeChannel, Vec<f64>>,
}

impl GradeChannelDecomposition {
    pub fn zeros(embedding_dim: usize) -> Result<Self, SignalKernelError> {
        if embedding_dim == 0 {
            return Err(SignalKernelError::InvalidEmbeddingDim { dim: 0 });
        }
        let mut channels = BTreeMap::new();
        for grade in GradeChannel::channels_for_dim(embedding_dim) {
            channels.insert(grade, vec![0.0; grade.component_count(embedding_dim)]);
        }
        if channels.is_empty() {
            return Err(SignalKernelError::EmptyGradeDecomposition);
        }
        Ok(Self {
            embedding_dim,
            channels,
        })
    }

    /// Lift a flat feature vector into grade channels (scalar = mean, vector = values, grade-two = pairwise products).
    pub fn from_feature_vector(values: &[f64]) -> Result<Self, SignalKernelError> {
        if values.is_empty() {
            return Err(SignalKernelError::EmptySeries);
        }
        let embedding_dim = values.len();
        let mut decomp = Self::zeros(embedding_dim)?;

        if let Some(scalar) = decomp.channels.get_mut(&GradeChannel::Scalar) {
            scalar[0] = mean(values);
        }
        if let Some(vector) = decomp.channels.get_mut(&GradeChannel::Vector) {
            vector.copy_from_slice(values);
        }
        if let Some(grade_two) = decomp.channels.get_mut(&GradeChannel::GradeTwo) {
            let mut idx = 0usize;
            for i in 0..embedding_dim {
                for j in (i + 1)..embedding_dim {
                    grade_two[idx] = values[i] * values[j];
                    idx += 1;
                }
            }
        }
        if embedding_dim >= 3 {
            if let Some(trivector) = decomp.channels.get_mut(&GradeChannel::Trivector) {
                let mut idx = 0usize;
                for i in 0..embedding_dim {
                    for j in (i + 1)..embedding_dim {
                        for k in (j + 1)..embedding_dim {
                            trivector[idx] = values[i] * values[j] * values[k];
                            idx += 1;
                        }
                    }
                }
            }
        }
        if let Some(pseudo) = decomp.channels.get_mut(&GradeChannel::Pseudoscalar) {
            pseudo[0] = values.iter().product();
        }

        Ok(decomp)
    }

    pub fn channel(&self, grade: GradeChannel) -> Option<&[f64]> {
        self.channels.get(&grade).map(|v| v.as_slice())
    }

    pub fn channel_mut(&mut self, grade: GradeChannel) -> Option<&mut [f64]> {
        self.channels.get_mut(&grade).map(|v| v.as_mut_slice())
    }
}

/// Even-grade rotor parameters per grade-two plane, keyed by grade channel.
#[derive(Clone, Debug, PartialEq)]
pub struct HyperRotor {
    pub embedding_dim: usize,
    /// Per-grade rotation angles (radians) applied within that grade channel only.
    pub grade_angles: BTreeMap<GradeChannel, Vec<f64>>,
}

impl HyperRotor {
    pub fn identity(embedding_dim: usize) -> Result<Self, SignalKernelError> {
        if embedding_dim == 0 {
            return Err(SignalKernelError::InvalidEmbeddingDim { dim: 0 });
        }
        let mut grade_angles = BTreeMap::new();
        for grade in GradeChannel::channels_for_dim(embedding_dim) {
            let count = grade.component_count(embedding_dim);
            grade_angles.insert(grade, vec![0.0; count]);
        }
        Ok(Self {
            embedding_dim,
            grade_angles,
        })
    }

    /// Construct a rotor from per-grade angle specifications (strictly grade-local).
    pub fn from_grade_angles(
        embedding_dim: usize,
        angles: BTreeMap<GradeChannel, Vec<f64>>,
    ) -> Result<Self, SignalKernelError> {
        if embedding_dim == 0 {
            return Err(SignalKernelError::InvalidEmbeddingDim { dim: 0 });
        }
        for grade in GradeChannel::channels_for_dim(embedding_dim) {
            let expected = grade.component_count(embedding_dim);
            match angles.get(&grade) {
                Some(v) if v.len() == expected => {}
                Some(v) => {
                    return Err(SignalKernelError::SeriesLengthMismatch {
                        expected,
                        got: v.len(),
                    });
                }
                None => {
                    return Err(SignalKernelError::EmptyGradeDecomposition);
                }
            }
        }
        Ok(Self {
            embedding_dim,
            grade_angles: angles,
        })
    }

    /// Apply hyper-dimensional rotor transforms decomposed strictly by grade channel.
    pub fn apply(&self, multivector: &mut GradeChannelDecomposition) -> Result<(), SignalKernelError> {
        if multivector.embedding_dim != self.embedding_dim {
            return Err(SignalKernelError::SeriesLengthMismatch {
                expected: self.embedding_dim,
                got: multivector.embedding_dim,
            });
        }

        for (grade, angles) in &self.grade_angles {
            let components = multivector
                .channel_mut(*grade)
                .ok_or(SignalKernelError::EmptyGradeDecomposition)?;
            if components.len() != angles.len() {
                return Err(SignalKernelError::SeriesLengthMismatch {
                    expected: components.len(),
                    got: angles.len(),
                });
            }
            apply_grade_channel_rotation(components, angles, *grade);
        }
        Ok(())
    }
}

/// Rotate components within a single grade channel (Givens-style plane rotations).
fn apply_grade_channel_rotation(components: &mut [f64], angles: &[f64], grade: GradeChannel) {
    match grade {
        GradeChannel::Scalar | GradeChannel::Pseudoscalar => {
            if !components.is_empty() {
                let theta = angles[0];
                let (c, s) = (theta.cos(), theta.sin());
                components[0] = c * components[0] - s * components[0];
            }
        }
        GradeChannel::Vector | GradeChannel::GradeTwo | GradeChannel::Trivector => {
            let n = components.len();
            if n < 2 {
                if n == 1 {
                    let theta = angles[0];
                    components[0] *= theta.cos();
                }
                return;
            }
            for (plane, theta) in angles.iter().enumerate() {
                let i = plane % n;
                let j = (plane + 1) % n;
                let (c, s) = (theta.cos(), theta.sin());
                let vi = components[i];
                let vj = components[j];
                components[i] = c * vi - s * vj;
                components[j] = s * vi + c * vj;
            }
        }
    }
}

// -----------------------------------------------------------------------------
// Signal projection matrix
// -----------------------------------------------------------------------------

/// Dense per-tick signal operator ledger (rows = master timeline, cols = projection channels).
#[derive(Clone, Debug, PartialEq)]
pub struct SignalProjectionMatrix {
    channels: BTreeMap<String, usize>,
    values: Array2<f64>,
}

impl SignalProjectionMatrix {
    pub fn new(master_len: usize, channel_ids: &[String]) -> Result<Self, SignalKernelError> {
        if master_len == 0 {
            return Err(SignalKernelError::EmptyMasterTimeline);
        }
        let mut channels = BTreeMap::new();
        for (idx, id) in channel_ids.iter().enumerate() {
            channels.insert(id.clone(), idx);
        }
        Ok(Self {
            channels,
            values: Array2::zeros((master_len, channel_ids.len().max(1))),
        })
    }

    pub fn rows(&self) -> usize {
        self.values.nrows()
    }

    pub fn cols(&self) -> usize {
        self.values.ncols()
    }

    pub fn channels(&self) -> &BTreeMap<String, usize> {
        &self.channels
    }

    pub fn values(&self) -> &Array2<f64> {
        &self.values
    }

    pub fn values_mut(&mut self) -> &mut Array2<f64> {
        &mut self.values
    }

    pub fn set_channel(&mut self, t: usize, channel: &str, value: f64) -> Result<(), SignalKernelError> {
        let col = self
            .channels
            .get(channel)
            .copied()
            .ok_or(SignalKernelError::SeriesLengthMismatch {
                expected: self.channels.len(),
                got: 0,
            })?;
        if t >= self.rows() {
            return Err(SignalKernelError::TimeIndexOutOfRange {
                t,
                len: self.rows(),
            });
        }
        self.values[(t, col)] = value;
        Ok(())
    }

    pub fn bind_indicator_row(&mut self, channel: &str, series: &[f64]) -> Result<(), SignalKernelError> {
        let col = self
            .channels
            .get(channel)
            .copied()
            .ok_or(SignalKernelError::SeriesLengthMismatch {
                expected: self.channels.len(),
                got: 0,
            })?;
        if series.len() != self.rows() {
            return Err(SignalKernelError::SeriesLengthMismatch {
                expected: self.rows(),
                got: series.len(),
            });
        }
        for (t, &v) in series.iter().enumerate() {
            self.values[(t, col)] = v;
        }
        Ok(())
    }
}

// -----------------------------------------------------------------------------
// SignalKernel orchestrator
// -----------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct SignalKernel {
    master_len: usize,
    projection: SignalProjectionMatrix,
    simd_kernel: SimdKernelChoice,
    embedding_dim: usize,
    rotor: HyperRotor,
    covariance_closure: Option<CovarianceDistanceClosure>,
}

impl SignalKernel {
    pub fn bootstrap(
        engine: &impl ExecutionEngineFeed,
        channel_ids: &[String],
        embedding_dim: usize,
    ) -> Result<Self, SignalKernelError> {
        let master_len = engine.master_timeline_len();
        Ok(Self {
            master_len,
            projection: SignalProjectionMatrix::new(master_len, channel_ids)?,
            simd_kernel: SimdKernelChoice::Auto,
            embedding_dim,
            rotor: HyperRotor::identity(embedding_dim)?,
            covariance_closure: None,
        })
    }

    pub fn master_len(&self) -> usize {
        self.master_len
    }

    pub fn projection(&self) -> &SignalProjectionMatrix {
        &self.projection
    }

    pub fn projection_mut(&mut self) -> &mut SignalProjectionMatrix {
        &mut self.projection
    }

    pub fn simd_kernel(&self) -> SimdKernelChoice {
        self.simd_kernel
    }

    pub fn set_simd_kernel(&mut self, kernel: SimdKernelChoice) {
        self.simd_kernel = kernel;
    }

    pub fn set_covariance_closure(&mut self, closure: CovarianceDistanceClosure) {
        self.covariance_closure = Some(closure);
    }

    pub fn covariance_closure(&self) -> Option<&CovarianceDistanceClosure> {
        self.covariance_closure.as_ref()
    }

    pub fn rotor(&self) -> &HyperRotor {
        &self.rotor
    }

    pub fn rotor_mut(&mut self) -> &mut HyperRotor {
        &mut self.rotor
    }

    /// Compute MACD on a close-price series and bind outputs into projection channels.
    pub fn ingest_macd(
        &mut self,
        close: &[f64],
        params: MacdParams,
        channel_macd: &str,
        channel_signal: &str,
        channel_hist: &str,
    ) -> Result<MacdIndicatorArray, SignalKernelError> {
        if close.len() != self.master_len {
            return Err(SignalKernelError::SeriesLengthMismatch {
                expected: self.master_len,
                got: close.len(),
            });
        }
        let array = MacdIndicatorArray::compute(close, params, self.simd_kernel)?;
        self.projection.bind_indicator_row(channel_macd, &array.macd)?;
        self.projection
            .bind_indicator_row(channel_signal, &array.signal)?;
        self.projection
            .bind_indicator_row(channel_hist, &array.hist)?;
        Ok(array)
    }

    /// Compute ADX on HLC series and bind into a projection channel.
    pub fn ingest_adx(
        &mut self,
        high: &[f64],
        low: &[f64],
        close: &[f64],
        params: AdxParams,
        channel: &str,
    ) -> Result<AdxIndicatorArray, SignalKernelError> {
        if close.len() != self.master_len {
            return Err(SignalKernelError::SeriesLengthMismatch {
                expected: self.master_len,
                got: close.len(),
            });
        }
        let array = AdxIndicatorArray::compute(high, low, close, params, self.simd_kernel)?;
        self.projection.bind_indicator_row(channel, &array.values)?;
        Ok(array)
    }

    /// Lift tick features, apply grade-channel rotor, and write scalar projection at `t`.
    pub fn project_rotor_at(
        &mut self,
        t: usize,
        features: &[f64],
        output_channel: &str,
    ) -> Result<GradeChannelDecomposition, SignalKernelError> {
        if t >= self.master_len {
            return Err(SignalKernelError::TimeIndexOutOfRange {
                t,
                len: self.master_len,
            });
        }
        let mut decomp = GradeChannelDecomposition::from_feature_vector(features)?;
        self.rotor.apply(&mut decomp)?;
        let scalar = decomp
            .channel(GradeChannel::Scalar)
            .and_then(|s| s.first())
            .copied()
            .unwrap_or(0.0);
        self.projection.set_channel(t, output_channel, scalar)?;
        Ok(decomp)
    }

    /// Evaluate the configured covariance distance closure at tick `t`.
    pub fn covariance_distance_at(
        &self,
        series_a: &[f64],
        series_b: &[f64],
        t: usize,
    ) -> Result<f64, SignalKernelError> {
        let closure = self
            .covariance_closure
            .as_ref()
            .ok_or(SignalKernelError::EmptyGradeDecomposition)?;
        closure.evaluate(series_a, series_b, t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution_engine::ExecutionEngine;
    use crate::trading_stage::TradingStage;
    use ndarray::Array2;

    fn sample_close(n: usize) -> Vec<f64> {
        (0..n).map(|i| 100.0 + (i as f64) * 0.5 + (i as f64).sin()).collect()
    }

    #[test]
    fn macd_wrapper_produces_aligned_output() {
        let close = sample_close(64);
        let out = MacdIndicatorArray::compute(&close, MacdParams::default(), SimdKernelChoice::Auto)
            .unwrap();
        assert_eq!(out.len(), close.len());
        assert!(out.hist.iter().any(|v| v.is_finite()));
    }

    #[test]
    fn adx_wrapper_produces_aligned_output() {
        let close = sample_close(64);
        let high: Vec<f64> = close.iter().map(|c| c + 1.0).collect();
        let low: Vec<f64> = close.iter().map(|c| c - 1.0).collect();
        let out = AdxIndicatorArray::compute(
            &high,
            &low,
            &close,
            AdxParams::default(),
            SimdKernelChoice::Auto,
        )
        .unwrap();
        assert_eq!(out.len(), close.len());
    }

    #[test]
    fn covariance_closure_mahalanobis_is_non_negative() {
        let a = sample_close(32);
        let b: Vec<f64> = a
            .iter()
            .enumerate()
            .map(|(i, x)| x * 0.02 * (i as f64).sin() + 50.0)
            .collect();
        let closure = CovarianceDistanceClosure::new(8, CovarianceMetric::Mahalanobis).unwrap();
        let d = closure.evaluate(&a, &b, 20).unwrap();
        assert!(d >= 0.0);
    }

    #[test]
    fn grade_decomposition_preserves_vector_grade() {
        let features = vec![1.0, 2.0, 3.0];
        let decomp = GradeChannelDecomposition::from_feature_vector(&features).unwrap();
        assert_eq!(decomp.channel(GradeChannel::Vector), Some(features.as_slice()));
        assert_eq!(decomp.channel(GradeChannel::Scalar), Some([2.0].as_slice()));
    }

    #[test]
    fn hyper_rotor_transforms_without_cross_grade_mixing() {
        let features = vec![1.0, 0.0];
        let mut decomp = GradeChannelDecomposition::from_feature_vector(&features).unwrap();
        let vector_before = decomp.channel(GradeChannel::Vector).unwrap().to_vec();
        let scalar_before = decomp.channel(GradeChannel::Scalar).unwrap()[0];

        let mut angles = BTreeMap::new();
        angles.insert(GradeChannel::Vector, vec![std::f64::consts::FRAC_PI_2, 0.0]);
        angles.insert(GradeChannel::Scalar, vec![0.0]);
        angles.insert(GradeChannel::GradeTwo, vec![0.0]);
        angles.insert(GradeChannel::Pseudoscalar, vec![0.0]);

        let rotor = HyperRotor::from_grade_angles(2, angles).unwrap();
        rotor.apply(&mut decomp).unwrap();

        let vector_after = decomp.channel(GradeChannel::Vector).unwrap();
        assert_ne!(vector_before, vector_after);
        assert_eq!(
            decomp.channel(GradeChannel::Scalar).unwrap()[0],
            scalar_before
        );
    }

    #[test]
    fn kernel_bootstraps_from_execution_engine() {
        let stage = TradingStage::new(Array2::zeros((32, 3)));
        let engine = ExecutionEngine::bootstrap(
            &stage,
            &["exec.a".into()],
            1,
            10_000.0,
            &[0.0],
        )
        .unwrap();
        let kernel = SignalKernel::bootstrap(
            &engine,
            &["sig.macd".into(), "sig.adx".into()],
            3,
        )
        .unwrap();
        assert_eq!(kernel.master_len(), 32);
        assert_eq!(kernel.projection().cols(), 2);
    }

    #[test]
    fn ingest_macd_binds_projection_channels() {
        let stage = TradingStage::new(Array2::zeros((48, 1)));
        let engine = ExecutionEngine::bootstrap(&stage, &["x".into()], 1, 0.0, &[0.0]).unwrap();
        let mut kernel = SignalKernel::bootstrap(
            &engine,
            &["macd.line".into(), "macd.signal".into(), "macd.hist".into()],
            2,
        )
        .unwrap();
        let close = sample_close(48);
        kernel
            .ingest_macd(
                &close,
                MacdParams::default(),
                "macd.line",
                "macd.signal",
                "macd.hist",
            )
            .unwrap();
        assert!(kernel.projection().values()[(47, 0)].is_finite());
    }
}
