//! Durable lobby intent; only the worker acknowledges releasing an unused seat.
use super::{AllocatedHuman, Manifest, atomic_json};
use crate::{match_service::MatchService, runtime::ServerRuntime};
use std::{
    fs,
    io::{self, Read},
    path::Path,
    time::{Duration, Instant},
};

pub(crate) const FILE: &str = "release-requests.json";
const MAX_BYTES: u64 = 16_384;

pub(crate) fn read(directory: &Path, manifest: &Manifest) -> io::Result<Vec<AllocatedHuman>> {
    let path = directory.join(FILE);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    if !metadata.is_file() || metadata.len() > MAX_BYTES {
        return Err(io::Error::other("invalid allocation release file"));
    }
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)?;
    let requests: Vec<AllocatedHuman> = serde_json::from_slice(&bytes)?;
    if bytes.len() > MAX_BYTES as usize
        || manifest.preference.rated()
        || requests.len() > manifest.humans.len()
        || requests.iter().any(|r| !manifest.humans.contains(r))
        || requests
            .iter()
            .enumerate()
            .any(|(i, r)| requests[..i].contains(r))
    {
        return Err(io::Error::other("release does not match allocation"));
    }
    Ok(requests)
}

pub(crate) fn request(directory: &Path, manifest: &Manifest, profile: &str) -> io::Result<()> {
    if manifest.preference.rated() {
        return Err(io::Error::other("Ranked cannot replace human seats"));
    }
    let human = manifest
        .humans
        .iter()
        .find(|h| h.profile_id == profile)
        .ok_or_else(|| io::Error::other("profile is not allocated"))?;
    let mut requests = read(directory, manifest)?;
    if !requests.contains(human) {
        requests.push(human.clone());
        atomic_json(&directory.join(FILE), &requests)?;
    }
    Ok(())
}

