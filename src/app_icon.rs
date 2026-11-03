//! PopChat's original two-blue-bubble tray artwork, shared by every app surface.
//! Coordinates and colors are unchanged; every size gets fresh 4×4 coverage
//! sampling instead of scaling a small bitmap. No fonts or external assets.

/// Windows shell / installer sizes, including the 256px Explorer frame.
pub const ICO_SIZES: [u32; 7] = [16, 24, 32, 48, 64, 128, 256];

/// Render square, straight-alpha RGBA pixels. Supported sizes are 1..=256.
/// At 32px this is byte-for-byte the original system tray renderer.
pub fn rgba(size: u32) -> Vec<u8> {
    assert!((1..=256).contains(&size), "icon size must be 1..=256");
    fn inside(x: f32, y: f32, left: f32, top: f32, right: f32, bottom: f32) -> bool {
        let radius = 4.0;
        let cx = x.clamp(left + radius, right - radius);
        let cy = y.clamp(top + radius, bottom - radius);
        (x - cx).powi(2) + (y - cy).powi(2) <= radius * radius
    }
    let mut rgba = vec![0; (size * size * 4) as usize];
    let scale = 32.0 / size as f32;
    for y in 0..size {
        for x in 0..size {
            let mut covered = 0;
            let mut front = 0;
            for sy in 0..4 {
                for sx in 0..4 {
                    let px = (x as f32 + (sx as f32 + 0.5) / 4.0) * scale;
                    let py = (y as f32 + (sy as f32 + 0.5) / 4.0) * scale;
                    let a = inside(px, py, 2.0, 3.0, 23.0, 21.0)
                        || ((4.0..=9.0).contains(&px)
                            && (19.0..=25.0).contains(&py)
                            && px + py < 30.0);
                    let b = inside(px, py, 12.0, 13.0, 30.0, 28.0)
                        || ((24.0..=28.0).contains(&px)
                            && (26.0..=31.0).contains(&py)
                            && py - px < 5.0);
                    covered += u16::from(a || b);
                    front += u16::from(b);
                }
            }
            let offset = ((y * size + x) * 4) as usize;
            if covered > 0 {
                let light = front * 110 / covered;
                rgba[offset] = (44 + light) as u8;
                rgba[offset + 1] = (112 + light / 2) as u8;
                rgba[offset + 2] = 240;
                rgba[offset + 3] = (covered * 255 / 16) as u8;
            }
        }
    }
    rgba
}

