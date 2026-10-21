//! Audit of the Dedenne ex + Indeedee ex meta deck: Dede-Circuit, Watch Over, Indeedee ex's
//! Psychic, and the Tools / Stadium / Oricorio the deck runs (Giant Cape, Clear Veil, Protective
//! Poncho, Starting Plains, Safeguard).
use deckgym::{
    actions::{Action, SimpleAction},
    card_ids::CardId,
    database::get_card_by_enum,
    models::{EnergyType, PlayedCard},
    state::State,
    test_support::{
        attack_action, get_initialized_game_with_board, get_test_game_with_board, trainer_from_id,
    },
    Game,
};

fn apply(game: &mut Game<'static>, actor: usize, action: SimpleAction) {
    game.apply_action(&Action {
        actor,
        action,
        is_stack: false,
    });
}

fn ability_offered(state: &State, in_play_idx: usize) -> bool {
    let (_, actions) = state.generate_possible_actions();
    actions
        .iter()
        .any(|a| a.action == SimpleAction::UseAbility { in_play_idx })
}

fn tool(card_id: CardId) -> deckgym::models::Card {
    get_card_by_enum(card_id)
}

fn dedenne() -> PlayedCard {
    PlayedCard::from_id(CardId::B3b024DedenneEx)
        .with_energy(vec![EnergyType::Lightning, EnergyType::Lightning])
}

// --- Dedenne ex: "This attack does 40 damage for each Pokémon Tool attached to all of your
// Pokémon." ---

#[test]
fn test_dede_circuit_counts_tools_on_all_your_pokemon() {
    let mut game = get_test_game_with_board(
        vec![
            dedenne().with_tool(tool(CardId::A2147GiantCape)),
            PlayedCard::from_id(CardId::B1121IndeedeeEx)
                .with_tool(tool(CardId::B2147ProtectivePoncho)),
            PlayedCard::from_id(CardId::A3066Oricorio).with_tool(tool(CardId::B3b064SmallBalloon)),
        ],
        // The opponent's own Tool does not count
        vec![PlayedCard::from_id(CardId::A1211Snorlax).with_tool(tool(CardId::A2147GiantCape))],
    );
    apply(&mut game, 0, attack_action(CardId::B3b024DedenneEx, 0));
    // Snorlax 150 + 20 (Giant Cape) - 3 * 40
    assert_eq!(game.get_state_clone().get_remaining_hp(1, 0), 50);
}

#[test]
fn test_dede_circuit_without_tools_does_nothing() {
    let mut game = get_test_game_with_board(
        vec![dedenne()],
        vec![PlayedCard::from_id(CardId::A1211Snorlax)],
    );
    apply(&mut game, 0, attack_action(CardId::B3b024DedenneEx, 0));
    assert_eq!(game.get_state_clone().get_remaining_hp(1, 0), 150);
}

// --- Indeedee ex: Watch Over "Once during your turn, you may heal 20 damage from your Active
// Pokémon." and Psychic "30 more damage for each Energy attached to your opponent's Active". ---

#[test]
fn test_watch_over_from_the_bench_heals_the_active_20() {
    let mut game = get_test_game_with_board(
        vec![
            dedenne().with_damage(50),
            PlayedCard::from_id(CardId::B1121IndeedeeEx).with_damage(30),
        ],
        vec![PlayedCard::from_id(CardId::A1211Snorlax)],
    );
    assert!(ability_offered(&game.get_state_clone(), 1));
    apply(&mut game, 0, SimpleAction::UseAbility { in_play_idx: 1 });
    let state = game.get_state_clone();
    assert_eq!(state.get_remaining_hp(0, 0), 90, "Active healed 20");
    assert_eq!(
        state.get_remaining_hp(0, 1),
        100,
        "Indeedee itself is not healed"
    );
}

#[test]
fn test_watch_over_once_per_turn_per_indeedee() {
    let mut game = get_test_game_with_board(
        vec![
            dedenne().with_damage(80),
            PlayedCard::from_id(CardId::B1121IndeedeeEx),
            PlayedCard::from_id(CardId::B1121IndeedeeEx),
        ],
        vec![PlayedCard::from_id(CardId::A1211Snorlax)],
    );
    apply(&mut game, 0, SimpleAction::UseAbility { in_play_idx: 1 });
    assert!(!ability_offered(&game.get_state_clone(), 1));
    assert!(ability_offered(&game.get_state_clone(), 2));
    apply(&mut game, 0, SimpleAction::UseAbility { in_play_idx: 2 });
    assert_eq!(game.get_state_clone().get_remaining_hp(0, 0), 80);
}

