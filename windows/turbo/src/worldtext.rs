//! The floating combat-text animation kernel: pure curves over ticks and rects.
//!
//! Ports the animation model of the reference combat-text overlay this mod
//! replaces: rise/fall/arc float curves with a late fade, crit pop-in with an
//! exponentially damped shake and a slot carousel that pushes earlier crits
//! outward, and the overlap fast-forward that keeps stacked lines readable.
//! Time arrives as integer ticks and is quantized to 180 virtual frames per
//! second, the same integer division of a performance counter the reference
//! performs; geometry is screen-space pixel rects. No game state is read
//! here — the adapter projects anchors and supplies text metrics — so every
//! curve is host-testable. One deliberate deviation: lifetime ends when the
//! quantized frame count reaches the total, where the reference compares raw
//! ticks against `total * frequency`; the two disagree by less than one
//! virtual frame.

/// Float lifetime (1.9 s) and fade start (1.3 s), in exact virtual frames.
///
/// Stored as the frame products rather than seconds so the quantized end
/// test compares exactly instead of against a `seconds * 180` rounding.
const FLOAT_TOTAL_FRAMES: f64 = 342.0;
/// See [`FLOAT_TOTAL_FRAMES`].
const FLOAT_FADE_FRAMES: f64 = 234.0;

/// Crit lifetime (2.2 s) and fade start (1.6 s), in exact virtual frames.
const CRIT_TOTAL_FRAMES: f64 = 396.0;
/// See [`CRIT_TOTAL_FRAMES`].
const CRIT_FADE_FRAMES: f64 = 288.0;
/// Crit pop-in duration (0.2 s): the centered small-face ramp, in frames.
const CRIT_FADE_IN_FRAMES: f64 = 36.0;
/// Crit shake and push duration (0.3 s), in frames.
const CRIT_IMPULSE_FRAMES: f64 = 54.0;

/// A screen-space pixel rect, exclusive on the right/bottom edge.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Rect {
    /// Left edge, pixels.
    pub left: i32,
    /// Top edge, pixels.
    pub top: i32,
    /// Right edge, pixels.
    pub right: i32,
    /// Bottom edge, pixels.
    pub bottom: i32,
}

impl Rect {
    /// The rect's height in pixels.
    pub const fn height(&self) -> i32 {
        self.bottom - self.top
    }

    /// Whether the two rects overlap at all.
    pub const fn intersects(&self, other: &Self) -> bool {
        self.left < other.right
            && other.left < self.right
            && self.top < other.bottom
            && other.top < self.bottom
    }

    /// The height of the overlap with `other`, `0` when they do not touch.
    pub const fn overlap_height(&self, other: &Self) -> i32 {
        if self.intersects(other) {
            let top = if self.top > other.top {
                self.top
            } else {
                other.top
            };
            let bottom = if self.bottom < other.bottom {
                self.bottom
            } else {
                other.bottom
            };
            bottom - top
        } else {
            0
        }
    }
}

/// Elapsed virtual frames since `start`, the reference's quantized division.
///
/// Signed on purpose: the overlap fast-forward backdates starts, and a start
/// may legitimately sit before tick zero of the process clock.
fn elapsed_frames(now: i64, start: i64, ticks_per_frame: i64) -> f64 {
    let frames = (now - start).max(0) / ticks_per_frame;
    // A lifetime is a few hundred frames; anything beyond the cap is already
    // long past every curve's end, so saturating keeps the cast exact.
    f64::from(u32::try_from(frames).unwrap_or(u32::MAX))
}

/// Which way a floating line travels over its lifetime.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Direction {
    /// Rises; the default for damage, heals and resists.
    Up,
    /// Falls; incoming ticks the reference draws downward.
    // Constructed only by the 32-bit adapter's script command; the host
    // test build has no caller for it.
    #[cfg_attr(not(target_arch = "x86"), allow(dead_code))]
    Down,
    /// Sweeps a quarter arc sideways while rising.
    Arc,
}

