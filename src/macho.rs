//! Mach-O universal ("fat") binary analysis — the read-only core of app-slim.
//!
//! Pure byte parsing of the fat header (big-endian), so it's fully testable without any
//! macOS framework. Reports the architecture slices and how much a thin-to-host-arch would
//! save. The actual thinning (rewrite + ad-hoc re-sign) is the deferred I/O step.

use serde::Serialize;

pub const CPU_ARM64: i32 = 0x0100_000C;
pub const CPU_X86_64: i32 = 0x0100_0007;

const FAT_MAGIC: u32 = 0xCAFE_BABE; // 32-bit fat
const FAT_MAGIC_64: u32 = 0xCAFE_BABF; // 64-bit fat

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct ArchSlice {
    pub cputype: i32,
    pub cpusubtype: i32,
    pub offset: u64,
    pub size: u64,
}

fn be_u32(b: &[u8], i: usize) -> u32 {
    u32::from_be_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}
fn be_u64(b: &[u8], i: usize) -> u64 {
    u64::from_be_bytes([
        b[i],
        b[i + 1],
        b[i + 2],
        b[i + 3],
        b[i + 4],
        b[i + 5],
        b[i + 6],
        b[i + 7],
    ])
}

/// Parse a fat Mach-O header into its architecture slices. Errors if the bytes are not a fat
/// Mach-O (e.g. a thin single-arch binary or a non-Mach-O file).
pub fn parse_fat(bytes: &[u8]) -> Result<Vec<ArchSlice>, String> {
    if bytes.len() < 8 {
        return Err("too short for a fat header".into());
    }
    let is64 = match be_u32(bytes, 0) {
        FAT_MAGIC => false,
        FAT_MAGIC_64 => true,
        _ => return Err("not a fat Mach-O (thin binary or non-Mach-O)".into()),
    };
    let n = be_u32(bytes, 4) as usize;
    if n > 64 {
        return Err(format!("implausible arch count {n}"));
    }
    let mut slices = Vec::with_capacity(n);
    let mut off = 8usize;
    for _ in 0..n {
        if is64 {
            if off + 32 > bytes.len() {
                return Err("truncated fat_arch_64".into());
            }
            slices.push(ArchSlice {
                cputype: be_u32(bytes, off) as i32,
                cpusubtype: be_u32(bytes, off + 4) as i32,
                offset: be_u64(bytes, off + 8),
                size: be_u64(bytes, off + 16),
            });
            off += 32;
        } else {
            if off + 20 > bytes.len() {
                return Err("truncated fat_arch".into());
            }
            slices.push(ArchSlice {
                cputype: be_u32(bytes, off) as i32,
                cpusubtype: be_u32(bytes, off + 4) as i32,
                offset: be_u32(bytes, off + 8) as u64,
                size: be_u32(bytes, off + 12) as u64,
            });
            off += 20;
        }
    }
    Ok(slices)
}

/// Bytes reclaimable by keeping only `keep` cputype. 0 if `keep` is absent (keep everything).
pub fn slim_savings(slices: &[ArchSlice], keep: i32) -> u64 {
    if !slices.iter().any(|s| s.cputype == keep) {
        return 0;
    }
    slices
        .iter()
        .filter(|s| s.cputype != keep)
        .map(|s| s.size)
        .sum()
}

/// Thin a fat Mach-O to a single architecture: return the bytes of the `keep` slice.
/// (The caller writes them out and ad-hoc re-signs — Pearcleaner notably does NOT re-sign.)
pub fn thin(bytes: &[u8], keep: i32) -> Result<Vec<u8>, String> {
    let slices = parse_fat(bytes)?;
    let s = slices
        .iter()
        .find(|s| s.cputype == keep)
        .ok_or_else(|| format!("architecture {keep:#x} not present in this binary"))?;
    let start = s.offset as usize;
    let end = start
        .checked_add(s.size as usize)
        .ok_or("slice offset+size overflow")?;
    if end > bytes.len() {
        return Err("slice extends past end of file (corrupt fat binary)".into());
    }
    Ok(bytes[start..end].to_vec())
}

