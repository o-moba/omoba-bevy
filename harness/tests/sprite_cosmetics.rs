//! Focused multi-client wire verification for sprite cosmetic identity.

use std::time::Duration;

use harness::{Bot, Character, HeroClass, ServerProcess, Team};

const TIMEOUT: Duration = Duration::from_secs(5);

#[test]
fn distinct_sprite_ids_round_trip_and_invalid_or_omitted_values_default() {
    let server = ServerProcess::spawn_with_env(&[("OMOBA_MATCH_MODE", "dev")]);
    let mut teapot = Bot::connect(server.addr());
    let mut jelly = Bot::connect(server.addr());
    let mut orchard = Bot::connect(server.addr());
    let mut invalid = Bot::connect(server.addr());
    let mut omitted = Bot::connect(server.addr());

    teapot.join_with_cosmetics(
        Team::Green,
        Character::Ipfs,
        HeroClass::Warrior,
        None,
        Some("mossback-teapot"),
    );
    jelly.join_with_cosmetics(
        Team::Blue,
        Character::Toka,
        HeroClass::Mage,
        None,
        Some("void-jelly-astronaut"),
    );
    orchard.join_with_cosmetics(
        Team::Green,
        Character::Cube,
        HeroClass::Cleric,
        None,
        Some("orchard-comet-centaur"),
    );
    invalid.join_with_cosmetics(
        Team::Green,
        Character::Wang,
        HeroClass::Ranger,
        None,
        Some("../not-an-asset"),
    );
    omitted.join(Team::Blue, Character::Cube);

    let teapot_id = teapot.my_id(TIMEOUT);
    let jelly_id = jelly.my_id(TIMEOUT);
    let orchard_id = orchard.my_id(TIMEOUT);
    let invalid_id = invalid.my_id(TIMEOUT);
    let omitted_id = omitted.my_id(TIMEOUT);

    let teapot_state = teapot
        .wait_for_player(
            teapot_id,
            |player| player.sprite_character.is_some(),
            TIMEOUT,
        )
        .expect("teapot identity should replicate");
    let jelly_state = teapot
        .wait_for_player(
            jelly_id,
            |player| player.sprite_character.is_some(),
            TIMEOUT,
        )
        .expect("remote jelly identity should replicate");
    let invalid_state = teapot
        .wait_for_player(
            invalid_id,
            |player| player.sprite_character.is_some(),
            TIMEOUT,
        )
        .expect("invalid identity should normalize and replicate");
    let orchard_state = teapot
        .wait_for_player(
            orchard_id,
            |player| player.sprite_character.is_some(),
            TIMEOUT,
        )
        .expect("new sprite identity should replicate");
    let omitted_state = teapot
        .wait_for_player(
            omitted_id,
            |player| player.sprite_character.is_some(),
            TIMEOUT,
        )
        .expect("omitted identity should default and replicate");

    assert_eq!(
        teapot_state.sprite_character.as_deref(),
        Some("mossback-teapot")
    );
    assert_eq!(
        jelly_state.sprite_character.as_deref(),
        Some("void-jelly-astronaut")
    );
    assert_eq!(
        orchard_state.sprite_character.as_deref(),
        Some("orchard-comet-centaur")
    );
    assert_eq!(
        invalid_state.sprite_character.as_deref(),
        Some("mossback-teapot")
    );
    assert_eq!(
        omitted_state.sprite_character.as_deref(),
        Some("mossback-teapot")
    );
}
