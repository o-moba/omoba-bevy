//! Smooths replicated transforms between snapshots and grounds them on local terrain.

use bevy::ecs::query::Or;
use bevy::prelude::*;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::maps::MapLayout;
use crate::model_scale::NormalizeModelScale;
use crate::sprite::PlayerVisualMode;

use super::components::{
    GameStateSnapshot, NetworkMinion, NetworkNeutral, PlayerUtility, RemotePlayer,
};

const MINION_RADIUS: f32 = 0.55;

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct NetworkGroundingSet;

/// Client-side terrain lift for server-driven entities. The server simulates
/// a flat ground plane; after interpolation writes the (flat) server Y, this
/// re-bases remote players and minions onto the local terrain (base pads and
/// their ramps), matching how the local player is grounded.
pub(in crate::net) fn ground_networked_entities(
    map_layout: Res<MapLayout>,
    visual_mode: Res<PlayerVisualMode>,
    mut remote_players: Query<
        (&mut Transform, Option<&NormalizeModelScale>),
        (With<RemotePlayer>, Without<NetworkMinion>),
    >,
    mut minions: Query<
        (&mut Transform, Option<&NormalizeModelScale>),
        (With<NetworkMinion>, Without<RemotePlayer>),
    >,
) {
    for (mut transform, normalization) in &mut remote_players {
        transform.translation.y = crate::player::ground_origin_y(
            &map_layout,
            *visual_mode,
            normalization,
            transform.translation.x,
            transform.translation.z,
        );
    }
    for (mut transform, normalization) in &mut minions {
        let terrain = if *visual_mode == PlayerVisualMode::Models3d {
            map_layout.terrain_height_3d(transform.translation.x, transform.translation.z)
        } else {
            map_layout.terrain_height(transform.translation.x, transform.translation.z)
        };
        // Slime models rest on their measured foot offset; the sphere radius
        // remains the fallback until the model is measured.
        let offset = match normalization.and_then(NormalizeModelScale::foot_local_y) {
            Some(foot_local_y) => -foot_local_y,
            None => MINION_RADIUS,
        };
        transform.translation.y = terrain + offset;
    }
}

/// Keep authoritative remaining durations moving between snapshots, including
/// packet loss or a paused virtual clock. Each new snapshot replaces this estimate.
pub(in crate::net) fn age_utility_timers(
    time: Res<Time<Real>>,
    game: Option<Res<GameStateSnapshot>>,
    mut players: Query<&mut PlayerUtility>,
) {
    let elapsed = time.delta_secs() * crate::sandbox::time_scale(game.as_deref());
    for mut utility in &mut players {
        let state = &mut utility.state;
        state.dash_remaining_secs = (state.dash_remaining_secs - elapsed).max(0.0);
        state.haste_remaining_secs = (state.haste_remaining_secs - elapsed).max(0.0);
        state.haste_active_secs = (state.haste_active_secs - elapsed).max(0.0);
    }
}

#[derive(Component, Clone, Copy, Debug)]
pub(in crate::net) struct NetEntityInterpolation {
    pub(in crate::net) from_translation: Vec3,
    pub(in crate::net) to_translation: Vec3,
    pub(in crate::net) from_rotation: Quat,
    pub(in crate::net) to_rotation: Quat,
    pub(in crate::net) elapsed: f32,
    pub(in crate::net) duration: f32,
}

#[derive(Clone, Copy, Debug)]
struct RemotePose {
    received: Instant,
    translation: Vec3,
    rotation: Quat,
}

/// Render two network ticks behind receipt time. Interpolating authoritative
/// samples avoids the old 50ms ease/stop/restart cycle whenever Wi-Fi delivers
/// the next snapshot a little late. Never extrapolate through another actor.
#[derive(Component, Clone, Debug)]
pub(in crate::net) struct RemotePlayerInterpolation {
    poses: VecDeque<RemotePose>,
}

impl RemotePlayerInterpolation {
    const DELAY: Duration = Duration::from_millis(100);

    pub(in crate::net) fn new(translation: Vec3, rotation: Quat, received: Instant) -> Self {
        Self {
            poses: VecDeque::from([RemotePose {
                received,
                translation,
                rotation,
            }]),
        }
    }

    pub(in crate::net) fn push(&mut self, translation: Vec3, rotation: Quat, received: Instant) {
        if let Some(last) = self.poses.back() {
            if received <= last.received {
                return;
            }
            // Respawns/reconnects are explicit teleports, not a journey across
            // the map. Drop old history after a long network interruption too.
            if translation.distance_squared(last.translation) > 12.0_f32.powi(2)
                || received.duration_since(last.received) > Duration::from_millis(500)
            {
                self.poses.clear();
            }
        }
        self.poses.push_back(RemotePose {
            received,
            translation,
            rotation,
        });
        while self.poses.len() > 8 {
            self.poses.pop_front();
        }
    }

    /// Latest authoritative sample, independent of the render delay.
    pub(in crate::net) fn latest_translation(&self) -> Option<Vec3> {
        self.poses.back().map(|pose| pose.translation)
    }

    /// An accepted dash is an instant relocation: drop the history so the hero
    /// reappears at the destination instead of sliding there over two ticks.
    pub(in crate::net) fn teleport(
        &mut self,
        translation: Vec3,
        rotation: Quat,
        received: Instant,
    ) {
        self.poses.clear();
        self.push(translation, rotation, received);
    }