/// One animation step's outcome for a floating line.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Tick {
    /// Lifetime over; the entry is dropped.
    End,
    /// Alive but not drawn this frame (anchor lost or sight blocked).
    Hidden,
    /// Draw at `rect` with the line's colour scaled by `alpha`.
    Draw {
        /// Where the text lands this frame.
        rect: Rect,
        /// Global opacity in `[0, 1]`, applied to fill and shadow alike.
        alpha: f64,
    },
}

/// Creation-time environment for a floating line, computed by the adapter.
pub struct FloatSpec {
    /// Rendered text width in pixels.
    pub width: i32,
    /// Rendered text height in pixels.
    pub height: i32,
    /// Total travel distance in pixels (already resolution- and self-scaled).
    pub floating_distance: i32,
    /// Arc sweep radius in pixels, only read by [`Direction::Arc`].
    pub arc_radius: f64,
    /// Which side an arc sweeps toward; the adapter alternates per line.
    pub arc_towards_right: bool,
    /// Creation instant in ticks.
    pub start_ticks: i64,
    /// Ticks per virtual frame (`frequency / 180`, integer division).
    pub ticks_per_frame: i64,
}

/// A floating combat-text line's animation state.
pub struct Floating {
    width: i32,
    height: i32,
    travel_distance: i32,
    arc_radius: f64,
    arc_sign: f64,
    start_ticks: i64,
    ticks_per_frame: i64,
    direction: Direction,
    rect: Rect,
}

impl Floating {
    /// A new line; the rect is provisional until the first [`Self::tick`].
    pub fn new(spec: &FloatSpec, direction: Direction) -> Self {
        Self {
            width: spec.width,
            height: spec.height,
            travel_distance: spec.floating_distance.max(1),
            arc_radius: spec.arc_radius,
            arc_sign: if spec.arc_towards_right { 1.0 } else { -1.0 },
            start_ticks: spec.start_ticks,
            ticks_per_frame: spec.ticks_per_frame.max(1),
            direction,
            rect: Rect {
                left: 0,
                top: 0,
                right: spec.width,
                bottom: spec.height,
            },
        }
    }

    /// The rect of the last drawn (or provisional) position.
    pub const fn rect(&self) -> Rect {
        self.rect
    }

    /// Advance to `now`; `anchor` is the projected stick point, if any.
    ///
    /// The rect updates whenever an anchor exists, even when the line ends up
    /// hidden by the sight test — the reference does the same so overlap
    /// bookkeeping keeps working while a unit is briefly obscured.
    pub fn tick(&mut self, now: i64, anchor: Option<(i32, i32)>, in_sight: bool) -> Tick {
        let elapsed = elapsed_frames(now, self.start_ticks, self.ticks_per_frame);
        let total = FLOAT_TOTAL_FRAMES;
        if elapsed >= total {
            return Tick::End;
        }
        let Some((ax, ay)) = anchor else {
            return Tick::Hidden;
        };

        let step = elapsed / total;
        let travelled = step * f64::from(self.travel_distance);
        let (dx, dy) = match self.direction {
            Direction::Up => (0, -truncate(travelled)),
            Direction::Down => (0, truncate(travelled)),
            Direction::Arc => {
                let quarter = core::f64::consts::FRAC_PI_2 * step;
                let x = self.arc_sign * (self.arc_radius * libm::cos(quarter) - self.arc_radius);
                let y = self.arc_radius * libm::sin(quarter);
                (truncate(x), -truncate(y))
            }
        };
        self.rect = Rect {
            left: ax - self.width / 2 + dx,
            top: ay - self.height + dy,
            right: ax + self.width / 2 + dx,
            bottom: ay + dy,
        };

        let fade_start = FLOAT_FADE_FRAMES;
        let alpha = if elapsed > fade_start {
            (1.0 - (elapsed - fade_start) / (total - fade_start)).max(0.0)
        } else {
            1.0
        };

        if in_sight {
            Tick::Draw {
                rect: self.rect,
                alpha,
            }
        } else {
            Tick::Hidden
        }
    }

