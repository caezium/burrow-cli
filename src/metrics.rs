//! CLI-side metrics history + digest, leak/runaway alerts, and the low-disk clean trigger.
//!
//! Pure aggregation (`digest`, `should_autoclean`, `sample_from_status`, `alerts_from_status`)
//! is tested directly. Samples persist as JSON-lines under `$BURROW_SNAPSHOT_DIR` (default
//! `~/.burrow/snapshots`) so `burrow digest` can summarize history the GUI's DB also tracks.

use crate::output::Failure;
use crate::platform;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Sample {
    pub ts: i64,
    pub health_score: i64,
    pub cpu_usage: f64,
    pub mem_used_percent: f64,
    pub disk_used_percent: f64,
}

/// Extract a reduced Sample from the engine's `status --json` output.
pub fn sample_from_status(json: &str, ts: i64) -> Result<Sample, String> {
    let v: Value = serde_json::from_str(json).map_err(|e| format!("status parse: {e}"))?;
    let health = v.get("health_score").and_then(Value::as_i64).unwrap_or(-1);
    let cpu = v
        .get("cpu")
        .and_then(|c| c.get("usage"))
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let mem = v
        .get("memory")
        .and_then(|m| m.get("used_percent"))
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let disk = v
        .get("disks")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|d| d.get("used_percent").and_then(Value::as_f64))
                .fold(0.0_f64, f64::max)
        })
        .unwrap_or(0.0);
    Ok(Sample {
        ts,
        health_score: health,
        cpu_usage: cpu,
        mem_used_percent: mem,
        disk_used_percent: disk,
    })
}

/// Leak/runaway alerts surfaced by the engine (its `process_alerts` array).
pub fn alerts_from_status(json: &str) -> Result<Vec<Value>, String> {
    let v: Value = serde_json::from_str(json).map_err(|e| format!("status parse: {e}"))?;
    Ok(v.get("process_alerts")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default())
}

#[derive(Debug, Serialize, PartialEq)]
pub struct Digest {
    pub count: usize,
    pub span_secs: i64,
    pub health_min: i64,
    pub health_avg: f64,
    pub cpu_peak: f64,
    pub cpu_avg: f64,
    pub mem_peak: f64,
    pub disk_max_used: f64,
}

/// Aggregate samples into a digest (None if empty).
pub fn digest(samples: &[Sample]) -> Option<Digest> {
    if samples.is_empty() {
        return None;
    }
    let n = samples.len();
    let tmin = samples.iter().map(|s| s.ts).min().unwrap();
    let tmax = samples.iter().map(|s| s.ts).max().unwrap();
    Some(Digest {
        count: n,
        span_secs: tmax - tmin,
        health_min: samples.iter().map(|s| s.health_score).min().unwrap(),
        health_avg: samples.iter().map(|s| s.health_score as f64).sum::<f64>() / n as f64,
        cpu_peak: samples.iter().map(|s| s.cpu_usage).fold(f64::MIN, f64::max),
        cpu_avg: samples.iter().map(|s| s.cpu_usage).sum::<f64>() / n as f64,
        mem_peak: samples
            .iter()
            .map(|s| s.mem_used_percent)
            .fold(f64::MIN, f64::max),
        disk_max_used: samples
            .iter()
            .map(|s| s.disk_used_percent)
            .fold(f64::MIN, f64::max),
    })
}

/// Storage-Sense-style trigger: clean when disk usage reaches the threshold percent.
pub fn should_autoclean(disk_used_percent: f64, threshold_percent: f64) -> bool {
    disk_used_percent >= threshold_percent
}

pub fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn store_file() -> PathBuf {
    platform::snapshot_dir().join("samples.jsonl")
}

/// Append a sample to the JSON-lines store.
pub fn append_sample(s: &Sample) -> Result<(), Failure> {
    use std::io::Write;
    let f = store_file();
    if let Some(parent) = f.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| Failure::io(format!("cannot create {}", parent.display()), &e))?;
    }
    let line = serde_json::to_string(s).map_err(|e| Failure::error(e.to_string()))? + "\n";
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&f)
        .map_err(|e| Failure::io(format!("cannot open {}", f.display()), &e))?;
    file.write_all(line.as_bytes())
        .map_err(|e| Failure::io(format!("cannot write {}", f.display()), &e))?;
    Ok(())
}

/// Load samples with `ts >= since_ts`.
pub fn load_samples(since_ts: i64) -> Vec<Sample> {
    let Ok(content) = std::fs::read_to_string(store_file()) else {
        return Vec::new();
    };
    content
        .lines()
        .filter_map(|l| serde_json::from_str::<Sample>(l).ok())
        .filter(|s| s.ts >= since_ts)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_status_fields_and_max_disk() {
        let s = sample_from_status(
            r#"{"health_score":92,"cpu":{"usage":12.4},"memory":{"used_percent":40.0},
                "disks":[{"used_percent":80.0},{"used_percent":84.0}]}"#,
            1000,
        )
        .unwrap();
        assert_eq!(s.health_score, 92);
        assert!((s.cpu_usage - 12.4).abs() < 1e-9);
        assert!((s.disk_used_percent - 84.0).abs() < 1e-9); // max across disks
    }

    #[test]
    fn digest_aggregates() {
        let s = vec![
            Sample {
                ts: 100,
                health_score: 90,
                cpu_usage: 10.0,
                mem_used_percent: 40.0,
                disk_used_percent: 80.0,
            },
            Sample {
                ts: 200,
                health_score: 80,
                cpu_usage: 50.0,
                mem_used_percent: 60.0,
                disk_used_percent: 84.0,
            },
        ];
        let d = digest(&s).unwrap();
        assert_eq!(d.count, 2);
        assert_eq!(d.span_secs, 100);
        assert_eq!(d.health_min, 80);
        assert!((d.cpu_peak - 50.0).abs() < 1e-9);
        assert!((d.disk_max_used - 84.0).abs() < 1e-9);
    }

    #[test]
    fn empty_digest_is_none() {
        assert!(digest(&[]).is_none());
    }

    #[test]
    fn autoclean_trigger() {
        assert!(should_autoclean(85.0, 80.0));
        assert!(should_autoclean(80.0, 80.0));
        assert!(!should_autoclean(70.0, 80.0));
    }

    #[test]
    fn extracts_alerts() {
        let v = alerts_from_status(r#"{"process_alerts":[{"pid":1,"name":"leaky"}]}"#).unwrap();
        assert_eq!(v.len(), 1);
        assert!(alerts_from_status("{}").unwrap().is_empty());
    }

    #[test]
    fn store_roundtrip_and_since_filter() {
        let dir = std::env::temp_dir().join(format!("burrow_snap_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::env::set_var("BURROW_SNAPSHOT_DIR", &dir);
        append_sample(&Sample {
            ts: 500,
            health_score: 88,
            cpu_usage: 5.0,
            mem_used_percent: 30.0,
            disk_used_percent: 50.0,
        })
        .unwrap();
        assert_eq!(load_samples(0).len(), 1);
        assert_eq!(load_samples(0)[0].health_score, 88);
        assert_eq!(load_samples(1000).len(), 0); // filtered by since
        std::env::remove_var("BURROW_SNAPSHOT_DIR");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