    fn sample(&mut self, now: Instant) -> RemotePose {
        let at = now.checked_sub(Self::DELAY).unwrap_or(now);
        while self.poses.len() > 2 && self.poses[1].received <= at {
            self.poses.pop_front();
        }
        let first = self.poses[0];
        let Some(second) = self.poses.get(1).copied() else {
            return first;
        };
        let span = second.received.duration_since(first.received).as_secs_f32();
        let fraction = (at.saturating_duration_since(first.received).as_secs_f32()
            / span.max(0.001))
        .clamp(0.0, 1.0);
        RemotePose {
            received: at,
            translation: first.translation.lerp(second.translation, fraction),
            rotation: first.rotation.slerp(second.rotation, fraction),
        }
    }
}

pub(in crate::net) fn interpolate_snapshot_entities(
    time: Res<Time>,
    mut entity_query: Query<
        (&mut Transform, &mut NetEntityInterpolation),
        Or<(With<NetworkMinion>, With<NetworkNeutral>)>,
    >,
) {
    for (mut transform, mut interpolation) in &mut entity_query {
        let duration = interpolation.duration.max(0.001);
        interpolation.elapsed = (interpolation.elapsed + time.delta_secs()).min(duration);
        let t = (interpolation.elapsed / duration).clamp(0.0, 1.0);
        transform.translation = interpolation
            .from_translation
            .lerp(interpolation.to_translation, t);
        transform.rotation = interpolation
            .from_rotation
            .slerp(interpolation.to_rotation, t);
    }
}

pub(in crate::net) fn interpolate_remote_players(
    mut player_query: Query<(&mut Transform, &mut RemotePlayerInterpolation), With<RemotePlayer>>,
) {
    let now = Instant::now();
    for (mut transform, mut interpolation) in &mut player_query {
        let pose = interpolation.sample(now);
        transform.translation = pose.translation;
        transform.rotation = pose.rotation;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delayed_wifi_snapshots_stay_in_motion_without_overshooting() {
        let start = Instant::now();
        let mut track = RemotePlayerInterpolation::new(Vec3::ZERO, Quat::IDENTITY, start);
        // Ordinary jitter around the 50ms server cadence. Position is the
        // authoritative constant-speed track; this also tests queue pruning.
        let arrivals = [50_u64, 118, 158, 213, 267, 309];
        let mut next = 0;
        let mut last = 0.0;
        for millis in (10_u64..=400).step_by(10) {
            while next < arrivals.len() && arrivals[next] <= millis {
                let at = arrivals[next];
                track.push(
                    Vec3::X * (at as f32 / 1000.0),
                    Quat::IDENTITY,
                    start + Duration::from_millis(at),
                );
                next += 1;
            }
            let pose = track.sample(start + Duration::from_millis(millis));
            if (110..=390).contains(&millis) {
                assert!(
                    (pose.translation.x - last - 0.01).abs() < 0.000_01,
                    "movement stalled at {millis}ms: {last} -> {}",
                    pose.translation.x
                );
            }
            assert!(pose.translation.x <= arrivals[next.saturating_sub(1)] as f32 / 1000.0);
            last = pose.translation.x;
        }
        let held = track.sample(start + Duration::from_secs(2));
        assert_eq!(
            held.translation,
            Vec3::X * 0.309,
            "never extrapolate on packet loss"
        );
    }

    #[test]
    fn remote_respawn_drops_old_path_and_out_of_order_samples() {
        let start = Instant::now();
        let mut track = RemotePlayerInterpolation::new(Vec3::ZERO, Quat::IDENTITY, start);
        track.push(Vec3::X, Quat::IDENTITY, start + Duration::from_millis(50));
        track.push(
            Vec3::X * 99.0,
            Quat::IDENTITY,
            start + Duration::from_millis(40),
        );
        assert_eq!(track.poses.len(), 2);
        track.push(
            Vec3::X * 40.0,
            Quat::from_rotation_y(2.0),
            start + Duration::from_millis(100),
        );
        let pose = track.sample(start + Duration::from_millis(100));
        assert_eq!(pose.translation, Vec3::X * 40.0);
        assert_eq!(pose.rotation, Quat::from_rotation_y(2.0));
        assert_eq!(track.poses.len(), 1);
    }

    #[test]
    fn utility_timers_expire_without_followup_snapshots_and_keep_acknowledgments() {
        let mut app = App::new();
        app.init_resource::<Time<Real>>()
            .add_systems(Update, age_utility_timers);
        let player = app
            .world_mut()
            .spawn(PlayerUtility {
                state: shared::utility::UtilityState {
                    dash_remaining_secs: shared::utility::DASH_COOLDOWN_SECS,
                    haste_remaining_secs: shared::utility::HASTE_COOLDOWN_SECS,
                    haste_active_secs: shared::utility::HASTE_DURATION_SECS,
                    last_request_id: 8,
                    dash_sequence: 3,
                },
            })
            .id();
        for frame in 1..=52 {
            app.world_mut()
                .resource_mut::<Time<Real>>()
                .advance_by(Duration::from_millis(500));
            app.update();
            let state = app.world().get::<PlayerUtility>(player).unwrap().state;
            if frame == 5 {
                assert_eq!(state.movement_multiplier(), 1.4);
            }
            if frame >= 6 {
                assert_eq!(state.movement_multiplier(), 1.0);
            }
            assert_eq!((state.last_request_id, state.dash_sequence), (8, 3));
        }
        let state = app.world().get::<PlayerUtility>(player).unwrap().state;
        assert_eq!(
            (
                state.dash_remaining_secs,
                state.haste_remaining_secs,
                state.haste_active_secs
            ),
            (0.0, 0.0, 0.0)
        );
    }
}