    /// Rewind the start so the line has already travelled `distance` pixels.
    ///
    /// Backdates in whole virtual frames like the reference, so a
    /// fast-forwarded line stays on the same quantized curve.
    pub fn fast_forward(&mut self, distance: i32) {
        let frames = f64::from(distance) / f64::from(self.travel_distance) * FLOAT_TOTAL_FRAMES;
        let whole = i64::from(truncate(frames));
        self.start_ticks -= whole * self.ticks_per_frame;
    }
}

/// Which of the two crit faces this frame draws with.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CritFont {
    /// The base face, shown centered during the pop-in ramp.
    Normal,
    /// The larger face the crit settles into.
    Big,
}

/// One animation step's outcome for a crit.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum CritTick {
    /// Lifetime over; the entry is dropped.
    End,
    /// Alive but not drawn this frame.
    Hidden,
    /// Draw at `rect` with `alpha`, using `font`, centered when `centered`.
    Draw {
        /// Where the text lands this frame.
        rect: Rect,
        /// Global opacity in `[0, 1]`.
        alpha: f64,
        /// Which face to draw with this frame.
        font: CritFont,
        /// Pop-in frames center the small face inside the big-face rect.
        centered: bool,
    },
}

/// Creation-time environment for a crit, computed by the adapter.
pub struct CritSpec {
    /// Text width in pixels, measured at the largest face like the reference.
    pub width: i32,
    /// Text height in pixels, measured at the largest face.
    pub height: i32,
    /// Creation instant in ticks.
    pub start_ticks: i64,
    /// Ticks per virtual frame.
    pub ticks_per_frame: i64,
}

/// A single crit line: pop-in, damped shake, and at most one outward push.
pub struct Crit {
    width: i32,
    height: i32,
    start_ticks: i64,
    ticks_per_frame: i64,
    push_end: (i32, i32),
    push_start_ticks: i64,
    rect: Rect,
}

impl Crit {
    /// A new crit; the rect is provisional until the first [`Self::tick`].
    pub fn new(spec: &CritSpec) -> Self {
        Self {
            width: spec.width,
            height: spec.height,
            start_ticks: spec.start_ticks,
            ticks_per_frame: spec.ticks_per_frame.max(1),
            push_end: (0, 0),
            push_start_ticks: 0,
            // A one-pixel seed like the reference's, so the first frame's
            // shake amplitude is nil rather than a full-height jolt.
            rect: Rect {
                left: 0,
                top: 0,
                right: 1,
                bottom: 1,
            },
        }
    }

    /// The rect of the last drawn (or provisional) position.
    pub const fn rect(&self) -> Rect {
        self.rect
    }

    /// Start the outward push toward the slot offset `to`.
    pub const fn push(&mut self, to: (i32, i32), now: i64) {
        self.push_end = to;
        self.push_start_ticks = now;
    }

    /// Advance to `now`; `anchor` is the projected stick point, if any.
    pub fn tick(&mut self, now: i64, anchor: Option<(i32, i32)>, in_sight: bool) -> CritTick {
        let elapsed = elapsed_frames(now, self.start_ticks, self.ticks_per_frame);
        let total = CRIT_TOTAL_FRAMES;
        if elapsed >= total {
            return CritTick::End;
        }
        let Some((ax, ay)) = anchor else {
            return CritTick::Hidden;
        };

        let fade_in = CRIT_FADE_IN_FRAMES;
        let (font, centered, alpha) = if elapsed < fade_in {
            (CritFont::Normal, true, (elapsed / fade_in).clamp(0.0, 1.0))
        } else {
            let fade_start = CRIT_FADE_FRAMES;
            let alpha = if elapsed > fade_start {
                (1.0 - (elapsed - fade_start) / (total - fade_start)).clamp(0.0, 1.0)
            } else {
                1.0
            };
            (CritFont::Big, false, alpha)
        };

        let impulse = CRIT_IMPULSE_FRAMES;
        let (shake_x, shake_y) = if elapsed < impulse {
            let t = elapsed / impulse;
            let power = libm::exp(-2.0 * t);
            let wave_x = libm::cos(core::f64::consts::PI * 2.0 * t);
            let wave_y = libm::sin(core::f64::consts::PI * 2.0 * t * 3.0);
            // The reference derives the amplitude from the previous frame's
            // rect height, which is why the seed rect above is one pixel.
            let amplitude = f64::from(self.rect.height()) / 6.0;
            (
                truncate(power * wave_x * amplitude),
                truncate(power * wave_y * amplitude),
            )
        } else {
            (0, 0)
        };

        let push_elapsed = elapsed_frames(now, self.push_start_ticks, self.ticks_per_frame);
        let push = (push_elapsed / impulse).min(1.0);
        let push_x = truncate(push * f64::from(self.push_end.0));
        let push_y = truncate(push * f64::from(self.push_end.1));

        self.rect = Rect {
            left: ax - self.width / 2 + push_x + shake_x,
            top: ay - self.height + push_y + shake_y,
            right: ax + self.width / 2 + push_x + shake_x,
            bottom: ay + push_y + shake_y,
        };

        if in_sight {
            CritTick::Draw {
                rect: self.rect,
                alpha,
                font,
                centered,
            }
        } else {
            CritTick::Hidden
        }
    }
}

