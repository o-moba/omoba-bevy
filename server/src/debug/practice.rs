use crate::runtime::ServerRuntime;
use shared::practice::PracticeCommand;
use std::{net::SocketAddr, time::Instant};
impl ServerRuntime {
    pub(super) fn handle_practice_command(
        &mut self,
        addr: SocketAddr,
        command: PracticeCommand,
        now: Instant,
    ) {
        self.combat_host()
            .handle_practice_command(addr, command, now);
        let bots: Vec<_> = self
            .world
            .players
            .iter()
            .filter(|(_, p)| p.hero.identity.is_bot)
            .map(|(a, _)| *a)
            .collect();
        for bot in bots {
            self.register_career_participant(bot);
        }
    }
}
