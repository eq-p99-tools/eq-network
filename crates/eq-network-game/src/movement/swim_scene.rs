//! Independent local replay against one immutable, supported body/scene.
//! Scene observations do not replace server corrections or ordinary motion caps.
use super::water::{WetCoefficients, WetInput, WetKind, WetVelocity};
use anyhow::{ensure, Result};
use std::{fmt, sync::Arc};

/// Immutable local collision/body snapshot. All positions use EQ XYZ, Z up.
/// An unknown query must fail; it must never be treated as clear geometry.
/// Providers query geometry only and must not move the live actor.
pub trait SwimScene: Send + Sync {
    /// The body's established feet-to-origin offset, S.
    fn step_height(&self) -> f32;
    /// Solid floor below the actual origin. `Ok(None)` means known no floor.
    /// # Errors
    /// Declines unknown geometry, body state or coverage.
    fn floor(&self, origin: [f32; 3]) -> Result<Option<f32>>;
    /// Conservative body sweep, retaining ordinary wall collision and slide.
    /// # Errors
    /// Declines unsupported or unknown collision queries.
    fn sweep(&self, from: [f32; 3], target: [f32; 3]) -> Result<[f32; 3]>;
    /// Known fully submerged water set at the actual body probes.
    /// # Errors
    /// Declines unknown region, surface, dry or unsupported body state.
    fn kind(&self, origin: [f32; 3]) -> Result<WetKind>;
    /// Whether a Z-only floor lift is clear for the same body.
    /// # Errors
    /// Declines unknown geometry or a retired body/scene.
    fn clear_lift(&self, from: [f32; 3], target: [f32; 3]) -> Result<bool>;
}

/// Identity of one retained immutable scene/body, shared with its real producer.
/// Replacing a scene does not transfer a reset or create continuation.
#[derive(Clone)]
pub struct SwimSceneHandle(Arc<dyn SwimScene>);
impl SwimSceneHandle {
    /// Retains the actual collision owner's immutable snapshot. This constructor
    /// supplies geometry, not movement authority or an entry/reset assertion.
    #[must_use]
    pub fn new(scene: impl SwimScene + 'static) -> Self {
        Self(Arc::new(scene))
    }
}
impl PartialEq for SwimSceneHandle {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl fmt::Debug for SwimSceneHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SwimSceneHandle(immutable)")
    }
}

/// A disposable complete-frame state. Velocity is never reconstructed from travel.
/// This is the conservative floor-following subset, not the unread native engine
/// or a replacement for the separate native stationary/animation gates.
#[derive(Clone, Debug)]
pub struct SwimSceneMotion {
    scene: SwimSceneHandle,
    origin: [f32; 3],
    velocity: WetVelocity,
    floor: Option<f32>,
}

/// Geometry-resolved evidence for one completed physical frame.
#[derive(Clone, Copy, Debug)]
pub struct SwimSceneFrame {
    /// Intended integrated delta before floor handling.
    pub raw: [f32; 3],
    /// Actual final origin, after conservative collision and Z-only floor clamp.
    pub origin: [f32; 3],
    /// A floor snap changed Z. Horizontal input/carry was not reconstructed.
    pub floor_snap: bool,
}

impl SwimSceneMotion {
    /// Begins only at a separately matched actual reset. The network owner
    /// enforces that anchor and independently queries this snapshot's floor.
    /// # Errors
    /// Rejects invalid body/pose, unknown medium or unknown/nonfinite floor.
    pub fn at_reset(scene: SwimSceneHandle, origin: [f32; 3]) -> Result<Self> {
        ensure!(
            origin.into_iter().all(f32::is_finite),
            "invalid scene reset position"
        );
        let height = scene.0.step_height();
        ensure!(
            height.is_finite() && height > 0.0,
            "invalid swim body offset"
        );
        scene.0.kind(origin)?;
        let floor = scene.0.floor(origin)?;
        ensure!(floor.is_none_or(f32::is_finite), "invalid swim floor");
        Ok(Self {
            scene,
            origin,
            velocity: WetVelocity::default(),
            floor,
        })
    }
    /// Current actual origin.
    #[must_use]
    pub const fn origin(&self) -> [f32; 3] {
        self.origin
    }
    /// Known current medium in this immutable scene.
    /// # Errors
    /// Declines unknown or unsupported medium.
    pub fn kind(&self) -> Result<WetKind> {
        self.scene.0.kind(self.origin)
    }
    /// Current native velocity and phase, in EQ horizontal axes.
    #[must_use]
    pub const fn velocity(&self) -> WetVelocity {
        self.velocity
    }
    /// Immutable scene identity; a new handle cannot continue this state.
    #[must_use]
    pub const fn scene(&self) -> &SwimSceneHandle {
        &self.scene
    }