/// The carousel slot a crit occupies around its unit.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Slot {
    /// The newest crit, at the anchor.
    Center,
    /// First ring, pushed sideways.
    Left,
    /// See [`Slot::Left`].
    Right,
    /// Second ring, pushed up and sideways.
    LeftTop,
    /// See [`Slot::LeftTop`].
    RightTop,
    /// Third ring, pushed down and sideways.
    LeftBottom,
    /// See [`Slot::LeftBottom`].
    RightBottom,
}

/// A per-unit crit group: the newest sits center, older ones push outward.
///
/// The reference picks the side of each ring at random; this port takes a
/// caller-supplied coin instead so the carousel is deterministic under test
/// (the adapter feeds it an alternating counter). `P` is an opaque per-crit
/// payload the adapter hangs render resources on; it drops with its crit.
pub struct CritsGroup<P> {
    entries: Vec<GroupEntry<P>>,
    push_width: i32,
    push_height: i32,
}

/// One carousel occupant.
struct GroupEntry<P> {
    slot: Slot,
    crit: Crit,
    payload: P,
    visible: bool,
}

impl<P> CritsGroup<P> {
    /// A new group; push distances come from a reference-string measurement.
    pub const fn new(push_width: i32, push_height: i32) -> Self {
        Self {
            entries: Vec::new(),
            push_width,
            push_height,
        }
    }

    /// Whether the group holds no live crits (a test-side probe).
    #[cfg(test)]
    fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The number of live crits in the group (a test-side probe).
    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries.len()
    }

    /// The crit occupying `slot`, if any.
    fn position(&self, slot: Slot) -> Option<usize> {
        self.entries.iter().position(|e| e.slot == slot)
    }

    /// Add a crit: it takes the center, the incumbent is pushed outward.
    ///
    /// The ladder fills the side ring, then the top ring, then the bottom
    /// ring; `coin` picks the side wherever both are free. With every slot
    /// taken, the incumbent center is simply replaced.
    pub fn add(&mut self, crit: Crit, payload: P, coin: bool, now: i64) {
        let Some(center) = self.position(Slot::Center) else {
            self.entries.push(GroupEntry {
                slot: Slot::Center,
                crit,
                payload,
                visible: true,
            });
            return;
        };

        let rings = [
            (
                Slot::Left,
                Slot::Right,
                (-self.push_width, 0),
                (self.push_width, 0),
            ),
            (
                Slot::LeftTop,
                Slot::RightTop,
                (-self.push_width, -self.push_height),
                (self.push_width, -self.push_height),
            ),
            (
                Slot::LeftBottom,
                Slot::RightBottom,
                (-self.push_width, self.push_height),
                (self.push_width, self.push_height),
            ),
        ];
        for (left, right, left_push, right_push) in rings {
            let (slot, push) = match (self.position(left), self.position(right)) {
                (None, None) => {
                    if coin {
                        (left, left_push)
                    } else {
                        (right, right_push)
                    }
                }
                (None, Some(_)) => (left, left_push),
                (Some(_), None) => (right, right_push),
                (Some(_), Some(_)) => continue,
            };
            let incumbent = &mut self.entries[center];
            incumbent.slot = slot;
            incumbent.crit.push(push, now);
            self.entries.push(GroupEntry {
                slot: Slot::Center,
                crit,
                payload,
                visible: true,
            });
            return;
        }

        // Every slot taken: the incumbent center is replaced outright.
        self.entries[center].crit = crit;
        self.entries[center].payload = payload;
        self.entries[center].visible = true;
    }

    /// Advance every crit; `true` while at least one is still alive.
    ///
    /// `tick` receives each crit and its payload and answers the crit's
    /// state; entries that end are removed (dropping their payload), and
    /// each survivor remembers whether it drew, feeding [`Self::intersects`].
    pub fn tick_all(&mut self, mut tick: impl FnMut(&mut Crit, &P) -> CritTick) -> bool {
        self.entries.retain_mut(|entry| {
            let state = tick(&mut entry.crit, &entry.payload);
            entry.visible = matches!(state, CritTick::Draw { .. });
            !matches!(state, CritTick::End)
        });
        !self.entries.is_empty()
    }

    /// Whether any crit drawn this frame overlaps `rect`.
    pub fn intersects(&self, rect: &Rect) -> bool {
        self.entries
            .iter()
            .any(|e| e.visible && e.crit.rect().intersects(rect))
    }

    /// Drain every entry's payload without ticking, for adapter teardown.
    // Called only from the 32-bit adapter's device-swap path; the host test
    // build has no caller for it.
    #[cfg_attr(not(target_arch = "x86"), allow(dead_code))]
    pub fn drain_payloads(&mut self) -> Vec<P> {
        self.entries.drain(..).map(|e| e.payload).collect()
    }
}

