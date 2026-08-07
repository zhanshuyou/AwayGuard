//! The menu bar icon, drawn from the AwayGuard design's "Menu bar icon —
//! template" sheet.
//!
//! Two rules from that sheet drive everything here:
//!
//! * **Shape carries state** — open · closed · lifting · slashed.
//! * **Weight carries the switch** — a hairline outline while the lock switch
//!   is off, a solid body once it is on.
//!
//! No colour and no badge dots, because the asset is a macOS *template*: the
//! system tints it for light and dark menu bars and only the alpha channel we
//! draw survives. That is also the accessibility argument the design makes —
//! the glyph has to read filled / open / slashed with every pixel the same
//! colour, so it survives monochrome rendering and colour blindness.
//!
//! The glyph is rasterised rather than shipped as PNGs so the five states
//! cannot drift apart: they are one geometry with three parameters (where the
//! shackle sits, whether the body is solid, whether a slash crosses it).

use crate::monitor::MonitorStatus;
use crate::proximity::Presence;
use tauri::image::Image;

/// Side of the design's artboard, in design units. Every constant below is
/// measured against this cell, exactly as the design sheet's 22 pt variant is.
const CELL: f32 = 22.0;

/// Pixel side of the rendered icon.
///
/// `tray-icon` scales whatever square image it is handed to 18 pt tall in the
/// menu bar, so 36 px is precisely @2x on a Retina display — no resampling,
/// no softened hairlines. The design's 22 pt cell therefore lands at 18 pt,
/// which keeps the glyph itself around 14.7 pt wide: menu-bar sized, with the
/// breathing room the artboard builds in.
const ICON_PX: u32 = 36;

/// Stroke weight of every hairline: the shackle always, the body while the
/// lock switch is off. Drawn inside the shape's bounds, like a CSS border.
const STROKE: f32 = 2.0;

const CENTER_X: f32 = CELL / 2.0;
const CENTER_Y: f32 = CELL / 2.0;

const BODY_W: f32 = 14.0;
const BODY_H: f32 = 10.0;
const BODY_R: f32 = 2.0;
/// The glyph is bottom-aligned in the artboard's 16.5-unit content box.
const BODY_BOTTOM: f32 = 19.25;
const BODY_TOP: f32 = BODY_BOTTOM - BODY_H;

/// Half the shackle's width, and also its corner radius — the arch's top is a
/// true semicircle.
const SHACKLE_R: f32 = 4.5;
const SHACKLE_TOP: f32 = 5.75;
/// The shackle's legs run 2 units *into* the body, so a solid body swallows
/// them and a hairline body meets them exactly at its inner edge.
const SHACKLE_BOTTOM: f32 = SHACKLE_TOP + 5.5;

/// How far the shackle slides sideways when the lock hangs open.
const OPEN_SHIFT: f32 = 5.0;
/// How far it rises while a departure is pending — the lock caught mid-open.
const LIFT: f32 = 4.5;

const SLASH_LEN: f32 = 23.5;
const SLASH_W: f32 = 2.0;
const SLASH_DEGREES: f32 = -40.0;

/// Opacity of the "no device" glyph. Dimming says "nothing is being watched"
/// without inventing a sixth shape.
const DIM: f32 = 0.4;

/// The five menu bar states from the design sheet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayGlyph {
    /// No device picked yet — open shackle, dimmed.
    NoDevice,
    /// Lock switch off — open shackle, hairline body.
    Disarmed,
    /// Armed and the phone is near — closed shackle, solid body.
    Armed,
    /// Armed and the phone has left — the shackle lifts.
    Pending,
    /// The sensing chain is broken — slashed.
    Failing,
}