#[test]
fn test_watch_over_does_not_heal_above_max_hp_with_giant_cape() {
    let mut game = get_test_game_with_board(
        vec![
            dedenne()
                .with_tool(tool(CardId::A2147GiantCape))
                .with_damage(10),
            PlayedCard::from_id(CardId::B1121IndeedeeEx),
        ],
        vec![PlayedCard::from_id(CardId::A1211Snorlax)],
    );
    assert_eq!(game.get_state_clone().get_remaining_hp(0, 0), 130);
    apply(&mut game, 0, SimpleAction::UseAbility { in_play_idx: 1 });
    assert_eq!(
        game.get_state_clone().get_remaining_hp(0, 0),
        140,
        "120 + 20 Giant Cape"
    );
}

#[test]
fn test_indeedee_psychic_adds_30_per_energy_on_the_opponents_active() {
    let attack_with = |energies: Vec<EnergyType>| {
        let mut game = get_test_game_with_board(
            vec![PlayedCard::from_id(CardId::B1121IndeedeeEx)
                .with_energy(vec![EnergyType::Psychic, EnergyType::Psychic])],
            vec![
                PlayedCard::from_id(CardId::A1211Snorlax).with_energy(energies),
                // Energy on the Bench does not count
                PlayedCard::from_id(CardId::A1001Bulbasaur).with_energy(vec![EnergyType::Grass]),
            ],
        );
        apply(&mut game, 0, attack_action(CardId::B1121IndeedeeEx, 0));
        150 - game.get_state_clone().get_remaining_hp(1, 0)
    };
    assert_eq!(attack_with(vec![]), 30);
    assert_eq!(
        attack_with(vec![
            EnergyType::Colorless,
            EnergyType::Water,
            EnergyType::Water
        ]),
        120
    );
}

// --- Tools, Stadium and Oricorio ---

#[test]
fn test_giant_cape_adds_20_hp() {
    let game = get_test_game_with_board(
        vec![PlayedCard::from_id(CardId::B1121IndeedeeEx).with_tool(tool(CardId::A2147GiantCape))],
        vec![PlayedCard::from_id(CardId::A1211Snorlax)],
    );
    assert_eq!(game.get_state_clone().get_remaining_hp(0, 0), 150);
}

#[test]
fn test_starting_plains_gives_basics_of_both_players_20_hp_and_stacks_with_giant_cape() {
    let mut game = get_test_game_with_board(
        vec![
            PlayedCard::from_id(CardId::B3b024DedenneEx).with_tool(tool(CardId::A2147GiantCape)),
            PlayedCard::from_id(CardId::A1002Ivysaur),
        ],
        vec![PlayedCard::from_id(CardId::A1211Snorlax)],
    );
    let mut state = game.get_state_clone();
    state.hands[0] = vec![get_card_by_enum(CardId::B2154StartingPlains)];
    game.set_state(state);
    apply(
        &mut game,
        0,
        SimpleAction::Play {
            trainer_card: trainer_from_id(CardId::B2154StartingPlains),
        },
    );
    let state = game.get_state_clone();
    assert_eq!(
        state.get_remaining_hp(0, 0),
        160,
        "120 + 20 Cape + 20 Plains"
    );
    assert_eq!(
        state.get_remaining_hp(0, 1),
        90,
        "Stage 1 Ivysaur does not get it"
    );
    assert_eq!(
        state.get_remaining_hp(1, 0),
        170,
        "the opponent's Basic gets it too"
    );
}

