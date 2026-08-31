use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use crate::distance_field::distance_field_from_alpha;
use crate::quantize::quantize_image;

use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

fn temp_dir(name: &str) -> PathBuf {
    let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    let path = env::temp_dir().join(format!("morf-image-{name}-{}-{id}", std::process::id()));
    fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn svg_is_scaled_and_cached_by_physical_size() {
    let root = temp_dir("svg");
    let path = root.join("square.svg");
    fs::write(
            &path,
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="2"><rect width="2" height="2" fill="#ff0000"/></svg>"##,
        )
        .unwrap();
    let mut cache = ImageCache::default();
    let first = cache.load(&path, 8, 4, 180).unwrap();
    let second = cache.load(&path, 8, 4, 180).unwrap();
    assert_eq!((first.width, first.height), (12, 6));
    assert_eq!(cache.intrinsic_size(&path).unwrap(), (2, 2));
    assert_eq!(&first.rgba[..4], &[255, 0, 0, 255]);
    assert!(Arc::ptr_eq(&first, &second));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn file_uri_is_decoded_and_shares_the_path_cache() {
    let root = temp_dir("file-uri");
    let path = root.join("square space.svg");
    fs::write(
            &path,
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="2"><rect width="2" height="2" fill="#00ff00"/></svg>"##,
        )
        .unwrap();
    let uri = format!("file://{}", path.to_string_lossy().replace(' ', "%20"));
    let mut cache = ImageCache::default();
    let from_uri = cache.load(&uri, 4, 4, 120).unwrap();
    let from_path = cache.load(&path, 4, 4, 120).unwrap();

    assert_eq!(cache.intrinsic_size(&uri).unwrap(), (2, 2));
    assert_eq!(&from_uri.rgba[..4], &[0, 255, 0, 255]);
    assert!(Arc::ptr_eq(&from_uri, &from_path));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn file_uri_rejects_remote_authorities_and_bad_escapes() {
    let mut cache = ImageCache::default();
    assert!(matches!(
        cache.intrinsic_size("file://remote/image.svg"),
        Err(ImageError::InvalidSource(_))
    ));
    assert!(matches!(
        cache.intrinsic_size("file:///tmp/bad%2.svg"),
        Err(ImageError::InvalidSource(_))
    ));
}

#[test]
fn icon_lookup_prefers_closest_directory_and_inherits() {
    let root = temp_dir("icons");
    let parent = root.join("parent");
    let child = root.join("child");
    fs::create_dir_all(parent.join("16x16/apps")).unwrap();
    fs::create_dir_all(parent.join("64x64/apps")).unwrap();
    fs::create_dir_all(&child).unwrap();
    fs::write(
            parent.join("index.theme"),
            "[Icon Theme]\nDirectories=16x16/apps,64x64/apps\n\n[16x16/apps]\nSize=16\nType=Fixed\n\n[64x64/apps]\nSize=64\nType=Fixed\n",
        )
        .unwrap();
    fs::write(
        child.join("index.theme"),
        "[Icon Theme]\nInherits=parent\nDirectories=\n",
    )
    .unwrap();
    let expected = parent.join("64x64/apps/demo.svg");
    fs::write(&expected, "<svg xmlns=\"http://www.w3.org/2000/svg\"/>").unwrap();
    let resolver = IconResolver::new(vec![root.clone()]);
    assert_eq!(resolver.find("demo", "child", 48).unwrap(), expected);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn quantizer_splits_the_widest_color_channel() {
    let image = ImageData {
        width: 4,
        height: 1,
        rgba: vec![
            0, 10, 20, 255, 20, 10, 20, 255, 220, 10, 20, 255, 240, 10, 20, 255,
        ],
    };
    let colors = quantize_image(&image, 1, None).unwrap();
    assert_eq!(colors, [[10, 10, 20, 255], [230, 10, 20, 255]]);
    let cropped = quantize_image(
        &image,
        0,
        Some(ImageRect {
            x: 2,
            y: 0,
            width: 2,
            height: 1,
        }),
    )
    .unwrap();
    assert_eq!(cropped, [[230, 10, 20, 255]]);
}

#[test]
fn signed_distance_field_marks_inside_edge_and_outside() {
    let mut rgba = vec![0; 7 * 7 * 4];
    for y in 2..5 {
        for x in 2..5 {
            rgba[(y * 7 + x) * 4 + 3] = 255;
        }
    }
    let image = ImageData {
        width: 7,
        height: 7,
        rgba,
    };
    let field = distance_field_from_alpha(&image, 3.0).unwrap();
    let distance = |x: usize, y: usize| field.rgba[(y * 7 + x) * 4];

    assert!(distance(3, 3) < 128);
    assert!((120..=136).contains(&distance(2, 3)));
    assert!(distance(0, 0) > 128);
}

#[test]
fn a_huge_request_is_refused_rather_than_silently_resized() {
    // Multiplying in u32 and saturating first does not clamp the answer, it
    // changes it: u32::MAX / 120 is a perfectly plausible-looking size that
    // bears no relation to what was asked for. Overflow has to be an error.
    let mut cache = ImageCache::default();
    let source = std::env::temp_dir().join("morf-image-huge.svg");
    std::fs::write(
        &source,
        br#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><rect width="8" height="8"/></svg>"#,
    )
    .unwrap();

    assert!(
        cache.load(&source, u32::MAX, u32::MAX, 240).is_err(),
        "a size that cannot be represented is refused"
    );
    let _ = std::fs::remove_file(&source);
}

#[test]
fn decoding_many_sizes_does_not_grow_the_cache_without_end() {
    // The cache key includes the pixel size, and that size comes off live
    // geometry — so an animated icon width mints one decode per step. Nothing
    // ever evicted them, which made an ordinary animation a memory leak.
    let mut cache = ImageCache::default();
    let source = std::env::temp_dir().join("morf-image-many.svg");
    std::fs::write(
        &source,
        br##"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><rect width="8" height="8" fill="#fff"/></svg>"##,
    )
    .unwrap();

    for size in 8..400u32 {
        cache.load(&source, size, size, 120).unwrap();
        cache.shrink();
    }
    assert!(
        cache.decoded_len() <= 128,
        "the cache stays bounded: {}",
        cache.decoded_len()
    );
    let _ = std::fs::remove_file(&source);
}