/// Deterministic, dependency-free ICO encoding (32-bit BGRA DIB plus AND mask).
/// Kept here so the checked-in installer/PE asset can be verified against the
/// exact renderer used by the tray and viewport; builds never rewrite it.
pub fn ico_bytes() -> Vec<u8> {
    let mut bytes = vec![0, 0, 1, 0, ICO_SIZES.len() as u8, 0];
    let mut frames = Vec::new();
    let mut offset = 6 + ICO_SIZES.len() * 16;
    for size in ICO_SIZES {
        let pixels = rgba(size);
        let mask_stride = size.div_ceil(32) as usize * 4;
        let image_size = pixels.len() + mask_stride * size as usize;
        let frame_size = 40 + image_size;
        bytes.extend_from_slice(&[size as u8, size as u8, 0, 0]);
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&32u16.to_le_bytes());
        bytes.extend_from_slice(&(frame_size as u32).to_le_bytes());
        bytes.extend_from_slice(&(offset as u32).to_le_bytes());
        offset += frame_size;

        let mut frame = Vec::with_capacity(frame_size);
        for value in [40, size, size * 2] {
            frame.extend_from_slice(&value.to_le_bytes());
        }
        frame.extend_from_slice(&1u16.to_le_bytes());
        frame.extend_from_slice(&32u16.to_le_bytes());
        for value in [0, image_size as u32, 0, 0, 0, 0] {
            frame.extend_from_slice(&value.to_le_bytes());
        }
        for y in (0..size as usize).rev() {
            for x in 0..size as usize {
                let pixel = &pixels[(y * size as usize + x) * 4..][..4];
                frame.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
            }
        }
        for y in (0..size as usize).rev() {
            let mut row = vec![0; mask_stride];
            for x in 0..size as usize {
                if pixels[(y * size as usize + x) * 4 + 3] == 0 {
                    row[x / 8] |= 0x80 >> (x % 8);
                }
            }
            frame.extend_from_slice(&row);
        }
        frames.push(frame);
    }
    for frame in frames {
        bytes.extend_from_slice(&frame);
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalable_coverage_colors_and_transparent_edges() {
        for size in ICO_SIZES {
            let pixels = rgba(size);
            assert_eq!(pixels.len(), (size * size * 4) as usize);
            let opaque = pixels.chunks_exact(4).filter(|px| px[3] == 255).count();
            let fraction = opaque as f32 / (size * size) as f32;
            assert!((0.40..0.65).contains(&fraction), "size={size}: {fraction}");
            assert!(pixels.chunks_exact(4).any(|px| px[3] > 0 && px[3] < 255));
            assert!(pixels.chunks_exact(4).any(|px| px == [44, 112, 240, 255]));
            assert!(pixels.chunks_exact(4).any(|px| px == [154, 167, 240, 255]));
            for (x, y) in [(0, 0), (size - 1, 0), (0, size - 1), (size - 1, size - 1)] {
                assert_eq!(pixels[((y * size + x) * 4 + 3) as usize], 0);
            }
            // The original tail reaches into the last row at 16px, but each
            // edge retains transparent space and the artwork never clips.
            for side in 0..4 {
                assert!((0..size).any(|i| {
                    let (x, y) = match side {
                        0 => (i, 0),
                        1 => (i, size - 1),
                        2 => (0, i),
                        _ => (size - 1, i),
                    };
                    pixels[((y * size + x) * 4 + 3) as usize] == 0
                }));
            }
        }
    }

    #[test]
    fn original_32px_pixels_are_preserved() {
        // FNV-1a snapshot of the pre-refactor tray_pixels() byte stream.
        let hash = rgba(32).iter().fold(0xcbf29ce484222325u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        });
        assert_eq!(hash, 0xd3376efa84792ba2);
    }

    #[test]
    fn ico_frames_decode_to_the_shared_rgba_and_have_correct_masks() {
        let ico = ico_bytes();
        assert_eq!(&ico[..6], &[0, 0, 1, 0, 7, 0]);
        let read_u32 = |at| u32::from_le_bytes(ico[at..at + 4].try_into().unwrap()) as usize;
        let mut expected_offset = 6 + ICO_SIZES.len() * 16;
        for (index, size) in ICO_SIZES.into_iter().enumerate() {
            let size = size as usize;
            let entry = 6 + index * 16;
            assert_eq!(ico[entry], size as u8); // 0 means 256 in ICO.
            assert_eq!(ico[entry + 1], size as u8);
            assert_eq!(&ico[entry + 4..entry + 8], &[1, 0, 32, 0]);
            let length = read_u32(entry + 8);
            let offset = read_u32(entry + 12);
            assert_eq!(offset, expected_offset);
            expected_offset += length;
            assert_eq!(read_u32(offset), 40);
            assert_eq!(read_u32(offset + 4), size);
            assert_eq!(read_u32(offset + 8), size * 2);
            let mask_stride = size.div_ceil(32) * 4;
            assert_eq!(length, 40 + size * size * 4 + mask_stride * size);
            let expected = rgba(size as u32);
            let mut decoded = vec![0; expected.len()];
            for y in 0..size {
                for x in 0..size {
                    let target = (y * size + x) * 4;
                    let source = offset + 40 + ((size - 1 - y) * size + x) * 4;
                    decoded[target..target + 4].copy_from_slice(&[
                        ico[source + 2],
                        ico[source + 1],
                        ico[source],
                        ico[source + 3],
                    ]);
                    let mask = offset + 40 + size * size * 4 + (size - 1 - y) * mask_stride + x / 8;
                    assert_eq!(
                        ico[mask] & (0x80 >> (x % 8)) != 0,
                        expected[target + 3] == 0
                    );
                }
            }
            assert_eq!(decoded, expected, "decoded {size}px frame");
        }
        assert_eq!(expected_offset, ico.len());
    }

    #[test]
    fn checked_in_ico_is_deterministic_and_current() {
        let first = ico_bytes();
        assert_eq!(first, ico_bytes());
        assert_eq!(first, include_bytes!("../assets/app-icon/app.ico"));
    }

    #[test]
    #[should_panic(expected = "icon size must be 1..=256")]
    fn rejects_zero_size() {
        rgba(0);
    }

    #[test]
    #[should_panic(expected = "icon size must be 1..=256")]
    fn rejects_unbounded_allocations() {
        rgba(u32::MAX);
    }
}
