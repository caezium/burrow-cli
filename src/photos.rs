//! Similar-photo detection via perceptual hashing (dHash).
//!
//! Decodes PNG/JPEG with the pure-Rust `image` crate (no system libs). The hash + grouping
//! are tested end-to-end against generated images. HEIC (the dominant Apple Photos format)
//! needs the `heif` feature + system libheif — the documented next integration step.

use image::{imageops::FilterType, DynamicImage};
use serde::Serialize;
use std::path::Path;

/// 64-bit difference hash from a 9x8 grayscale downscale (each bit: left brighter than right).
pub fn dhash(img: &DynamicImage) -> u64 {
    let small = img.resize_exact(9, 8, FilterType::Triangle).to_luma8();
    let mut hash = 0u64;
    let mut bit = 0;
    for y in 0..8u32 {
        for x in 0..8u32 {
            if small.get_pixel(x, y)[0] > small.get_pixel(x + 1, y)[0] {
                hash |= 1 << bit;
            }
            bit += 1;
        }
    }
    hash
}

pub fn hamming(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

pub fn hash_file(path: &Path) -> Result<u64, String> {
    let img = image::open(path).map_err(|e| format!("decode {}: {e}", path.display()))?;
    Ok(dhash(&img))
}

fn is_image(p: &Path) -> bool {
    matches!(
        p.extension()
            .and_then(|e| e.to_str())
            .map(|s| s.to_lowercase())
            .as_deref(),
        Some("png") | Some("jpg") | Some("jpeg")
    )
}

#[derive(Debug, Serialize, PartialEq)]
pub struct Group {
    pub paths: Vec<String>,
}

/// Greedy clustering: group items whose hashes are within `threshold` hamming distance.
/// Only groups with >1 member are returned.
pub fn group_similar(items: &[(String, u64)], threshold: u32) -> Vec<Group> {
    let mut groups: Vec<(u64, Vec<String>)> = Vec::new();
    for (path, h) in items {
        if let Some(g) = groups
            .iter_mut()
            .find(|(rep, _)| hamming(*rep, *h) <= threshold)
        {
            g.1.push(path.clone());
        } else {
            groups.push((*h, vec![path.clone()]));
        }
    }
    groups
        .into_iter()
        .filter(|(_, v)| v.len() > 1)
        .map(|(_, paths)| Group { paths })
        .collect()
}

/// Scan a directory (non-recursive) for similar PNG/JPEG images.
pub fn scan_dir(dir: &Path, threshold: u32) -> Vec<Group> {
    let mut items = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let p = e.path();
            if is_image(&p) {
                if let Ok(h) = hash_file(&p) {
                    items.push((p.to_string_lossy().into_owned(), h));
                }
            }
        }
    }
    items.sort_by(|a, b| a.0.cmp(&b.0));
    group_similar(&items, threshold)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Luma};

    /// A horizontal "tent" (rises then falls in x) -> a distinctive, non-trivial dhash.
    fn tent_x(offset: i32) -> DynamicImage {
        let img = ImageBuffer::from_fn(64, 64, |x, _y| {
            let base = if x < 32 { x } else { 63 - x } as i32;
            Luma([(base * 4 + offset).clamp(0, 255) as u8])
        });
        DynamicImage::ImageLuma8(img)
    }
    /// A vertical tent -> horizontally uniform -> hash ~0 (structurally different).
    fn tent_y() -> DynamicImage {
        let img = ImageBuffer::from_fn(64, 64, |_x, y| {
            let base = if y < 32 { y } else { 63 - y } as i32;
            Luma([(base * 4).clamp(0, 255) as u8])
        });
        DynamicImage::ImageLuma8(img)
    }

    #[test]
    fn dhash_is_deterministic_and_nontrivial() {
        let h = dhash(&tent_x(0));
        assert_eq!(h, dhash(&tent_x(0)));
        assert!(h != 0, "tent_x should produce a non-trivial hash");
    }

    #[test]
    fn similar_images_group() {
        let items = vec![
            ("a.png".to_string(), dhash(&tent_x(0))),
            ("b.png".to_string(), dhash(&tent_x(8))), // same structure, brighter
        ];
        let g = group_similar(&items, 10);
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].paths.len(), 2);
    }

    #[test]
    fn different_images_do_not_group() {
        let items = vec![
            ("a.png".to_string(), dhash(&tent_x(0))),
            ("c.png".to_string(), dhash(&tent_y())),
        ];
        assert!(group_similar(&items, 5).is_empty());
    }

    #[test]
    fn scan_dir_finds_similar_pngs_end_to_end() {
        let dir = std::env::temp_dir().join(format!("burrow_photos_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        tent_x(0).save(dir.join("a.png")).unwrap();
        tent_x(8).save(dir.join("b.png")).unwrap(); // similar
        tent_y().save(dir.join("c.png")).unwrap(); // different
        let groups = scan_dir(&dir, 10);
        assert_eq!(
            groups.len(),
            1,
            "a.png + b.png should form one similar group"
        );
        assert_eq!(groups[0].paths.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