/// The host CPU type (slice to keep when slimming on this machine).
pub fn host_cputype() -> i32 {
    if cfg!(target_arch = "aarch64") {
        CPU_ARM64
    } else {
        CPU_X86_64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a synthetic 32-bit fat header with the given (cputype, size) arches.
    fn fat32(arches: &[(i32, u32)]) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&FAT_MAGIC.to_be_bytes());
        b.extend_from_slice(&(arches.len() as u32).to_be_bytes());
        for (i, (ct, size)) in arches.iter().enumerate() {
            b.extend_from_slice(&(*ct as u32).to_be_bytes()); // cputype
            b.extend_from_slice(&0u32.to_be_bytes()); // cpusubtype
            b.extend_from_slice(&((0x1000 * (i as u32 + 1)).to_be_bytes())); // offset
            b.extend_from_slice(&size.to_be_bytes()); // size
            b.extend_from_slice(&14u32.to_be_bytes()); // align
        }
        b
    }

    /// Build a fat binary with real slice payloads laid out right after the header.
    fn fat_with_payloads(arches: &[(i32, &[u8])]) -> Vec<u8> {
        let n = arches.len();
        let header = 8 + n * 20;
        let mut offsets = Vec::new();
        let mut cur = header;
        for (_, p) in arches {
            offsets.push(cur);
            cur += p.len();
        }
        let mut b = Vec::new();
        b.extend_from_slice(&FAT_MAGIC.to_be_bytes());
        b.extend_from_slice(&(n as u32).to_be_bytes());
        for (i, (ct, p)) in arches.iter().enumerate() {
            b.extend_from_slice(&(*ct as u32).to_be_bytes());
            b.extend_from_slice(&0u32.to_be_bytes());
            b.extend_from_slice(&(offsets[i] as u32).to_be_bytes());
            b.extend_from_slice(&(p.len() as u32).to_be_bytes());
            b.extend_from_slice(&14u32.to_be_bytes());
        }
        for (_, p) in arches {
            b.extend_from_slice(p);
        }
        b
    }

    #[test]
    fn thin_extracts_the_kept_slice() {
        let arm: &[u8] = b"ARM64-slice-payload";
        let x86: &[u8] = b"X86_64-slice";
        let fat = fat_with_payloads(&[(CPU_ARM64, arm), (CPU_X86_64, x86)]);
        assert_eq!(thin(&fat, CPU_ARM64).unwrap(), arm);
        assert_eq!(thin(&fat, CPU_X86_64).unwrap(), x86);
        assert!(thin(&fat, 0x999).is_err()); // absent arch
    }

    #[test]
    fn parses_two_arch_fat() {
        let bytes = fat32(&[(CPU_ARM64, 5000), (CPU_X86_64, 3000)]);
        let slices = parse_fat(&bytes).unwrap();
        assert_eq!(slices.len(), 2);
        assert_eq!(slices[0].cputype, CPU_ARM64);
        assert_eq!(slices[0].size, 5000);
        assert_eq!(slices[1].cputype, CPU_X86_64);
        assert_eq!(slices[1].size, 3000);
    }

    #[test]
    fn savings_keeps_host_drops_rest() {
        let slices = parse_fat(&fat32(&[(CPU_ARM64, 5000), (CPU_X86_64, 3000)])).unwrap();
        assert_eq!(slim_savings(&slices, CPU_ARM64), 3000); // drop x86_64
        assert_eq!(slim_savings(&slices, CPU_X86_64), 5000); // drop arm64
    }

    #[test]
    fn savings_zero_when_keep_absent() {
        let slices = parse_fat(&fat32(&[(CPU_ARM64, 5000)])).unwrap();
        assert_eq!(slim_savings(&slices, CPU_X86_64), 0); // would-keep arch not present
    }

    #[test]
    fn thin_binary_is_not_fat() {
        // 0xFEEDFACF = thin 64-bit Mach-O magic
        let mut b = 0xFEED_FACFu32.to_be_bytes().to_vec();
        b.extend_from_slice(&[0u8; 8]);
        assert!(parse_fat(&b).is_err());
    }
}