impl TrayGlyph {
    /// Which glyph a status snapshot should show.
    ///
    /// The order of these tests is the priority order. A broken sensing chain
    /// outranks the lock switch on purpose: while the monitor cannot see the
    /// phone, an "armed" glyph would promise protection the app is not
    /// providing, and a silent failure must never look identical to safe.
    /// "No device" outranks even that, because the missing device *is* the
    /// error in that case and the design gives it its own, quieter shape.
    pub fn for_status(status: &MonitorStatus, has_target: bool) -> Self {
        if !has_target {
            return Self::NoDevice;
        }
        if status.error.is_some() {
            return Self::Failing;
        }
        if !status.armed {
            return Self::Disarmed;
        }
        // A countdown is the clearest "pending", but the shackle should also
        // lift the moment a departure is confirmed with the grace period set
        // to zero, where there is never a countdown to observe.
        if status.grace_remaining.is_some() || status.presence == Presence::Away {
            return Self::Pending;
        }
        Self::Armed
    }

    /// Where the shackle sits relative to the closed position.
    fn shackle_offset(self) -> (f32, f32) {
        match self {
            Self::NoDevice | Self::Disarmed => (OPEN_SHIFT, 0.0),
            Self::Pending => (0.0, -LIFT),
            Self::Armed | Self::Failing => (0.0, 0.0),
        }
    }

    /// Solid body once the lock switch is on and sensing works; hairline
    /// otherwise.
    fn body_is_solid(self) -> bool {
        matches!(self, Self::Armed | Self::Pending)
    }

    fn alpha(self) -> f32 {
        match self {
            Self::NoDevice => DIM,
            _ => 1.0,
        }
    }

    /// Signed distance from a point in design space to the whole glyph:
    /// negative inside, positive outside, in design units.
    fn distance(self, x: f32, y: f32) -> f32 {
        let (dx, dy) = self.shackle_offset();
        let mut d = sd_shackle(x - dx, y - dy);
        d = d.min(sd_body(x, y, self.body_is_solid()));
        if self == Self::Failing {
            d = d.min(sd_slash(x, y));
        }
        d
    }

    /// Renders the glyph as a macOS template image.
    pub fn image(self) -> Image<'static> {
        let alpha = self.alpha();
        // Width of one pixel measured in design units — the unit the signed
        // distances come back in, and so the unit the edge has to be
        // feathered over.
        let unit = CELL / ICON_PX as f32;
        let mut rgba = vec![0u8; (ICON_PX * ICON_PX * 4) as usize];

        for row in 0..ICON_PX {
            for col in 0..ICON_PX {
                let x = (col as f32 + 0.5) * unit;
                let y = (row as f32 + 0.5) * unit;
                // Distance-to-coverage antialiasing: a pixel whose centre sits
                // exactly on the edge is half covered, and coverage runs out
                // over the pixel's own width either side of it.
                let coverage = (0.5 - self.distance(x, y) / unit).clamp(0.0, 1.0);
                let i = ((row * ICON_PX + col) * 4) as usize;
                // RGB stays black: a template image is tinted by the system
                // from its alpha alone, so the colour channels are never read.
                rgba[i + 3] = (coverage * alpha * 255.0).round() as u8;
            }
        }

        Image::new_owned(rgba, ICON_PX, ICON_PX)
    }
}

/// Signed distance to an axis-aligned rounded rectangle.
fn sd_round_rect(x: f32, y: f32, cx: f32, cy: f32, hw: f32, hh: f32, r: f32) -> f32 {
    let qx = (x - cx).abs() - (hw - r);
    let qy = (y - cy).abs() - (hh - r);
    (qx.max(0.0).hypot(qy.max(0.0))) + qx.max(qy).min(0.0) - r
}

/// Signed distance to an arch: a rectangle `2r` wide whose top is capped by a
/// semicircle and whose bottom edge is square.
fn sd_arch(x: f32, y: f32, top: f32, bottom: f32, r: f32) -> f32 {
    let cap = top + r;
    if y < cap {
        (x - CENTER_X).hypot(y - cap) - r
    } else {
        ((x - CENTER_X).abs() - r).max(y - bottom)
    }
}

/// The shackle: an arch hollowed out by a second arch inset by one stroke on
/// the sides and top, leaving the bottom open so the legs run into the body.
fn sd_shackle(x: f32, y: f32) -> f32 {
    let outer = sd_arch(x, y, SHACKLE_TOP, SHACKLE_BOTTOM, SHACKLE_R);
    let inner = sd_arch(
        x,
        y,
        SHACKLE_TOP + STROKE,
        SHACKLE_BOTTOM,
        SHACKLE_R - STROKE,
    );
    outer.max(-inner)
}