/// The tallest overlap between `rect` and any rect the iterator yields.
///
/// Feeds the fast-forward pass: when a new line lands on existing ones, every
/// line in the overlapped list is advanced by this height so the stack keeps
/// spreading instead of piling up.
pub fn max_overlap_height(rect: &Rect, others: impl Iterator<Item = Rect>) -> i32 {
    others.fold(0, |best, other| best.max(rect.overlap_height(&other)))
}

/// `f64 -> i32` by truncation toward zero, saturating, without x87 help.
///
/// Pixel offsets are all small; the saturation only guards degenerate input.
fn truncate(v: f64) -> i32 {
    if v >= 2_147_483_647.0 {
        i32::MAX
    } else if v <= -2_147_483_648.0 {
        i32::MIN
    } else {
        // Range-checked right above, and an i32-range f64->i32 truncation
        // lowers to a single SSE2 instruction on every target here.
        // range-checked above
        v as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A spec with round numbers: 100 wide, 20 tall, 90px travel, 10 ticks/frame.
    fn spec(start: i64) -> FloatSpec {
        FloatSpec {
            width: 100,
            height: 20,
            floating_distance: 90,
            arc_radius: 50.0,
            arc_towards_right: true,
            start_ticks: start,
            ticks_per_frame: 10,
        }
    }

    /// Ticks for `frames` virtual frames under the round-number spec.
    const fn ticks(frames: i64) -> i64 {
        frames * 10
    }

    #[test]
    fn rise_moves_up_linearly_and_ends() {
        let mut float = Floating::new(&spec(0), Direction::Up);
        let Tick::Draw { rect: r0, alpha } = float.tick(ticks(0), Some((200, 300)), true) else {
            panic!("first tick draws");
        };
        assert!((alpha - 1.0).abs() < 1e-12);
        assert_eq!((r0.left, r0.right), (150, 250));
        assert_eq!((r0.top, r0.bottom), (280, 300));

        // Half the lifetime (171 of 342 frames) has travelled half of 90px.
        let Tick::Draw { rect: r_half, .. } = float.tick(ticks(171), Some((200, 300)), true) else {
            panic!("mid-life tick draws");
        };
        assert_eq!(r_half.bottom, 300 - 45);

        assert_eq!(float.tick(ticks(342), Some((200, 300)), true), Tick::End);
    }

    #[test]
    fn fade_is_late_and_linear() {
        let mut float = Floating::new(&spec(0), Direction::Up);
        // 1.3s * 180 = frame 234 is the last full-alpha frame.
        let Tick::Draw { alpha, .. } = float.tick(ticks(234), Some((0, 0)), true) else {
            panic!("draws before fade");
        };
        assert!((alpha - 1.0).abs() < 1e-12);
        // Halfway through the fade window (234..342), alpha is one half.
        let Tick::Draw { alpha, .. } = float.tick(ticks(288), Some((0, 0)), true) else {
            panic!("draws inside fade");
        };
        assert!((alpha - 0.5).abs() < 1e-9);
    }

    #[test]
    fn arc_sweeps_sideways_by_sign() {
        let mut right = Floating::new(&spec(0), Direction::Arc);
        let mut left_spec = spec(0);
        left_spec.arc_towards_right = false;
        let mut left = Floating::new(&left_spec, Direction::Arc);

        let Tick::Draw { rect: rr, .. } = right.tick(ticks(171), Some((0, 0)), true) else {
            panic!("right arc draws");
        };
        let Tick::Draw { rect: rl, .. } = left.tick(ticks(171), Some((0, 0)), true) else {
            panic!("left arc draws");
        };
        // cos leaves the sweep term negative, so the rightward sign moves
        // left of the anchor and the leftward sign mirrors it around it.
        assert!(rr.left < -50);
        assert_eq!(rl.left, -100 - rr.left);
        // Both rise the same amount.
        assert_eq!(rr.top, rl.top);
        assert!(rr.top < -20);
    }

    #[test]
    fn hidden_keeps_rect_and_missing_anchor_does_not_move_it() {
        let mut float = Floating::new(&spec(0), Direction::Up);
        let _ = float.tick(ticks(100), Some((200, 300)), true);
        let placed = float.rect();
        // Sight loss still updated the rect for overlap bookkeeping.
        assert_eq!(
            float.tick(ticks(120), Some((210, 300)), false),
            Tick::Hidden
        );
        assert_ne!(float.rect(), placed);
        let moved = float.rect();
        // A missing anchor leaves the rect exactly where it was.
        assert_eq!(float.tick(ticks(140), None, true), Tick::Hidden);
        assert_eq!(float.rect(), moved);
    }

    #[test]
    fn fast_forward_backdates_whole_frames() {
        let mut float = Floating::new(&spec(ticks(100)), Direction::Up);
        // 45px of 90px travel = half the lifetime = 171 frames.
        float.fast_forward(45);
        let Tick::Draw { rect, .. } = float.tick(ticks(100), Some((0, 100)), true) else {
            panic!("draws after fast-forward");
        };
        assert_eq!(rect.bottom, 100 - 45);
    }

    /// A crit spec with the same round numbers.
    fn crit_spec(start: i64) -> CritSpec {
        CritSpec {
            width: 120,
            height: 30,
            start_ticks: start,
            ticks_per_frame: 10,
        }
    }

    #[test]
    fn crit_pops_in_centered_then_settles_big() {
        let mut crit = Crit::new(&crit_spec(0));
        let CritTick::Draw {
            font,
            centered,
            alpha,
            ..
        } = crit.tick(ticks(18), Some((0, 0)), true)
        else {
            panic!("pop-in draws");
        };
        assert_eq!(font, CritFont::Normal);
        assert!(centered);
        assert!((alpha - 0.5).abs() < 1e-9);

        let CritTick::Draw {
            font,
            centered,
            alpha,
            ..
        } = crit.tick(ticks(100), Some((0, 0)), true)
        else {
            panic!("settled crit draws");
        };
        assert_eq!(font, CritFont::Big);
        assert!(!centered);
        assert!((alpha - 1.0).abs() < 1e-12);

        assert_eq!(crit.tick(ticks(396), Some((0, 0)), true), CritTick::End);
    }

    #[test]
    fn crit_shake_decays_to_zero() {
        let mut crit = Crit::new(&crit_spec(0));
        // First frame: the seed rect is one pixel tall, so no jolt.
        let CritTick::Draw { rect, .. } = crit.tick(ticks(0), Some((300, 400)), true) else {
            panic!("first tick draws");
        };
        assert_eq!((rect.left, rect.right), (240, 360));
        // After the impulse window (54 frames) the rect is exactly anchored.
        let CritTick::Draw { rect, .. } = crit.tick(ticks(60), Some((300, 400)), true) else {
            panic!("post-impulse draws");
        };
        assert_eq!(
            (rect.left, rect.top, rect.right, rect.bottom),
            (240, 370, 360, 400)
        );
    }

    #[test]
    fn crit_push_ramps_and_caps() {
        let mut crit = Crit::new(&crit_spec(0));
        let _ = crit.tick(ticks(60), Some((0, 0)), true);
        crit.push((-80, 0), ticks(60));
        let CritTick::Draw { rect, .. } = crit.tick(ticks(87), Some((0, 0)), true) else {
            panic!("mid-push draws");
        };
        // Half the impulse window: half the push distance.
        assert_eq!(rect.left, -60 - 40);
        let CritTick::Draw { rect, .. } = crit.tick(ticks(200), Some((0, 0)), true) else {
            panic!("post-push draws");
        };
        assert_eq!(rect.left, -60 - 80);
    }

    /// Ticks every crit with a fixed anchor and full visibility.
    fn tick_group(group: &mut CritsGroup<()>, now: i64) -> bool {
        group.tick_all(|crit, ()| crit.tick(now, Some((0, 0)), true))
    }

    #[test]
    fn carousel_ladder_fills_rings_then_replaces_center() {
        let mut group = CritsGroup::<()>::new(50, 40);
        for i in 0..7 {
            group.add(Crit::new(&crit_spec(0)), (), i % 2 == 0, ticks(0));
        }
        assert_eq!(group.len(), 7);
        // All seven slots occupied exactly once.
        for slot in [
            Slot::Center,
            Slot::Left,
            Slot::Right,
            Slot::LeftTop,
            Slot::RightTop,
            Slot::LeftBottom,
            Slot::RightBottom,
        ] {
            assert!(group.position(slot).is_some(), "{slot:?} filled");
        }
        // The eighth replaces the center without growing the group.
        group.add(Crit::new(&crit_spec(0)), (), true, ticks(0));
        assert_eq!(group.len(), 7);
        // The group survives a tick and every entry stays visible.
        assert!(tick_group(&mut group, ticks(10)));
        assert!(group.intersects(&Rect {
            left: -10,
            top: -10,
            right: 10,
            bottom: 10
        }));
    }

    #[test]
    fn carousel_coin_picks_the_free_side() {
        let mut group = CritsGroup::<()>::new(50, 40);
        group.add(Crit::new(&crit_spec(0)), (), false, ticks(0));
        // Coin false: the incumbent goes right.
        group.add(Crit::new(&crit_spec(0)), (), false, ticks(0));
        assert!(group.position(Slot::Right).is_some());
        assert!(group.position(Slot::Left).is_none());
        // Right taken: the next incumbent must go left regardless of coin.
        group.add(Crit::new(&crit_spec(0)), (), false, ticks(0));
        assert!(group.position(Slot::Left).is_some());
    }

    #[test]
    fn group_ends_when_all_crits_end() {
        let mut group = CritsGroup::<()>::new(50, 40);
        group.add(Crit::new(&crit_spec(0)), (), true, ticks(0));
        assert!(tick_group(&mut group, ticks(10)));
        assert!(!tick_group(&mut group, ticks(396)));
        assert!(group.is_empty());
    }

    #[test]
    fn overlap_height_takes_the_tallest() {
        let base = Rect {
            left: 0,
            top: 0,
            right: 100,
            bottom: 20,
        };
        let shallow = Rect {
            left: 50,
            top: 15,
            right: 150,
            bottom: 40,
        };
        let deep = Rect {
            left: 20,
            top: 8,
            right: 60,
            bottom: 40,
        };
        let clear = Rect {
            left: 0,
            top: 30,
            right: 100,
            bottom: 50,
        };
        let rects = [shallow, deep, clear];
        assert_eq!(max_overlap_height(&base, rects.iter().copied()), 12);
        assert_eq!(max_overlap_height(&clear, [base].into_iter()), 0);
    }
}
