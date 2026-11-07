use super::status::{DeviceStatus, Level, level_for};
use crate::drivers::profile::{CaseReporting, DeviceProfile};

pub const COLOR_HIGH: [u8; 4] = [0x2E, 0xCC, 0x71, 0xFF]; // Green
pub const COLOR_MID: [u8; 4] = [0xF3, 0x9C, 0x12, 0xFF]; // Amber
pub const COLOR_LOW: [u8; 4] = [0xE7, 0x4C, 0x3C, 0xFF]; // Red
pub const COLOR_GLYPH: [u8; 3] = [0xD0, 0xD0, 0xD0];
pub const COLOR_GLYPH_DISCONNECTED: [u8; 3] = [0x70, 0x70, 0x70];

const SVG_LEFT: &str = include_str!("../../assets/icons/bbm-earbuds-stem-left-symbolic.svg");
const SVG_RIGHT: &str = include_str!("../../assets/icons/bbm-earbuds-stem-right-symbolic.svg");
const SVG_CASE: &str = include_str!("../../assets/icons/bbm-case-normal-symbolic.svg");
const SVG_SINGLE: &str = include_str!("../../assets/icons/bbm-earbuds-single-symbolic.svg");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    Left,
    Right,
    Case,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconState {
    Live,
    Connecting,
    Disconnected,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ring {
    pub cx: f32,
    pub cy: f32,
    pub radius: f32,
    pub stroke: f32,
}

pub fn ring_geometry(size: u32) -> Ring {
    let size_f = size as f32;
    let one = size_f / 16.0;
    let stroke = (2.0 * one).round().max(2.0);
    let radius = (size_f - stroke) / 2.0;
    Ring {
        cx: size_f / 2.0,
        cy: size_f / 2.0,
        radius,
        stroke,
    }
}

pub fn sweep(pct: u8) -> f32 {
    if pct == 0 {
        0.0
    } else {
        let raw = (pct.min(100) as f32 / 100.0) * std::f32::consts::TAU;
        raw.max(0.15)
    }
}

pub fn slot_value(slot: Slot, status: &DeviceStatus) -> (Option<u8>, IconState) {
    match status {
        DeviceStatus::Disconnected => (None, IconState::Disconnected),
        DeviceStatus::Connecting => (None, IconState::Connecting),
        DeviceStatus::Connected { view: None, .. } => (None, IconState::Live),
        DeviceStatus::Connected {
            view: Some(view), ..
        } => {
            let val = match slot {
                Slot::Left => view.left,
                Slot::Right => view.right,
                Slot::Case => view.case,
            };
            (val, IconState::Live)
        }
    }
}

pub fn slot_visible(slot: Slot, status: &DeviceStatus, profile: &DeviceProfile) -> bool {
    match status {
        DeviceStatus::Connecting | DeviceStatus::Disconnected => match slot {
            Slot::Left | Slot::Right => false,
            Slot::Case => true,
        },
        DeviceStatus::Connected { view, source } => match source {
            crate::app::status::Source::Windows
            | crate::app::status::Source::FastPairHfp
            | crate::app::status::Source::VendorHfp(_) => match slot {
                Slot::Left => true,
                Slot::Right | Slot::Case => false,
            },
            crate::app::status::Source::FastPair { .. } => match slot {
                Slot::Left | Slot::Right => true,
                Slot::Case => view.as_ref().and_then(|v| v.case).is_some(),
            },
            crate::app::status::Source::Vendor(_) => match slot {
                Slot::Left | Slot::Right => true,
                Slot::Case => match view {
                    None => profile.case == CaseReporting::Always,
                    Some(v) => match profile.case {
                        CaseReporting::Never => false,
                        CaseReporting::WithBudInCase => v.case.is_some(),
                        CaseReporting::Always => true,
                    },
                },
            },
        },
    }
}

pub fn glyph_box(size: u32) -> f32 {
    let ring = ring_geometry(size);
    (2.0 * (ring.radius - ring.stroke) - 1.0).max(1.0)
}

fn add_arc(
    pb: &mut resvg::tiny_skia::PathBuilder,
    cx: f32,
    cy: f32,
    radius: f32,
    start_angle: f32,
    sweep_angle: f32,
) {
    if sweep_angle <= 0.0 {
        return;
    }
    let n_segments = (sweep_angle / std::f32::consts::FRAC_PI_2).ceil().max(1.0) as usize;
    let step = sweep_angle / n_segments as f32;
    let mut current_angle = start_angle;

    let p0_x = cx + radius * current_angle.cos();
    let p0_y = cy + radius * current_angle.sin();
    pb.move_to(p0_x, p0_y);

    for _ in 0..n_segments {
        let next_angle = current_angle + step;
        let delta = next_angle - current_angle;
        let k = (4.0 / 3.0) * (delta / 4.0).tan();

        let p1_x = cx + radius * (current_angle.cos() - k * current_angle.sin());
        let p1_y = cy + radius * (current_angle.sin() + k * current_angle.cos());

        let p2_x = cx + radius * (next_angle.cos() + k * next_angle.sin());
        let p2_y = cy + radius * (next_angle.sin() - k * next_angle.cos());

        let p3_x = cx + radius * next_angle.cos();
        let p3_y = cy + radius * next_angle.sin();

        pb.cubic_to(p1_x, p1_y, p2_x, p2_y, p3_x, p3_y);
        current_angle = next_angle;
    }
}

pub fn render_slot(slot: Slot, value: Option<u8>, state: IconState, size: u32) -> Vec<u8> {
    render_slot_with_style(slot, value, state, size, false)
}

pub fn render_slot_with_style(
    slot: Slot,
    value: Option<u8>,
    state: IconState,
    size: u32,
    single_neutral: bool,
) -> Vec<u8> {
    let ring = ring_geometry(size);
    let mut pixmap = match resvg::tiny_skia::Pixmap::new(size, size) {
        Some(p) => p,
        None => return vec![0; (size * size * 4) as usize],
    };

    let glyph_rgb = match state {
        IconState::Disconnected => COLOR_GLYPH_DISCONNECTED,
        IconState::Live | IconState::Connecting => COLOR_GLYPH,
    };

    // 1. Base ring: full circle in glyph color at 40% alpha
    let mut pb_base = resvg::tiny_skia::PathBuilder::new();
    pb_base.push_circle(ring.cx, ring.cy, ring.radius);
    if let Some(base_path) = pb_base.finish() {
        let base_color = resvg::tiny_skia::Color::from_rgba8(
            glyph_rgb[0],
            glyph_rgb[1],
            glyph_rgb[2],
            (255.0f32 * 0.40).round() as u8,
        );
        let mut paint = resvg::tiny_skia::Paint::default();
        paint.set_color(base_color);
        paint.anti_alias = true;

        let stroke = resvg::tiny_skia::Stroke {
            width: ring.stroke,
            line_cap: resvg::tiny_skia::LineCap::Butt,
            ..Default::default()
        };

        pixmap.stroke_path(
            &base_path,
            &paint,
            &stroke,
            resvg::tiny_skia::Transform::identity(),
            None,
        );
    }

    // 2. Fill arc: clockwise from 12 o'clock, colored by level_for(pct)
    if state == IconState::Live
        && let Some(pct) = value
        && pct > 0
    {
        let sweep_rad = sweep(pct);
        let stroke = resvg::tiny_skia::Stroke {
            width: ring.stroke,
            line_cap: resvg::tiny_skia::LineCap::Butt,
            ..Default::default()
        };

        let fill_color = match level_for(pct) {
            Level::Low => COLOR_LOW,
            Level::Mid => COLOR_MID,
            Level::High => COLOR_HIGH,
            Level::Unknown => [0, 0, 0, 0],
        };
        let mut fill_paint = resvg::tiny_skia::Paint::default();
        fill_paint.set_color_rgba8(fill_color[0], fill_color[1], fill_color[2], fill_color[3]);
        fill_paint.anti_alias = true;

        let arc_path = if (sweep_rad - std::f32::consts::TAU).abs() < 1e-4 {
            let mut pb = resvg::tiny_skia::PathBuilder::new();
            pb.push_circle(ring.cx, ring.cy, ring.radius);
            pb.finish()
        } else {
            let mut pb = resvg::tiny_skia::PathBuilder::new();
            add_arc(
                &mut pb,
                ring.cx,
                ring.cy,
                ring.radius,
                -std::f32::consts::FRAC_PI_2,
                sweep_rad,
            );
            pb.finish()
        };

        if let Some(path) = arc_path {
            pixmap.stroke_path(
                &path,
                &fill_paint,
                &stroke,
                resvg::tiny_skia::Transform::identity(),
                None,
            );
        }
    }

    // 3. Glyph: rendered into ring interior box, centered and recolored
    let svg_str = match slot {
        Slot::Left if single_neutral => SVG_SINGLE,
        Slot::Left => SVG_LEFT,
        Slot::Right => SVG_RIGHT,
        Slot::Case => SVG_CASE,
    };

    let opt = resvg::usvg::Options::default();
    if let Ok(tree) = resvg::usvg::Tree::from_str(svg_str, &opt) {
        let box_size = glyph_box(size);
        let target_x = ring.cx - box_size / 2.0;
        let target_y = ring.cy - box_size / 2.0;
        let svg_w = tree.size().width();
        let svg_h = tree.size().height();
        let scale = (box_size / svg_w).min(box_size / svg_h);
        let transform = resvg::tiny_skia::Transform::from_scale(scale, scale)
            .post_translate(target_x, target_y);

        if let Some(mut glyph_pixmap) = resvg::tiny_skia::Pixmap::new(size, size) {
            resvg::render(&tree, transform, &mut glyph_pixmap.as_mut());

            for chunk in glyph_pixmap.data_mut().as_chunks_mut::<4>().0 {
                let a = chunk[3];
                if a > 0 {
                    chunk[0] = ((glyph_rgb[0] as u16 * a as u16 + 127) / 255) as u8;
                    chunk[1] = ((glyph_rgb[1] as u16 * a as u16 + 127) / 255) as u8;
                    chunk[2] = ((glyph_rgb[2] as u16 * a as u16 + 127) / 255) as u8;
                }
            }

            pixmap.draw_pixmap(
                0,
                0,
                glyph_pixmap.as_ref(),
                &resvg::tiny_skia::PixmapPaint::default(),
                resvg::tiny_skia::Transform::identity(),
                None,
            );
        }
    }

    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for p in pixmap.pixels() {
        let c = p.demultiply();
        out.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::status::BatteryView;
    use std::f32::consts::{FRAC_PI_2, PI, TAU};

    #[test]
    fn test_ring_geometry_inside_canvas() {
        for size in [16, 20, 24, 32] {
            let ring = ring_geometry(size);
            let half_stroke = ring.stroke / 2.0;
            assert!(
                ring.cx - ring.radius - half_stroke >= 0.0,
                "Ring extends past left: size={}, ring={:?}",
                size,
                ring
            );
            assert!(
                ring.cx + ring.radius + half_stroke <= size as f32,
                "Ring extends past right: size={}, ring={:?}",
                size,
                ring
            );
            assert!(
                ring.cy - ring.radius - half_stroke >= 0.0,
                "Ring extends past top: size={}, ring={:?}",
                size,
                ring
            );
            assert!(
                ring.cy + ring.radius + half_stroke <= size as f32,
                "Ring extends past bottom: size={}, ring={:?}",
                size,
                ring
            );
        }
    }

    #[test]
    fn test_sweep() {
        assert_eq!(sweep(0), 0.0);
        assert!((sweep(100) - TAU).abs() < 1e-5);
        assert!((sweep(150) - TAU).abs() < 1e-5);
        assert!((sweep(50) - PI).abs() < 1e-5);
        assert!(sweep(1) >= 0.15);
    }

    #[test]
    fn test_slot_value() {
        // Disconnected
        for slot in [Slot::Left, Slot::Right, Slot::Case] {
            assert_eq!(
                slot_value(slot, &DeviceStatus::Disconnected),
                (None, IconState::Disconnected)
            );
        }

        // Connecting
        for slot in [Slot::Left, Slot::Right, Slot::Case] {
            assert_eq!(
                slot_value(slot, &DeviceStatus::Connecting),
                (None, IconState::Connecting)
            );
        }

        // Connected(None)
        for slot in [Slot::Left, Slot::Right, Slot::Case] {
            assert_eq!(
                slot_value(slot, &DeviceStatus::connected_vendor(None)),
                (None, IconState::Live)
            );
        }

        // Connected(Some(view))
        let view = BatteryView {
            left: Some(86),
            right: Some(89),
            case: None,
        };
        assert_eq!(
            slot_value(Slot::Left, &DeviceStatus::connected_vendor(Some(view))),
            (Some(86), IconState::Live)
        );
        assert_eq!(
            slot_value(Slot::Right, &DeviceStatus::connected_vendor(Some(view))),
            (Some(89), IconState::Live)
        );
        assert_eq!(
            slot_value(Slot::Case, &DeviceStatus::connected_vendor(Some(view))),
            (None, IconState::Live)
        );

        let view_with_case = BatteryView {
            left: Some(40),
            right: Some(45),
            case: Some(90),
        };
        assert_eq!(
            slot_value(
                Slot::Case,
                &DeviceStatus::connected_vendor(Some(view_with_case))
            ),
            (Some(90), IconState::Live)
        );
    }

    fn sample_pixel(buf: &[u8], size: u32, x: usize, y: usize) -> [u8; 4] {
        let idx = (y * size as usize + x) * 4;
        [buf[idx], buf[idx + 1], buf[idx + 2], buf[idx + 3]]
    }

    fn ring_pixel(ring: Ring, angle: f32) -> (usize, usize) {
        let x = (ring.cx + ring.radius * angle.cos()).round() as usize;
        let y = (ring.cy + ring.radius * angle.sin()).round() as usize;
        let max = (ring.cx * 2.0).round() as usize - 2;
        (x.clamp(1, max), y.clamp(1, max))
    }

    #[test]
    fn test_render_slot_buffer_length_and_corner_transparent() {
        for size in [16, 32] {
            let buf = render_slot(Slot::Left, Some(100), IconState::Live, size);
            assert_eq!(buf.len(), (size * size * 4) as usize);
            let corner = sample_pixel(&buf, size, 0, 0);
            assert_eq!(
                corner[3], 0,
                "Corner pixel must be transparent at size {}",
                size
            );
        }
    }

    #[test]
    fn test_render_100_left_live() {
        for size in [16, 32] {
            let ring = ring_geometry(size);
            let buf = render_slot(Slot::Left, Some(100), IconState::Live, size);

            // 12 o'clock (-pi/2)
            let (x12, y12) = ring_pixel(ring, -FRAC_PI_2);
            let p12 = sample_pixel(&buf, size, x12, y12);
            assert_eq!(
                p12, COLOR_HIGH,
                "12 o'clock pixel should be green at size {}",
                size
            );

            // 6 o'clock (pi/2)
            let (x6, y6) = ring_pixel(ring, FRAC_PI_2);
            let p6 = sample_pixel(&buf, size, x6, y6);
            assert_eq!(
                p6, COLOR_HIGH,
                "6 o'clock pixel should be green at size {}",
                size
            );
        }
    }

    #[test]
    fn test_render_25_live() {
        for size in [16, 32] {
            let ring = ring_geometry(size);
            let buf = render_slot(Slot::Left, Some(25), IconState::Live, size);

            // 3 o'clock: point inside the 25% arc near 3 o'clock (just counter-clockwise of angle 0)
            let (x3, y3) = ring_pixel(ring, -0.1);
            let p3 = sample_pixel(&buf, size, x3, y3);
            let expected_fill = match level_for(25) {
                Level::Mid => COLOR_MID,
                _ => unreachable!(),
            };
            assert_eq!(
                p3, expected_fill,
                "3 o'clock pixel should be colored at size {}",
                size
            );

            // 9 o'clock (pi rad) is the dimmed base
            let (x9, y9) = ring_pixel(ring, PI);
            let p9 = sample_pixel(&buf, size, x9, y9);
            assert!(
                p9[3] < p3[3],
                "9 o'clock alpha ({}) should be below fill alpha ({}) at size {}",
                p9[3],
                p3[3],
                size
            );
            assert!(
                p9[3] > 0,
                "9 o'clock should have dimmed base ring at size {}",
                size
            );
        }
    }

    #[test]
    fn test_render_15_live() {
        for size in [16, 32] {
            let ring = ring_geometry(size);
            let buf = render_slot(Slot::Left, Some(15), IconState::Live, size);

            // 12 o'clock-ish pixel just clockwise of the top (-pi/2 + delta)
            let delta = sweep(15) * 0.5; // halfway along the 15% arc
            let (x_arc, y_arc) = ring_pixel(ring, -FRAC_PI_2 + delta);
            let p_arc = sample_pixel(&buf, size, x_arc, y_arc);
            assert_eq!(
                p_arc, COLOR_LOW,
                "Pixel along 15% arc should be red at size {}",
                size
            );
        }
    }

    #[test]
    fn test_render_none_value() {
        for size in [16, 32] {
            let ring = ring_geometry(size);
            let buf = render_slot(Slot::Left, None, IconState::Live, size);

            // Check sample pixels around the ring: none should have a level color
            for angle_step in 0..8 {
                let angle = angle_step as f32 * TAU / 8.0;
                let (x, y) = ring_pixel(ring, angle);
                let p = sample_pixel(&buf, size, x, y);
                assert_ne!(
                    p, COLOR_HIGH,
                    "Pixel at angle {} has HIGH color at size {}",
                    angle, size
                );
                assert_ne!(
                    p, COLOR_MID,
                    "Pixel at angle {} has MID color at size {}",
                    angle, size
                );
                assert_ne!(
                    p, COLOR_LOW,
                    "Pixel at angle {} has LOW color at size {}",
                    angle, size
                );
            }
        }
    }

    #[test]
    fn test_render_glyph_pixels() {
        for size in [16, 32] {
            let ring = ring_geometry(size);
            let box_size = glyph_box(size);
            let min_x = (ring.cx - box_size / 2.0).round() as usize;
            let max_x = (ring.cx + box_size / 2.0).round() as usize;
            let min_y = (ring.cy - box_size / 2.0).round() as usize;
            let max_y = (ring.cy + box_size / 2.0).round() as usize;

            for slot in [Slot::Left, Slot::Right, Slot::Case] {
                let buf = render_slot(slot, Some(50), IconState::Live, size);
                let mut found_opaque = false;
                for y in min_y..=max_y {
                    for x in min_x..=max_x {
                        let p = sample_pixel(&buf, size, x, y);
                        if p[3] > 200 {
                            found_opaque = true;
                            break;
                        }
                    }
                    if found_opaque {
                        break;
                    }
                }
                assert!(
                    found_opaque,
                    "Expected opaque glyph pixel inside inner box for slot {:?} at size {}",
                    slot, size
                );
            }
        }
    }

    #[test]
    fn test_render_disconnected_differs_from_live() {
        for size in [16, 32] {
            let buf_live = render_slot(Slot::Left, None, IconState::Live, size);
            let buf_disc = render_slot(Slot::Left, None, IconState::Disconnected, size);
            assert_ne!(
                buf_live, buf_disc,
                "Disconnected rendering must differ from Live at size {}",
                size
            );

            let ring = ring_geometry(size);
            let (x, y) = ring_pixel(ring, 0.0);
            let p_live = sample_pixel(&buf_live, size, x, y);
            let p_disc = sample_pixel(&buf_disc, size, x, y);
            assert_ne!(
                p_live, p_disc,
                "Ring color must differ between Live and Disconnected at size {}",
                size
            );
        }
    }

    #[test]
    fn test_render_base_ring_straight_alpha() {
        for size in [16, 32] {
            let ring = ring_geometry(size);
            let buf = render_slot(Slot::Left, None, IconState::Live, size);
            let (x9, y9) = ring_pixel(ring, PI);
            let p9 = sample_pixel(&buf, size, x9, y9);
            // Expected: D0D0D0 (208, 208, 208) with tolerance 2, alpha about 102 (tolerance 2)
            assert!(
                (p9[0] as i16 - 0xD0).abs() <= 2,
                "Red channel {} not near 0xD0 at size {} (got {:?})",
                p9[0],
                size,
                p9
            );
            assert!(
                (p9[1] as i16 - 0xD0).abs() <= 2,
                "Green channel {} not near 0xD0 at size {} (got {:?})",
                p9[1],
                size,
                p9
            );
            assert!(
                (p9[2] as i16 - 0xD0).abs() <= 2,
                "Blue channel {} not near 0xD0 at size {} (got {:?})",
                p9[2],
                size,
                p9
            );
            assert!(
                (p9[3] as i16 - 102).abs() <= 2,
                "Alpha channel {} not near 102 at size {} (got {:?})",
                p9[3],
                size,
                p9
            );
        }
    }

    #[test]
    fn test_slot_visible() {
        let profile_never = DeviceProfile {
            case: CaseReporting::Never,
        };
        let profile_with_bud = DeviceProfile {
            case: CaseReporting::WithBudInCase,
        };
        let profile_always = DeviceProfile {
            case: CaseReporting::Always,
        };
        let profiles = [profile_never, profile_with_bud, profile_always];

        let not_connected_statuses = [DeviceStatus::Connecting, DeviceStatus::Disconnected];

        // 1. Connecting & Disconnected:
        // Left & Right: hidden for all profiles
        // Case: shown for all profiles
        for status in &not_connected_statuses {
            for profile in &profiles {
                assert!(
                    !slot_visible(Slot::Left, status, profile),
                    "Left must be hidden for {:?}",
                    status
                );
                assert!(
                    !slot_visible(Slot::Right, status, profile),
                    "Right must be hidden for {:?}",
                    status
                );
                assert!(
                    slot_visible(Slot::Case, status, profile),
                    "Case must be shown for {:?}",
                    status
                );
            }
        }

        // 2. Connected(None):
        // Left & Right: shown for all profiles
        // Case: Never -> hidden, WithBudInCase -> hidden, Always -> shown
        let status_connected_none = DeviceStatus::connected_vendor(None);
        for profile in &profiles {
            assert!(
                slot_visible(Slot::Left, &status_connected_none, profile),
                "Left must be shown for Connected(None)"
            );
            assert!(
                slot_visible(Slot::Right, &status_connected_none, profile),
                "Right must be shown for Connected(None)"
            );
        }
        assert!(
            !slot_visible(Slot::Case, &status_connected_none, &profile_never),
            "Case must be hidden for Connected(None) under Never"
        );
        assert!(
            !slot_visible(Slot::Case, &status_connected_none, &profile_with_bud),
            "Case must be hidden for Connected(None) under WithBudInCase"
        );
        assert!(
            slot_visible(Slot::Case, &status_connected_none, &profile_always),
            "Case must be shown for Connected(None) under Always"
        );

        // 3. Connected(Some(v)):
        // Left & Right: shown for all profiles
        // Case: Never -> hidden
        //       WithBudInCase -> shown if v.case.is_some()
        //       Always -> shown
        let view_no_case = BatteryView {
            left: Some(80),
            right: Some(85),
            case: None,
        };
        let view_with_case = BatteryView {
            left: Some(80),
            right: Some(85),
            case: Some(50),
        };

        for view in [&view_no_case, &view_with_case] {
            let status = DeviceStatus::connected_vendor(Some(*view));
            for profile in &profiles {
                assert!(
                    slot_visible(Slot::Left, &status, profile),
                    "Left must be shown for Connected(Some) with view {:?}",
                    view
                );
                assert!(
                    slot_visible(Slot::Right, &status, profile),
                    "Right must be shown for Connected(Some) with view {:?}",
                    view
                );
            }

            // Case under Never: always hidden
            assert!(
                !slot_visible(Slot::Case, &status, &profile_never),
                "Case must be hidden under Never for view {:?}",
                view
            );

            // Case under Always: always shown
            assert!(
                slot_visible(Slot::Case, &status, &profile_always),
                "Case must be shown under Always for view {:?}",
                view
            );
        }

        // Case under WithBudInCase: shown iff v.case.is_some()
        assert!(
            !slot_visible(
                Slot::Case,
                &DeviceStatus::connected_vendor(Some(view_no_case)),
                &profile_with_bud
            ),
            "Case must be hidden under WithBudInCase when case is None"
        );
        assert!(
            slot_visible(
                Slot::Case,
                &DeviceStatus::connected_vendor(Some(view_with_case)),
                &profile_with_bud
            ),
            "Case must be shown under WithBudInCase when case is Some"
        );

        // 4. Source::Windows and Source::FastPairHfp: only Left visible, Right and Case hidden
        for source in [
            crate::app::status::Source::Windows,
            crate::app::status::Source::FastPairHfp,
        ] {
            let status = DeviceStatus::Connected {
                view: Some(view_with_case),
                source,
            };
            for profile in &profiles {
                assert!(
                    slot_visible(Slot::Left, &status, profile),
                    "Left must be visible for source {:?}",
                    source
                );
                assert!(
                    !slot_visible(Slot::Right, &status, profile),
                    "Right must be hidden for source {:?}",
                    source
                );
                assert!(
                    !slot_visible(Slot::Case, &status, profile),
                    "Case must be hidden for source {:?}",
                    source
                );
            }
        }

        // 5. Source::FastPair: Left and Right always visible, Case only visible if Some
        let fp_no_case = DeviceStatus::Connected {
            view: Some(view_no_case),
            source: crate::app::status::Source::FastPair {
                model_id: None,
                case_last_known: false,
            },
        };
        let fp_with_case = DeviceStatus::Connected {
            view: Some(view_with_case),
            source: crate::app::status::Source::FastPair {
                model_id: None,
                case_last_known: false,
            },
        };
        let fp_with_last_known_case = DeviceStatus::Connected {
            view: Some(view_with_case),
            source: crate::app::status::Source::FastPair {
                model_id: None,
                case_last_known: true,
            },
        };
        for profile in &profiles {
            assert!(slot_visible(Slot::Left, &fp_no_case, profile));
            assert!(slot_visible(Slot::Right, &fp_no_case, profile));
            assert!(
                !slot_visible(Slot::Case, &fp_no_case, profile),
                "FastPair Case must be hidden when None"
            );

            assert!(slot_visible(Slot::Left, &fp_with_case, profile));
            assert!(slot_visible(Slot::Right, &fp_with_case, profile));
            assert!(
                slot_visible(Slot::Case, &fp_with_case, profile),
                "FastPair Case must be shown when Some"
            );

            assert!(slot_visible(Slot::Left, &fp_with_last_known_case, profile));
            assert!(slot_visible(Slot::Right, &fp_with_last_known_case, profile));
            assert!(
                slot_visible(Slot::Case, &fp_with_last_known_case, profile),
                "FastPair Case must be shown when Some (last known)"
            );
        }
    }

    #[test]
    fn test_render_slot_with_style_single_neutral() {
        for size in [16, 32] {
            let buf_stem =
                render_slot_with_style(Slot::Left, Some(80), IconState::Live, size, false);
            let buf_neutral =
                render_slot_with_style(Slot::Left, Some(80), IconState::Live, size, true);

            assert_eq!(buf_stem.len(), (size * size * 4) as usize);
            assert_eq!(buf_neutral.len(), (size * size * 4) as usize);
            assert_ne!(
                buf_stem, buf_neutral,
                "Neutral glyph rendering must differ from stem left at size {}",
                size
            );
        }
    }

    #[test]
    fn test_glyph_box() {
        // 11 px touched the ring, back to the Phase 4 size.
        assert_eq!(glyph_box(16), 9.0);
        assert_eq!(glyph_box(32), 19.0);
    }
}