fn sd_body(x: f32, y: f32, solid: bool) -> f32 {
    let cy = (BODY_TOP + BODY_BOTTOM) / 2.0;
    let outer = sd_round_rect(x, y, CENTER_X, cy, BODY_W / 2.0, BODY_H / 2.0, BODY_R);
    if solid {
        return outer;
    }
    let inner = sd_round_rect(
        x,
        y,
        CENTER_X,
        cy,
        BODY_W / 2.0 - STROKE,
        BODY_H / 2.0 - STROKE,
        (BODY_R - STROKE).max(0.0),
    );
    outer.max(-inner)
}

/// The error slash, a bar rotated across the whole glyph.
fn sd_slash(x: f32, y: f32) -> f32 {
    let (sin, cos) = SLASH_DEGREES.to_radians().sin_cos();
    let dx = x - CENTER_X;
    let dy = y - CENTER_Y;
    // Rotate the sample point into the bar's own frame rather than rotating
    // the bar, which keeps the distance function axis-aligned.
    let lx = dx * cos + dy * sin;
    let ly = -dx * sin + dy * cos;
    sd_round_rect(lx, ly, 0.0, 0.0, SLASH_LEN / 2.0, SLASH_W / 2.0, 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [TrayGlyph; 5] = [
        TrayGlyph::NoDevice,
        TrayGlyph::Disarmed,
        TrayGlyph::Armed,
        TrayGlyph::Pending,
        TrayGlyph::Failing,
    ];

    fn status(armed: bool, presence: Presence, error: Option<&str>) -> MonitorStatus {
        MonitorStatus {
            presence,
            rssi: None,
            armed,
            error: error.map(str::to_string),
            grace_remaining: None,
            last_seen: None,
            pending: None,
            pending_samples: 0,
            poll_interval: 2,
        }
    }

    /// Alpha channel of the rendered glyph, as one row-major plane.
    fn alpha_plane(glyph: TrayGlyph) -> Vec<u8> {
        let image = glyph.image();
        image.rgba().iter().skip(3).step_by(4).copied().collect()
    }

    fn alpha_at(plane: &[u8], x: f32, y: f32) -> u8 {
        let scale = ICON_PX as f32 / CELL;
        let col = (x * scale) as usize;
        let row = (y * scale) as usize;
        plane[row * ICON_PX as usize + col]
    }

    #[test]
    fn no_device_wins_even_though_it_is_reported_as_an_error() {
        // The polling loop reports "no device selected" through the same
        // `error` field a dead Bluetooth adapter uses, so priority order --
        // not just presence of an error -- is what keeps these apart.
        let s = status(true, Presence::Unknown, Some("no device selected"));
        assert_eq!(TrayGlyph::for_status(&s, false), TrayGlyph::NoDevice);
    }

    #[test]
    fn a_sensing_failure_outranks_the_lock_switch() {
        let s = status(true, Presence::Near, Some("bluetooth adapter unavailable"));
        assert_eq!(TrayGlyph::for_status(&s, true), TrayGlyph::Failing);
    }

    #[test]
    fn the_switch_and_presence_pick_the_remaining_three() {
        assert_eq!(
            TrayGlyph::for_status(&status(false, Presence::Near, None), true),
            TrayGlyph::Disarmed
        );
        assert_eq!(
            TrayGlyph::for_status(&status(true, Presence::Near, None), true),
            TrayGlyph::Armed
        );
        assert_eq!(
            TrayGlyph::for_status(&status(true, Presence::Away, None), true),
            TrayGlyph::Pending
        );
    }

    #[test]
    fn a_countdown_lifts_the_shackle_even_before_the_departure_confirms() {
        let mut s = status(true, Presence::Near, None);
        s.grace_remaining = Some(4);
        assert_eq!(TrayGlyph::for_status(&s, true), TrayGlyph::Pending);
    }

    #[test]
    fn every_glyph_draws_something_inside_its_bounds() {
        for glyph in ALL {
            let plane = alpha_plane(glyph);
            assert_eq!(plane.len(), (ICON_PX * ICON_PX) as usize);
            assert!(
                plane.iter().any(|&a| a > 0),
                "{glyph:?} rendered nothing at all"
            );
            // Nothing may touch the artboard edge, or the menu bar would clip
            // it against its neighbours.
            let px = ICON_PX as usize;
            for i in 0..px {
                for &a in &[
                    plane[i],
                    plane[(px - 1) * px + i],
                    plane[i * px],
                    plane[i * px + px - 1],
                ] {
                    assert_eq!(a, 0, "{glyph:?} bleeds into the artboard edge");
                }
            }
        }
    }

    #[test]
    fn the_body_is_solid_only_once_the_lock_switch_is_on() {
        // The centre of the body: filled by a solid body, hollow inside a
        // hairline one. The error slash crosses the body well left of here,
        // so it does not muddy the sample.
        let middle_y = (BODY_TOP + BODY_BOTTOM) / 2.0;
        for glyph in ALL {
            let a = alpha_at(&alpha_plane(glyph), CENTER_X, middle_y);
            if glyph.body_is_solid() {
                assert!(a > 200, "{glyph:?} should have a solid body, got {a}");
            } else {
                assert_eq!(a, 0, "{glyph:?} should have a hollow body");
            }
        }
    }

    #[test]
    fn an_open_shackle_hangs_clear_of_the_closed_position() {
        // Crown of the arch in its closed position. Sampling the legs instead
        // would prove nothing: a hairline body's top border runs straight
        // through them, so those pixels are lit either way.
        let crown_y = SHACKLE_TOP + STROKE / 2.0;
        assert!(alpha_at(&alpha_plane(TrayGlyph::Armed), CENTER_X, crown_y) > 200);
        for glyph in [TrayGlyph::Disarmed, TrayGlyph::NoDevice] {
            let plane = alpha_plane(glyph);
            assert_eq!(
                alpha_at(&plane, CENTER_X, crown_y),
                0,
                "{glyph:?} should have swung its shackle aside"
            );
            // ...and it has to actually be over there, not simply gone.
            assert!(alpha_at(&plane, CENTER_X + OPEN_SHIFT, crown_y) > 0);
        }
    }

    #[test]
    fn a_pending_departure_lifts_the_shackle_without_opening_it() {
        // The crown, one lift higher: bare while the lock is shut, covered
        // once the shackle has risen -- and risen straight up, not aside.
        let y = SHACKLE_TOP + STROKE / 2.0 - LIFT;
        assert_eq!(alpha_at(&alpha_plane(TrayGlyph::Armed), CENTER_X, y), 0);
        assert!(alpha_at(&alpha_plane(TrayGlyph::Pending), CENTER_X, y) > 200);
    }

    #[test]
    fn the_dimmed_glyph_is_the_armed_one_at_reduced_opacity() {
        // Same shape as Disarmed, only fainter -- the design dims rather than
        // redrawing, so the two must agree pixel for pixel up to the scale.
        let disarmed = alpha_plane(TrayGlyph::Disarmed);
        let no_device = alpha_plane(TrayGlyph::NoDevice);
        for (bright, dim) in disarmed.iter().zip(&no_device) {
            let expected = (f32::from(*bright) * DIM).round() as u8;
            assert!(
                dim.abs_diff(expected) <= 1,
                "expected {expected} at {DIM} opacity, got {dim}"
            );
        }
        assert!(no_device.iter().any(|&a| a > 0));
    }

    #[test]
    fn all_five_glyphs_are_visually_distinct() {
        let planes: Vec<_> = ALL.iter().map(|g| alpha_plane(*g)).collect();
        for (i, a) in planes.iter().enumerate() {
            for (j, b) in planes.iter().enumerate().skip(i + 1) {
                assert_ne!(a, b, "{:?} and {:?} render identically", ALL[i], ALL[j]);
            }
        }
    }
}
