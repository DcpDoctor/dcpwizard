//! EBU R128 loudness measurement and target-normalization wiring.
//!
//! Delegates to [`postkit::loudness`], which meters a WAV or a PCM MXF in
//! process, no ffmpeg, and does the sample-domain gain for target
//! normalization (dom#1382).

pub use postkit::loudness::{
    AdjustError, DEFAULT_TRUE_PEAK_CEILING_DBTP, GainPlan, LoudnessResult, LoudnessTarget,
    adjust_loudness, measure_loudness, measure_true_peak_dbtp,
};

/// Parse a `--loudness-target` spec into a [`LoudnessTarget`].
///
/// Accepts `leqm=<db>` (ISO 21727 Leq(m), the cinema metric) or `lufs=<value>`
/// (EBU R128 integrated). Case-insensitive; `leq(m)=` is also accepted.
pub fn parse_loudness_target(spec: &str) -> Result<LoudnessTarget, String> {
    let (metric, value) = spec
        .split_once('=')
        .ok_or_else(|| format!("loudness target '{spec}' must be metric=value, e.g. leqm=85"))?;
    let v: f64 = value.trim().parse().map_err(|_| {
        format!("loudness target value '{value}' is not a number (e.g. leqm=85 or lufs=-20)")
    })?;
    match metric.trim().to_lowercase().replace("(m)", "").as_str() {
        "leqm" | "leq" => Ok(LoudnessTarget::LeqM(v)),
        "lufs" => Ok(LoudnessTarget::IntegratedLufs(v)),
        other => Err(format!(
            "unknown loudness metric '{other}' (use leqm or lufs)"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_leqm_and_lufs() {
        assert!(matches!(
            parse_loudness_target("leqm=85").unwrap(),
            LoudnessTarget::LeqM(v) if (v - 85.0).abs() < 1e-9
        ));
        assert!(matches!(
            parse_loudness_target("Leq(m)=82.5").unwrap(),
            LoudnessTarget::LeqM(v) if (v - 82.5).abs() < 1e-9
        ));
        assert!(matches!(
            parse_loudness_target("LUFS=-20").unwrap(),
            LoudnessTarget::IntegratedLufs(v) if (v + 20.0).abs() < 1e-9
        ));
    }

    #[test]
    fn rejects_bad_specs() {
        assert!(parse_loudness_target("85").is_err());
        assert!(parse_loudness_target("leqm=loud").is_err());
        assert!(parse_loudness_target("rms=-14").is_err());
    }
}
