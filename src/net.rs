//! Per-process network attribution. macOS uses the built-in `nettop` byte counters. Windows
//! currently uses a conservative `netstat -ano` + `tasklist` fallback so the command is
//! available on real Windows while an ETW/IP Helper implementation can replace it later.

use crate::platform;
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize, PartialEq)]
pub struct ProcNet {
    pub name: String,
    pub pid: u32,
    pub bytes_in: u64,
    pub bytes_out: u64,
    pub total: u64,
}

pub fn collect() -> Result<Vec<ProcNet>, String> {
    if platform::is_windows() {
        let netstat = run_command("netstat", &["-ano"])?;
        let tasklist = run_command("tasklist", &["/fo", "csv", "/nh"]).unwrap_or_default();
        Ok(parse_netstat(&netstat, &tasklist))
    } else if platform::is_macos() {
        let out = resolve_nettop().and_then(|b| run_nettop(&b))?;
        Ok(parse_nettop(&out))
    } else {
        Err("per-app network is unavailable on this platform".into())
    }
}

fn run_command(program: &str, args: &[&str]) -> Result<String, String> {
    let out = std::process::Command::new(program)
        .args(args)
        .output()
        .map_err(|e| format!("failed to run {program}: {e}"))?;
    if !out.status.success() {
        return Err(format!("{program} exited {}", out.status));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Parse `nettop -P -L 1 -x -J bytes_in,bytes_out` CSV (rows: `name.pid,bytes_in,bytes_out,`)
/// into per-process rows, ranked by total bytes descending.
pub fn parse_nettop(output: &str) -> Vec<ProcNet> {
    let mut rows = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with(',') || line.contains("bytes_in") {
            continue;
        }
        let f: Vec<&str> = line.split(',').collect();
        if f.len() < 3 || f[0].is_empty() {
            continue;
        }
        let (name, pid) = match f[0].rsplit_once('.') {
            Some((n, p)) if !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) => {
                (n.to_string(), p.parse().unwrap_or(0))
            }
            _ => (f[0].to_string(), 0),
        };
        let bytes_in: u64 = f[1].trim().parse().unwrap_or(0);
        let bytes_out: u64 = f[2].trim().parse().unwrap_or(0);
        rows.push(ProcNet {
            name,
            pid,
            bytes_in,
            bytes_out,
            total: bytes_in + bytes_out,
        });
    }
    rows.sort_by_key(|r| std::cmp::Reverse(r.total));
    rows
}

pub fn parse_netstat(netstat: &str, tasklist: &str) -> Vec<ProcNet> {
    let names = parse_tasklist_names(tasklist);
    let mut counts: HashMap<u32, u64> = HashMap::new();
    for line in netstat.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 4 {
            continue;
        }
        let proto = fields[0].to_ascii_uppercase();
        if proto != "TCP" && proto != "UDP" {
            continue;
        }
        if let Some(pid) = fields.last().and_then(|s| s.parse::<u32>().ok()) {
            *counts.entry(pid).or_insert(0) += 1;
        }
    }
    let mut rows: Vec<ProcNet> = counts
        .into_iter()
        .map(|(pid, count)| ProcNet {
            name: names
                .get(&pid)
                .cloned()
                .unwrap_or_else(|| format!("pid:{pid}")),
            pid,
            bytes_in: 0,
            bytes_out: 0,
            total: count,
        })
        .collect();
    rows.sort_by_key(|r| std::cmp::Reverse(r.total));
    rows
}

fn parse_tasklist_names(tasklist: &str) -> HashMap<u32, String> {
    let mut out = HashMap::new();
    for line in tasklist.lines() {
        let fields = parse_csv_line(line);
        if fields.len() < 2 {
            continue;
        }
        if let Ok(pid) = fields[1].parse::<u32>() {
            out.insert(pid, fields[0].clone());
        }
    }
    out
}

fn parse_csv_line(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                cur.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            ',' if !quoted => {
                fields.push(cur.clone());
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    fields.push(cur);
    fields
}

pub fn resolve_nettop() -> Result<PathBuf, String> {
    let p = PathBuf::from("/usr/bin/nettop");
    if p.exists() {
        Ok(p)
    } else {
        Err("nettop not found (macOS only); per-app network is unavailable".into())
    }
}

pub fn run_nettop(bin: &Path) -> Result<String, String> {
    let out = platform::command(bin, ["-P", "-L", "1", "-x", "-J", "bytes_in,bytes_out"])?
        .output()
        .map_err(|e| format!("failed to run nettop: {e}"))?;
    if !out.status.success() {
        return Err(format!("nettop exited {}", out.status));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_ranks_by_total() {
        let csv =
            ",bytes_in,bytes_out,\nlaunchd.1,0,0,\napsd.573,28142,40592,\nchrome.900,1000,2000,\n";
        let r = parse_nettop(csv);
        assert_eq!(r.len(), 3);
        assert_eq!(r[0].name, "apsd");
        assert_eq!(r[0].pid, 573);
        assert_eq!(r[0].total, 68734);
        assert_eq!(r[1].name, "chrome");
        assert_eq!(r[2].name, "launchd");
    }

    #[test]
    fn handles_dotted_process_names() {
        let r = parse_nettop("com.apple.WebKit.Networking.123,5,7,\n");
        assert_eq!(r[0].name, "com.apple.WebKit.Networking");
        assert_eq!(r[0].pid, 123);
        assert_eq!(r[0].total, 12);
    }

    #[test]
    fn skips_header_and_blank_lines() {
        assert!(parse_nettop(",bytes_in,bytes_out,\n\n   \n").is_empty());
    }

    #[test]
    fn parses_windows_netstat_with_task_names() {
        let netstat = "\
Proto  Local Address          Foreign Address        State           PID\n\
TCP    127.0.0.1:5000         127.0.0.1:5001         ESTABLISHED     42\n\
TCP    127.0.0.1:5002         127.0.0.1:5003         ESTABLISHED     42\n\
UDP    0.0.0.0:5353           *:*                                    7\n";
        let tasklist = "\"chrome.exe\",\"42\",\"Console\",\"1\",\"10,000 K\"\n\"dns.exe\",\"7\",\"Console\",\"1\",\"1,000 K\"\n";
        let rows = parse_netstat(netstat, tasklist);
        assert_eq!(rows[0].name, "chrome.exe");
        assert_eq!(rows[0].pid, 42);
        assert_eq!(rows[0].total, 2);
        assert_eq!(rows[1].name, "dns.exe");
    }
}