impl ServerRuntime {
    /// An intent arriving after the durable roster freezes loses the race to
    /// start. It cannot free admission, remove a participant or invent an ACK.
    pub(crate) fn apply_allocated_releases(&mut self, now: Instant, force: bool) {
        if self.match_started_at.is_some() || self.career_roster_is_frozen() {
            return;
        }
        let MatchService::Worker(worker) = &mut self.match_service else {
            return;
        };
        if worker.manifest.preference.rated()
            || worker.recovery
            || worker.aborted
            || (!force
                && worker.last_release_scan.is_some_and(|at| {
                    now.saturating_duration_since(at) < Duration::from_millis(250)
                }))
        {
            return;
        }
        worker.last_release_scan = Some(now);
        let Ok(requests) = read(&worker.directory, &worker.manifest) else {
            return;
        };
        let pending: Vec<_> = requests
            .into_iter()
            .filter(|h| !worker.released.contains(&h.session_id))
            .collect();
        for human in pending {
            let address = self
                .world
                .players
                .iter()
                .find(|(_, p)| {
                    p.joined
                        && p.session_id.as_deref() == Some(human.session_id.as_str())
                        && p.career_profile
                            .as_ref()
                            .is_some_and(|p| p.profile_id == human.profile_id)
                })
                .map(|(addr, _)| *addr);
            if let Some(address) = address {
                self.leave_match(address, now);
            }
            let MatchService::Worker(worker) = &mut self.match_service else {
                return;
            };
            if !worker.released.contains(&human.session_id) {
                worker.released.push(human.session_id);
            }
            worker.aborted = worker
                .manifest
                .humans
                .iter()
                .all(|h| worker.released.contains(&h.session_id));
            // Publish the ACK on the next worker pass, without a heartbeat delay.
            worker.last_status = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::match_allocation::{Phase, Status, runtime_tests::fixture};
    use shared::career::ProfileSummary;

    #[test]
    fn handoff_release_binds_exact_manifest_identity_and_never_releases_ranked() {
        let (mut rt, _, _) = fixture();
        let worker = rt.match_service.worker().unwrap();
        let dir = worker.directory.clone();
        fs::create_dir_all(&dir).unwrap();
        let mut impostor = worker.manifest.humans[0].clone();
        impostor.profile_id = "c".repeat(64);
        atomic_json(&dir.join(FILE), &vec![impostor]).unwrap();
        assert!(read(&dir, &worker.manifest).is_err());
        rt.apply_allocated_releases(Instant::now(), true);
        assert!(rt.match_service.worker().unwrap().released.is_empty());
        fs::remove_file(dir.join(FILE)).unwrap();
        if let MatchService::Worker(worker) = &mut rt.match_service {
            worker.manifest.preference = shared::match_service::MatchPreference::Ranked;
            assert!(request(&dir, &worker.manifest, &"a".repeat(64)).is_err());
        }
        assert!(!dir.join(FILE).exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn handoff_release_replaces_one_joined_human_and_acknowledges_without_aborting_others() {
        let (mut rt, one, join) = fixture();
        let now = Instant::now();
        let two = "127.0.0.1:60201".parse().unwrap();
        let profile = ProfileSummary::new("c".repeat(64), "Second".into());
        rt.career
            .backend
            .test_authenticated(two, profile.clone(), "seat-b");
        if let MatchService::Worker(worker) = &mut rt.match_service {
            let mut human = worker.manifest.humans[0].clone();
            human.profile_id = profile.profile_id.clone();
            human.session_id = "seat-b".into();
            worker.manifest.humans.push(human);
        }
        rt.handle_packet(one, join.clone(), now);
        let mut other_join = join;
        if let shared::wire::ClientPacket::Join { session_id, .. } = &mut other_join {
            *session_id = Some("seat-b".into());
        }
        rt.handle_packet(two, other_join, now);
        assert!(rt.world.players[&one].joined && rt.world.players[&two].joined);
        let worker = rt.match_service.worker().unwrap();
        let dir = worker.directory.clone();
        fs::create_dir_all(&dir).unwrap();
        request(&dir, &worker.manifest, &"a".repeat(64)).unwrap();
        rt.tick_match_service(now);
        assert!(!rt.world.players[&one].joined);
        assert!(rt.world.players[&two].joined);
        let worker = rt.match_service.worker().unwrap();
        assert_eq!(worker.released, ["seat-a"]);
        assert!(!worker.aborted);
        let status: Status =
            serde_json::from_slice(&fs::read(dir.join("status.json")).unwrap()).unwrap();
        assert_eq!(status.released_sessions, ["seat-a"]);
        assert!(!matches!(status.phase, Phase::Failed));
        assert_eq!(rt.world.players.values().filter(|p| p.joined).count(), 10);
        assert!(!dir.join("cancel.json").exists());

        assert!(!rt.begin_career_round(now));
        assert!(
            rt.career_roster_is_frozen(),
            "real durable allocation is pending ACK"
        );
        let before = rt.career_allocation_for_test().unwrap();
        request(
            &dir,
            &rt.match_service.worker().unwrap().manifest,
            &profile.profile_id,
        )
        .unwrap();
        rt.apply_allocated_releases(now + Duration::from_secs(1), true);
        assert!(
            rt.world.players[&two].joined,
            "a late intent cannot mutate frozen roster"
        );
        assert_eq!(rt.match_service.worker().unwrap().released, ["seat-a"]);
        let after = rt.career_allocation_for_test().unwrap();
        assert_eq!(before.result_id, after.result_id);
        assert_eq!(before.participants, after.participants);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn handoff_release_of_last_reserved_human_never_starts_a_bots_only_round() {
        let (mut rt, _, _) = fixture();
        let worker = rt.match_service.worker().unwrap();
        let dir = worker.directory.clone();
        fs::create_dir_all(&dir).unwrap();
        request(&dir, &worker.manifest, &"a".repeat(64)).unwrap();
        rt.apply_allocated_releases(Instant::now(), true);
        assert!(rt.match_service.worker().unwrap().aborted);
        assert!(!rt.begin_career_round(Instant::now()));
        assert!(rt.career_allocation_for_test().is_none());
        fs::remove_dir_all(dir).unwrap();
    }
}
