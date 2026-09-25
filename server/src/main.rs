mod balance;
#[cfg(test)]
mod balance_probe;
mod basic_attack;
mod bots;
mod career_backend;
mod career_port;
mod career_runtime;
#[cfg(test)]
mod career_runtime_tests;
mod combat_feedback;
mod debug;
mod entities;
mod forest_pickups;
mod formation;
mod game_world;
mod hero;
mod hero_stats;
mod hero_timers;
#[cfg(test)]
mod map_config_tests;
mod match_allocation;
mod match_pool;
mod match_rules;
mod match_service;
mod match_stats;
#[cfg(test)]
mod minion_path_tests;
#[cfg(test)]
mod navigation_tests;
mod neutrals;
#[cfg(test)]
mod objective_balance_tests;
mod party;
mod passport_admission;
#[cfg(test)]
mod practice_tests;
mod prematch;
mod progression;
mod public_transport;
#[cfg(test)]
mod release_tests;
mod runtime;
mod sandbox;
mod session;
mod shop;
mod sim;
mod snapshot;
mod social;
mod targeting_qa;
#[cfg(test)]
mod terminal_result_tests;
#[cfg(test)]
mod tests;
mod utility;
mod vision;
mod world;

use std::io;

fn main() -> io::Result<()> {
    runtime::run()
}