    /// Executes one observed completed frame on a disposable copy.
    /// Native rates, caps and input are independently supplied by each owner.
    /// This subset keeps old-floor snap, sweep, new-floor refresh and Z-only
    /// clamp; it does not guess the unread animation bypass or native cache.
    /// # Errors
    /// Declines unknown phase/medium/geometry, invalid time/input or blocked lift.
    #[allow(
        clippy::float_cmp,
        reason = "Exact zero horizontal target cannot produce horizontal motion"
    )]
    pub fn frame(
        &self,
        set: WetCoefficients,
        input: WetInput,
        seconds: f32,
    ) -> Result<(Self, SwimSceneFrame)> {
        ensure!(
            seconds.is_finite() && seconds > 1e-5 && seconds <= 0.15,
            "unobserved or excessive swim frame time"
        );
        ensure!(
            input
                .horizontal
                .into_iter()
                .chain([input.vertical])
                .all(f32::is_finite)
                && !input.jump,
            "unsupported swim frame input"
        );
        ensure!(
            self.velocity.has_ordinary_call_clock(),
            "unknown swim scene phase"
        );
        let mut next = self.clone();
        let (horizontal, vertical) = next.velocity.advance(set, input, seconds);
        ensure!(
            next.velocity.has_ordinary_call_clock(),
            "unknown resulting swim scene phase"
        );
        let raw = [horizontal[0], horizontal[1], vertical];
        let mut target = std::array::from_fn(|axis| self.origin[axis] + raw[axis]);
        let mut floor_snap = false;
        if let Some(floor) = self.floor {
            if target[2] - self.scene.0.step_height() < floor {
                target[2] = floor + self.scene.0.step_height();
                next.velocity.zero_vertical();
                floor_snap = true;
            }
        }
        // The snap is part of the sweep target, never a teleport around a wall.
        let mut origin = self.scene.0.sweep(self.origin, target)?;
        ensure!(
            origin.into_iter().all(f32::is_finite),
            "nonfinite swim sweep"
        );
        // No horizontal input/carry at representable resolution means no
        // horizontal displacement, including gravity-only slope projection.
        // Other collision results are independently recomputed from the retained
        // immutable scene and bounded by earned path in the enclosing trial.
        ensure!(
            !(target[0] == self.origin[0] && target[1] == self.origin[1])
                || (origin[0] == self.origin[0] && origin[1] == self.origin[1]),
            "swim sweep introduced unearned horizontal travel"
        );
        self.scene.0.kind(origin)?;
        next.floor = self.scene.0.floor(origin)?;
        ensure!(
            next.floor.is_none_or(f32::is_finite),
            "invalid refreshed swim floor"
        );
        if let Some(floor) = next.floor {
            if origin[2] < floor {
                let from = origin;
                origin[2] = floor;
                ensure!(
                    self.scene.0.clear_lift(from, origin)?,
                    "blocked swim floor lift"
                );
                next.velocity.zero_vertical();
                floor_snap = true;
            }
        }
        self.scene.0.kind(origin)?;
        next.origin = origin;
        Ok((
            next,
            SwimSceneFrame {
                raw,
                origin,
                floor_snap,
            },
        ))
    }
}

/// Only a matched ordinary acceptance commits this complete replay state.
#[derive(Clone, Debug)]
pub(super) struct Trial {
    pub velocity: WetVelocity,
    pub scene: Option<SwimSceneMotion>,
}
impl From<WetVelocity> for Trial {
    fn from(velocity: WetVelocity) -> Self {
        Self {
            velocity,
            scene: None,
        }
    }
}

/// Accumulates physical path instead of substituting its endpoint chord.
pub(super) struct PathBudget {
    horizontal: f64,
    earned_horizontal: f64,
    up: f64,
    down: f64,
    spatial: f64,
}
impl PathBudget {
    pub const fn new() -> Self {
        Self {
            horizontal: 0.0,
            earned_horizontal: 0.0,
            up: 0.0,
            down: 0.0,
            spatial: 0.0,
        }
    }
    pub fn add(&mut self, from: [f32; 3], to: [f32; 3], raw: [f32; 3]) {
        let d: [f64; 3] = std::array::from_fn(|axis| f64::from(to[axis]) - f64::from(from[axis]));
        let flat = d[0].hypot(d[1]);
        self.horizontal += flat;
        self.earned_horizontal += f64::from(raw[0]).hypot(f64::from(raw[1]));
        self.up += d[2].max(0.0);
        self.down += (-d[2]).max(0.0);
        self.spatial += flat.hypot(d[2]);
    }
    pub fn check(&self, caps: super::water::WetCaps, seconds: f32) -> Result<()> {
        let time = f64::from(seconds);
        // Exactly one existing sample padding, never one padding per frame.
        ensure!(
            self.horizontal
                <= (f64::from(caps.horizontal) * time).min(self.earned_horizontal) + 0.01,
            "swim physical path exceeds horizontal cap"
        );
        ensure!(
            self.up <= f64::from(caps.up) * time + 0.01
                && self.down <= f64::from(caps.down) * time + 0.01,
            "swim physical path exceeds vertical cap"
        );
        ensure!(
            self.spatial
                <= f64::from(caps.horizontal).hypot(f64::from(caps.up.max(caps.down))) * time
                    + 0.01,
            "swim physical path exceeds total cap"
        );
        Ok(())
    }
}