#[test]
fn test_safeguard_blocks_attacks_from_ex_active_and_bench_but_not_from_non_ex() {
    // Articuno ex's Blizzard: 80 to the Active, 10 to each Benched Pokémon
    let mut game = get_initialized_game_with_board(
        0,
        1,
        3,
        vec![
            PlayedCard::from_id(CardId::A3066Oricorio),
            PlayedCard::from_id(CardId::A3066Oricorio),
            PlayedCard::from_id(CardId::B3b024DedenneEx),
        ],
        vec![
            PlayedCard::from_id(CardId::A3b104ArticunoEx).with_energy(vec![
                EnergyType::Water,
                EnergyType::Water,
                EnergyType::Water,
            ]),
        ],
    );
    apply(&mut game, 1, attack_action(CardId::A3b104ArticunoEx, 1));
    let state = game.get_state_clone();
    assert_eq!(
        state.get_remaining_hp(0, 0),
        70,
        "Active Oricorio takes nothing from an ex"
    );
    assert_eq!(
        state.get_remaining_hp(0, 1),
        70,
        "Benched Oricorio takes nothing from an ex"
    );
    assert_eq!(state.get_remaining_hp(0, 2), 110, "Dedenne ex takes the 10");

    // A non-ex attacker does damage to Oricorio
    let mut game = get_initialized_game_with_board(
        0,
        1,
        3,
        vec![PlayedCard::from_id(CardId::A3066Oricorio)],
        vec![PlayedCard::from_id(CardId::A1089Greninja)
            .with_energy(vec![EnergyType::Water, EnergyType::Water])],
    );
    apply(&mut game, 1, attack_action(CardId::A1089Greninja, 0));
    assert_eq!(game.get_state_clone().get_remaining_hp(0, 0), 10);
}

#[test]
fn test_oricorio_zzzap_does_50() {
    let mut game = get_test_game_with_board(
        vec![PlayedCard::from_id(CardId::A3066Oricorio)
            .with_energy(vec![EnergyType::Lightning, EnergyType::Psychic])],
        vec![PlayedCard::from_id(CardId::A1211Snorlax)],
    );
    apply(&mut game, 0, attack_action(CardId::A3066Oricorio, 0));
    assert_eq!(game.get_state_clone().get_remaining_hp(1, 0), 100);
}

#[test]
fn test_clear_veil_blocks_energy_discard_from_the_opponents_attack_but_not_damage() {
    // Beedrill ex's Crushing Spear: 80 damage and "Discard a random Energy from your opponent's
    // Active Pokémon." Discarding the Energy is an effect of the attack.
    let defender = |veil: bool| {
        let mut p = dedenne();
        if veil {
            p = p.with_tool(tool(CardId::B4149ClearVeil));
        }
        p
    };
    let energies_after = |veil: bool| {
        let mut game = get_initialized_game_with_board(
            0,
            1,
            3,
            vec![defender(veil)],
            vec![PlayedCard::from_id(CardId::A2b003BeedrillEx)
                .with_energy(vec![EnergyType::Grass, EnergyType::Grass])],
        );
        apply(&mut game, 1, attack_action(CardId::A2b003BeedrillEx, 0));
        game.play_until_stable();
        let state = game.get_state_clone();
        (
            state.get_active(0).attached_energy.len(),
            state.get_remaining_hp(0, 0),
        )
    };
    assert_eq!(energies_after(false), (1, 40));
    assert_eq!(
        energies_after(true),
        (2, 40),
        "Clear Veil keeps the Energy, not the HP"
    );
}

#[test]
fn test_protective_poncho_blocks_bench_damage_from_attacks_only_while_benched() {
    let mut game = get_initialized_game_with_board(
        0,
        1,
        3,
        vec![
            dedenne().with_tool(tool(CardId::B2147ProtectivePoncho)),
            PlayedCard::from_id(CardId::B1121IndeedeeEx)
                .with_tool(tool(CardId::B2147ProtectivePoncho)),
        ],
        vec![
            PlayedCard::from_id(CardId::A3b104ArticunoEx).with_energy(vec![
                EnergyType::Water,
                EnergyType::Water,
                EnergyType::Water,
            ]),
        ],
    );
    apply(&mut game, 1, attack_action(CardId::A3b104ArticunoEx, 1));
    let state = game.get_state_clone();
    assert_eq!(state.get_remaining_hp(0, 0), 40, "the Active takes the 80");
    assert_eq!(
        state.get_remaining_hp(0, 1),
        130,
        "the Benched one takes nothing"
    );
}
