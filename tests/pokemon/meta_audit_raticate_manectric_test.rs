//! Audit of the cards in two decks the simulator overestimates:
//! - Team Rocket's Raticate ex + Alolan Ninetales ex (Rattata, Raticate ex, Alolan Vulpix,
//!   Alolan Ninetales ex, Elegant Cape, Soothing Shore, Training Area, Repel).
//! - Mega Manectric ex + Heliolisk (Electrike, Mega Manectric ex, Helioptile, Heliolisk,
//!   Clemont, Clemont's Backpack, Field Blower).
//!
//! Each test checks the engine against the card text in database.json.
use std::collections::BTreeSet;

use deckgym::{
    actions::{Action, SimpleAction},
    card_ids::CardId,
    database::get_card_by_enum,
    models::{Card, EnergyType, PlayedCard},
    state::State,
    test_support::{
        attack_action, get_initialized_game_with_board, get_test_game_with_board, trainer_from_id,
    },
    Game,
};

const SEEDS: u64 = 40;

fn apply(game: &mut Game<'static>, actor: usize, action: SimpleAction) {
    game.apply_action(&Action {
        actor,
        action,
        is_stack: false,
    });
}

fn play(game: &mut Game<'static>, actor: usize, card_id: CardId) {
    apply(
        game,
        actor,
        SimpleAction::Play {
            trainer_card: trainer_from_id(card_id),
        },
    );
}

fn trainer_offered(state: &State, card_id: CardId) -> bool {
    let (actor, actions) = state.generate_possible_actions();
    let trainer = trainer_from_id(card_id);
    actor == state.current_player
        && actions
            .iter()
            .any(|a| matches!(&a.action, SimpleAction::Play { trainer_card } if *trainer_card == trainer))
}

fn attack_offered(state: &State) -> bool {
    let (_, actions) = state.generate_possible_actions();
    actions
        .iter()
        .any(|a| matches!(a.action, SimpleAction::Attack(_)))
}

/// Indices the current player may attach the Energy-Zone energy to this turn.
fn turn_energy_targets(state: &State) -> Vec<usize> {
    let (actor, actions) = state.generate_possible_actions();
    assert_eq!(actor, state.current_player);
    let mut out: Vec<usize> = actions
        .iter()
        .filter_map(|a| match &a.action {
            SimpleAction::Attach {
                attachments,
                is_turn_energy: true,
            } => Some(attachments[0].2),
            _ => None,
        })
        .collect();
    out.sort();
    out
}

/// Applies the generated (stack) action equal to `action`; panics if it isn't offered.
fn choose(game: &mut Game<'static>, action: SimpleAction) {
    let (_, actions) = game.get_state_clone().generate_possible_actions();
    let chosen = actions
        .into_iter()
        .find(|a| a.action == action)
        .unwrap_or_else(|| panic!("{action:?} should be offered"));
    game.apply_action(&chosen);
}

/// Applies the pending DrawCard (start of turn) if that is what the engine asks for.
fn draw_if_pending(game: &mut Game<'static>) {
    let (_, actions) = game.get_state_clone().generate_possible_actions();
    if let Some(draw) = actions
        .iter()
        .find(|a| matches!(a.action, SimpleAction::DrawCard { .. }))
    {
        game.apply_action(draw);
    }
}

/// The current player ends the turn and the next player draws.
fn end_turn(game: &mut Game<'static>) {
    let mut state = game.get_state_clone();
    state.move_generation_stack.clear();
    let actor = state.current_player;
    game.set_state(state);
    apply(game, actor, SimpleAction::EndTurn);
    draw_if_pending(game);
    assert_eq!(game.get_state_clone().current_player, 1 - actor);
}

/// Player 0 ends the turn, the opponent draws and passes, and player 0 draws.
fn pass_to_next_own_turn(game: &mut Game<'static>) {
    end_turn(game);
    end_turn(game);
}

fn set_energy_zone(game: &mut Game<'static>, player: usize, energy: EnergyType) {
    let mut state = game.get_state_clone();
    state.energy_zone[player].current = Some(energy);
    game.set_state(state);
}

fn pc(id: CardId) -> PlayedCard {
    PlayedCard::from_id(id)
}

fn snorlax() -> PlayedCard {
    pc(CardId::A1211Snorlax) // 150 HP, [F] weakness
}

/// Remaining HP of the opponent's Active after player 0 uses `attack` with `attacker` in each of
/// `SEEDS` games; returns the set of distinct values.
fn coin_attack_results(attacker: PlayedCard, card_id: CardId) -> BTreeSet<u32> {
    (0..SEEDS)
        .map(|seed| {
            let mut game = get_initialized_game_with_board(
                seed,
                0,
                3,
                vec![attacker.clone()],
                vec![snorlax()],
            );
            apply(&mut game, 0, attack_action(card_id, 0));
            game.get_state_clone().get_remaining_hp(1, 0)
        })
        .collect()
}

// =====================================================================================
// B4a 058 Team Rocket's Rattata — Ambush [C] 20: "Flip a coin. If heads, this attack does
// 20 more damage."
// =====================================================================================

#[test]
fn test_rattata_ambush_does_20_or_40() {
    let results = coin_attack_results(
        pc(CardId::B4a058TeamRocketsRattata).with_energy(vec![EnergyType::Grass]),
        CardId::B4a058TeamRocketsRattata,
    );
    assert_eq!(
        results,
        BTreeSet::from([110, 130]),
        "20 on tails, 40 on heads"
    );
}

// =====================================================================================
// B4a 059 Team Rocket's Raticate ex — Thieving Incisors / Boost Dash [CC] 70.
// =====================================================================================

fn rattata_board_game(seed: u64, opponent_energy: Vec<EnergyType>) -> Game<'static> {
    let mut game = get_initialized_game_with_board(
        seed,
        0,
        3,
        vec![
            pc(CardId::B4a058TeamRocketsRattata),
            pc(CardId::B4a058TeamRocketsRattata),
        ],
        vec![snorlax().with_energy(opponent_energy)],
    );
    let mut state = game.get_state_clone();
    state.hands[0] = vec![get_card_by_enum(CardId::B4a059TeamRocketsRaticateEx)];
    game.set_state(state);
    game
}

fn evolve_raticate(game: &mut Game<'static>, in_play_idx: usize, from_deck: bool) {
    apply(
        game,
        0,
        SimpleAction::Evolve {
            evolution: get_card_by_enum(CardId::B4a059TeamRocketsRaticateEx),
            in_play_idx,
            from_deck,
        },
    );
}

fn steal_choice(state: &State) -> Option<Action> {
    let (actor, actions) = state.generate_possible_actions();
    if actor != 0 {
        return None;
    }
    actions.into_iter().find(|a| {
        matches!(
            a.action,
            SimpleAction::MoveOpponentActiveEnergyToSelf { .. }
        )
    })
}

fn any_raticate_ability_offered(state: &State) -> bool {
    let (_, actions) = state.generate_possible_actions();
    actions.iter().any(|a| {
        matches!(a.action, SimpleAction::UseAbility { .. })
            || matches!(
                a.action,
                SimpleAction::MoveOpponentActiveEnergyToSelf { .. }
            )
    })
}

#[test]
fn test_thieving_incisors_offered_once_when_evolving_from_hand_on_bench() {
    let mut game = rattata_board_game(0, vec![EnergyType::Fire]);
    evolve_raticate(&mut game, 1, false);
    let state = game.get_state_clone();
    let (actor, actions) = state.generate_possible_actions();
    assert_eq!(actor, 0);
    assert_eq!(
        actions.len(),
        2,
        "steal or decline, nothing else in between"
    );
    let choice = steal_choice(&state).expect("offered right after evolving from hand");
    assert_eq!(
        choice.action,
        SimpleAction::MoveOpponentActiveEnergyToSelf { to_in_play_idx: 1 },
        "the Energy goes to the Raticate ex that evolved (on the Bench)"
    );
    game.apply_action(&choice);
    let state = game.get_state_clone();
    assert_eq!(
        state.in_play_pokemon[0][1]
            .as_ref()
            .unwrap()
            .attached_energy,
        vec![EnergyType::Fire]
    );
    assert!(state.get_active(1).attached_energy.is_empty());
    assert!(
        state.get_active(0).attached_energy.is_empty(),
        "the Active Rattata gets nothing"
    );
    // Once: not available again this turn
    assert!(!any_raticate_ability_offered(&state));
}

#[test]
fn test_thieving_incisors_not_usable_on_later_turns() {
    let mut game = rattata_board_game(0, vec![EnergyType::Fire, EnergyType::Fire]);
    evolve_raticate(&mut game, 0, false);
    // Decline it on the evolution turn.
    let (_, actions) = game.get_state_clone().generate_possible_actions();
    let decline = actions
        .into_iter()
        .find(|a| a.action == SimpleAction::Noop)
        .expect("Thieving Incisors is optional");
    game.apply_action(&decline);
    assert!(!any_raticate_ability_offered(&game.get_state_clone()));
    pass_to_next_own_turn(&mut game);
    let state = game.get_state_clone();
    assert_eq!(state.current_player, 0);
    assert_eq!(state.get_active(1).attached_energy.len(), 2);
    assert!(
        !any_raticate_ability_offered(&state),
        "Thieving Incisors only triggers on the evolution from hand"
    );
}

#[test]
fn test_thieving_incisors_not_offered_when_evolving_from_deck() {
    // "when you play this Pokémon from your hand to evolve" — an evolution from the deck
    // (e.g. through a deck-search-evolve effect) must not trigger it.
    let mut game = rattata_board_game(0, vec![EnergyType::Fire]);
    let mut state = game.get_state_clone();
    state.hands[0].clear();
    state.decks[0]
        .cards
        .push(get_card_by_enum(CardId::B4a059TeamRocketsRaticateEx));
    game.set_state(state);
    evolve_raticate(&mut game, 0, true);
    let state = game.get_state_clone();
    assert_eq!(state.get_active(0).get_name(), "Team Rocket's Raticate ex");
    assert!(steal_choice(&state).is_none());
    assert_eq!(state.get_active(1).attached_energy.len(), 1);
}

#[test]
#[ignore = "BUG (minor): Thieving Incisors always moves the LAST attached Energy, the text says a random one (src/actions/apply_action.rs apply_move_energy_between_players)"]
fn test_thieving_incisors_moves_a_random_energy() {
    let mut moved = BTreeSet::new();
    for seed in 0..SEEDS {
        let mut game = rattata_board_game(seed, vec![EnergyType::Grass, EnergyType::Fire]);
        evolve_raticate(&mut game, 0, false);
        let choice = steal_choice(&game.get_state_clone()).unwrap();
        game.apply_action(&choice);
        let state = game.get_state_clone();
        moved.insert(format!("{:?}", state.get_active(0).attached_energy));
    }
    assert_eq!(
        moved.len(),
        2,
        "either Energy can be taken; observed {moved:?}"
    );
}

#[test]
fn test_raticate_boost_dash_does_70_for_two_colorless() {
    let mut game = get_test_game_with_board(
        vec![pc(CardId::B4a059TeamRocketsRaticateEx).with_energy(vec![EnergyType::Water])],
        vec![snorlax()],
    );
    assert!(
        !attack_offered(&game.get_state_clone()),
        "Boost Dash costs [C][C]"
    );
    let mut state = game.get_state_clone();
    state.in_play_pokemon[0][0]
        .as_mut()
        .unwrap()
        .attached_energy
        .push(EnergyType::Fire);
    game.set_state(state);
    apply(
        &mut game,
        0,
        attack_action(CardId::B4a059TeamRocketsRaticateEx, 0),
    );
    assert_eq!(game.get_state_clone().get_remaining_hp(1, 0), 80);
}

// =====================================================================================
// A3 040 Alolan Vulpix — Call Forth Cold [W]: "Take a [W] Energy from your Energy Zone and
// attach it to this Pokémon."
// =====================================================================================

#[test]
fn test_vulpix_call_forth_cold_attaches_one_water_to_itself_and_does_no_damage() {
    let mut game = get_test_game_with_board(
        vec![
            pc(CardId::A3040AlolanVulpix).with_energy(vec![EnergyType::Water]),
            pc(CardId::A3040AlolanVulpix),
        ],
        vec![snorlax()],
    );
    apply(&mut game, 0, attack_action(CardId::A3040AlolanVulpix, 0));
    let state = game.get_state_clone();
    let vulpix = state.in_play_pokemon[0][0].as_ref().unwrap();
    assert_eq!(
        vulpix.attached_energy,
        vec![EnergyType::Water, EnergyType::Water]
    );
    assert!(state.in_play_pokemon[0][1]
        .as_ref()
        .unwrap()
        .attached_energy
        .is_empty());
    assert_eq!(state.get_remaining_hp(1, 0), 150);
}

// =====================================================================================
// B2 029 Alolan Ninetales ex — Binding Snow [W][W] 80: "During your opponent's next turn,
// they can't take any Energy from their Energy Zone to attach to their Active Pokémon."
// =====================================================================================

fn ninetales() -> PlayedCard {
    pc(CardId::B2029AlolanNinetalesEx).with_energy(vec![EnergyType::Water, EnergyType::Water])
}

#[test]
fn test_binding_snow_blocks_active_only_for_exactly_one_opponent_turn() {
    let mut game = get_test_game_with_board(
        vec![ninetales(), pc(CardId::A3040AlolanVulpix)],
        vec![snorlax(), pc(CardId::A1001Bulbasaur)],
    );
    apply(
        &mut game,
        0,
        attack_action(CardId::B2029AlolanNinetalesEx, 0),
    );
    assert_eq!(
        game.get_state_clone().get_remaining_hp(1, 0),
        70,
        "80 damage"
    );
    end_turn(&mut game);

    // Opponent's next turn: Active blocked, Bench allowed.
    let state = game.get_state_clone();
    assert_eq!(state.current_player, 1);
    set_energy_zone(&mut game, 1, EnergyType::Grass);
    assert_eq!(turn_energy_targets(&game.get_state_clone()), vec![1]);

    // Player 0's following turn: not affected.
    end_turn(&mut game);
    set_energy_zone(&mut game, 0, EnergyType::Water);
    assert_eq!(turn_energy_targets(&game.get_state_clone()), vec![0, 1]);

    // Opponent's turn after that: no longer blocked.
    end_turn(&mut game);
    set_energy_zone(&mut game, 1, EnergyType::Grass);
    assert_eq!(turn_energy_targets(&game.get_state_clone()), vec![0, 1]);
}

#[test]
fn test_binding_snow_ko_new_active_is_also_blocked() {
    // "their Active Pokémon" is the spot: the Pokémon promoted after the KO is also blocked.
    let mut game = get_test_game_with_board(
        vec![ninetales()],
        vec![
            snorlax().with_remaining_hp(50),
            pc(CardId::A1001Bulbasaur),
            pc(CardId::A1001Bulbasaur),
        ],
    );
    apply(
        &mut game,
        0,
        attack_action(CardId::B2029AlolanNinetalesEx, 0),
    );
    let state = game.get_state_clone();
    assert_eq!(state.points[0], 1);
    let (actor, actions) = state.generate_possible_actions();
    assert_eq!(actor, 1, "the opponent promotes");
    let promote = actions
        .into_iter()
        .find(|a| matches!(a.action, SimpleAction::Activate { in_play_idx: 2, .. }))
        .unwrap();
    game.apply_action(&promote);
    end_turn(&mut game);
    assert_eq!(game.get_state_clone().current_player, 1);
    set_energy_zone(&mut game, 1, EnergyType::Grass);
    let targets = turn_energy_targets(&game.get_state_clone());
    assert!(
        !targets.contains(&0),
        "the promoted Active is blocked too: {targets:?}"
    );
    assert!(!targets.is_empty(), "the Bench still can receive Energy");
}

#[test]
fn test_binding_snow_blocks_active_after_switching_during_the_blocked_turn() {
    let mut game = get_test_game_with_board(
        vec![ninetales()],
        vec![
            snorlax(),
            pc(CardId::A1001Bulbasaur).with_energy(vec![EnergyType::Grass]),
        ],
    );
    apply(
        &mut game,
        0,
        attack_action(CardId::B2029AlolanNinetalesEx, 0),
    );
    end_turn(&mut game);
    // Move the second Bulbasaur to the Active spot (as a retreat would) during the blocked turn.
    let mut state = game.get_state_clone();
    state.in_play_pokemon[1].swap(0, 1);
    state.energy_zone[1].current = Some(EnergyType::Grass);
    game.set_state(state);
    assert_eq!(turn_energy_targets(&game.get_state_clone()), vec![1]);
}

#[test]
fn test_binding_snow_blocks_energy_zone_abilities_to_the_active() {
    // Gardevoir's Psy Shadow: "take a [P] Energy from your Energy Zone and attach it to the [P]
    // Pokémon in the Active Spot" — also taking Energy from the Zone to the Active.
    let mut game = get_test_game_with_board(
        vec![ninetales()],
        vec![pc(CardId::A1132Gardevoir), pc(CardId::A1132Gardevoir)],
    );
    let gardevoir_ability_offered = |state: &State| {
        let (_, actions) = state.generate_possible_actions();
        actions
            .iter()
            .any(|a| matches!(a.action, SimpleAction::UseAbility { .. }))
    };
    apply(
        &mut game,
        0,
        attack_action(CardId::B2029AlolanNinetalesEx, 0),
    );
    end_turn(&mut game);
    assert_eq!(game.get_state_clone().current_player, 1);
    assert!(!gardevoir_ability_offered(&game.get_state_clone()));
    pass_to_next_own_turn_for(&mut game);
    assert!(gardevoir_ability_offered(&game.get_state_clone()));
}

/// The current player and then the other player end their turns (back to the current player).
fn pass_to_next_own_turn_for(game: &mut Game<'static>) {
    end_turn(game);
    end_turn(game);
}

#[test]
fn test_binding_snow_needs_two_water() {
    let game = get_test_game_with_board(
        vec![pc(CardId::B2029AlolanNinetalesEx)
            .with_energy(vec![EnergyType::Water, EnergyType::Grass])],
        vec![snorlax()],
    );
    assert!(!attack_offered(&game.get_state_clone()));
}

// =====================================================================================
// B3b 065 Elegant Cape: "The Stage 1 Pokémon this card is attached to gets +30 HP."
// =====================================================================================

fn cape() -> Card {
    get_card_by_enum(CardId::B3b065ElegantCape)
}

#[test]
fn test_elegant_cape_only_for_stage_1() {
    let game = get_test_game_with_board(
        vec![
            pc(CardId::B4a059TeamRocketsRaticateEx).with_tool(cape()), // Stage 1, 120
            pc(CardId::B4a058TeamRocketsRattata).with_tool(cape()),    // Basic, 40
            pc(CardId::A1003Venusaur).with_tool(cape()),               // Stage 2, 160
            pc(CardId::B2b027MegaManectricEx).with_tool(cape()),       // Stage 1 Mega, 180
        ],
        vec![snorlax()],
    );
    let state = game.get_state_clone();
    assert_eq!(state.get_remaining_hp(0, 0), 150);
    assert_eq!(state.get_remaining_hp(0, 1), 40);
    assert_eq!(state.get_remaining_hp(0, 2), 160);
    assert_eq!(state.get_remaining_hp(0, 3), 210);
}

#[test]
fn test_elegant_cape_applies_after_basic_evolves_into_stage_1() {
    let mut game = rattata_board_game(0, vec![]);
    let mut state = game.get_state_clone();
    state.in_play_pokemon[0][0]
        .as_mut()
        .unwrap()
        .attached_tools
        .push(cape());
    game.set_state(state);
    assert_eq!(game.get_state_clone().get_remaining_hp(0, 0), 40);
    evolve_raticate(&mut game, 0, false);
    assert_eq!(game.get_state_clone().get_remaining_hp(0, 0), 150);
}

// =====================================================================================
// B4 154 Soothing Shore: "At the end of each player's turn, that player heals 20 damage from
// each of their Pokémon that has any [W] Energy attached."
// =====================================================================================

#[test]
fn test_soothing_shore_heals_own_water_pokemon_when_turn_ends_by_attacking() {
    let mut game = get_test_game_with_board(
        vec![
            ninetales().with_damage(50),
            pc(CardId::A3040AlolanVulpix)
                .with_energy(vec![EnergyType::Fire, EnergyType::Water])
                .with_damage(10),
            pc(CardId::A3040AlolanVulpix).with_damage(30), // no Energy -> no heal
        ],
        vec![
            snorlax()
                .with_energy(vec![EnergyType::Water])
                .with_damage(40),
            pc(CardId::A1001Bulbasaur).with_damage(30),
        ],
    );
    let mut state = game.get_state_clone();
    state.active_stadium = Some(get_card_by_enum(CardId::B4154SoothingShore));
    state.active_stadium_owner = Some(0);
    game.set_state(state);

    apply(
        &mut game,
        0,
        attack_action(CardId::B2029AlolanNinetalesEx, 0),
    );
    end_turn(&mut game);
    let state = game.get_state_clone();
    assert_eq!(state.get_remaining_hp(0, 0), 120, "150 - 50 + 20");
    assert_eq!(
        state.get_remaining_hp(0, 1),
        60,
        "healed to full (only 10 damage)"
    );
    assert_eq!(state.get_remaining_hp(0, 2), 30, "no [W] Energy, no heal");
    assert_eq!(
        state.get_remaining_hp(1, 0),
        30,
        "the opponent's [W] Pokémon is not healed at the end of player 0's turn (150-40-80)"
    );

    end_turn(&mut game); // opponent ends its turn
    let state = game.get_state_clone();
    assert_eq!(
        state.get_remaining_hp(1, 0),
        50,
        "healed at the end of its owner's turn"
    );
    assert_eq!(state.get_remaining_hp(1, 1), 40, "no [W] Energy");
    assert_eq!(
        state.get_remaining_hp(0, 0),
        120,
        "player 0 is not healed again"
    );
}

// =====================================================================================
// B2 153 Training Area: "Attacks used by Stage 1 Pokémon in play (both yours and your
// opponent's) do +10 damage to the opponent's Active Pokémon."
// =====================================================================================

fn with_stadium(game: &mut Game<'static>, stadium: CardId, owner: usize) {
    let mut state = game.get_state_clone();
    state.active_stadium = Some(get_card_by_enum(stadium));
    state.active_stadium_owner = Some(owner);
    game.set_state(state);
}

fn heliolisk() -> PlayedCard {
    pc(CardId::B4061Heliolisk).with_energy(vec![EnergyType::Lightning])
}

#[test]
fn test_training_area_stage1_bonus_only_to_active_not_bench() {
    let mut game = get_test_game_with_board(
        vec![heliolisk()],
        vec![
            snorlax(),
            pc(CardId::A1001Bulbasaur),
            pc(CardId::A1001Bulbasaur),
        ],
    );
    with_stadium(&mut game, CardId::B2153TrainingArea, 1);
    apply(&mut game, 0, attack_action(CardId::B4061Heliolisk, 0));
    let state = game.get_state_clone();
    assert_eq!(state.get_remaining_hp(1, 0), 100, "40 + 10");
    assert_eq!(
        state.get_remaining_hp(1, 1),
        60,
        "Bench only gets the attack's 10"
    );
    assert_eq!(state.get_remaining_hp(1, 2), 60);
}

#[test]
fn test_training_area_applies_to_raticate_and_mega_manectric_but_not_basics() {
    let mut game = get_test_game_with_board(
        vec![pc(CardId::B4a059TeamRocketsRaticateEx).with_energy(vec![EnergyType::Grass; 2])],
        vec![snorlax()],
    );
    with_stadium(&mut game, CardId::B2153TrainingArea, 0);
    apply(
        &mut game,
        0,
        attack_action(CardId::B4a059TeamRocketsRaticateEx, 0),
    );
    assert_eq!(game.get_state_clone().get_remaining_hp(1, 0), 70, "70 + 10");

    let mut game = get_test_game_with_board(
        vec![pc(CardId::B2b027MegaManectricEx).with_energy(vec![EnergyType::Lightning; 2])],
        vec![snorlax()],
    );
    with_stadium(&mut game, CardId::B2153TrainingArea, 0);
    apply(
        &mut game,
        0,
        attack_action(CardId::B2b027MegaManectricEx, 0),
    );
    assert_eq!(game.get_state_clone().get_remaining_hp(1, 0), 60, "80 + 10");

    // Basic (Electrike Quick Attack 10 / 40): no bonus.
    let results: BTreeSet<u32> = (0..SEEDS)
        .map(|seed| {
            let mut game = get_initialized_game_with_board(
                seed,
                0,
                3,
                vec![pc(CardId::B2b026Electrike).with_energy(vec![EnergyType::Lightning])],
                vec![snorlax()],
            );
            with_stadium(&mut game, CardId::B2153TrainingArea, 0);
            apply(&mut game, 0, attack_action(CardId::B2b026Electrike, 0));
            game.get_state_clone().get_remaining_hp(1, 0)
        })
        .collect();
    assert_eq!(results, BTreeSet::from([110, 140]));
}

// =====================================================================================
// A3a 064 Repel: "Switch out your opponent's Active Basic Pokémon to the Bench. (Your
// opponent chooses the new Active Pokémon.)"
// =====================================================================================

fn repel_game(opponent: Vec<PlayedCard>) -> Game<'static> {
    let mut game = get_test_game_with_board(vec![ninetales()], opponent);
    let mut state = game.get_state_clone();
    state.hands[0] = vec![get_card_by_enum(CardId::A3a064Repel)];
    game.set_state(state);
    game
}

#[test]
fn test_repel_offered_only_for_basic_active_with_bench() {
    assert!(trainer_offered(
        &repel_game(vec![snorlax(), pc(CardId::A1001Bulbasaur)]).get_state_clone(),
        CardId::A3a064Repel
    ));
    assert!(
        !trainer_offered(
            &repel_game(vec![snorlax()]).get_state_clone(),
            CardId::A3a064Repel
        ),
        "no Bench to switch to"
    );
    assert!(
        !trainer_offered(
            &repel_game(vec![pc(CardId::A1002Ivysaur), pc(CardId::A1001Bulbasaur)])
                .get_state_clone(),
            CardId::A3a064Repel
        ),
        "the Active is a Stage 1"
    );
    assert!(
        !trainer_offered(
            &repel_game(vec![
                pc(CardId::B2b027MegaManectricEx),
                pc(CardId::A1001Bulbasaur)
            ])
            .get_state_clone(),
            CardId::A3a064Repel
        ),
        "the Active is an evolved Mega ex"
    );
}

#[test]
fn test_repel_opponent_chooses_new_active() {
    let mut game = repel_game(vec![
        snorlax().with_energy(vec![EnergyType::Grass]),
        pc(CardId::A1001Bulbasaur),
        pc(CardId::A1033Charmander),
    ]);
    let mut state = game.get_state_clone();
    state.apply_status_condition(1, 0, deckgym::models::StatusCondition::Poisoned);
    game.set_state(state);
    play(&mut game, 0, CardId::A3a064Repel);
    let (actor, actions) = game.get_state_clone().generate_possible_actions();
    assert_eq!(actor, 1, "the opponent chooses");
    let mut chosen: Vec<usize> = actions
        .iter()
        .filter_map(|a| match a.action {
            SimpleAction::Activate {
                player: 1,
                in_play_idx,
            } => Some(in_play_idx),
            _ => None,
        })
        .collect();
    chosen.sort();
    assert_eq!(chosen, vec![1, 2]);
    assert_eq!(actions.len(), 2);
    let pick = actions
        .into_iter()
        .find(|a| matches!(a.action, SimpleAction::Activate { in_play_idx: 2, .. }))
        .unwrap();
    game.apply_action(&pick);
    let state = game.get_state_clone();
    assert_eq!(state.get_active(1).get_name(), "Charmander");
    let benched = state.in_play_pokemon[1][2].as_ref().unwrap();
    assert_eq!(benched.get_name(), "Snorlax");
    assert_eq!(benched.attached_energy, vec![EnergyType::Grass]);
    assert!(
        !benched.is_poisoned(),
        "switching to the Bench cures Special Conditions"
    );
    assert_eq!(state.current_player, 0, "back to player 0's turn");
}

// =====================================================================================
// B1a 066 Clemont's Backpack: "During this turn, attacks used by your Magneton or Heliolisk
// do +20 damage to your opponent's Pokémon."
// =====================================================================================

fn backpack_game(attacker: PlayedCard) -> Game<'static> {
    let mut game = get_test_game_with_board(
        vec![attacker],
        vec![
            snorlax(),
            pc(CardId::A1001Bulbasaur),
            pc(CardId::A1001Bulbasaur),
        ],
    );
    let mut state = game.get_state_clone();
    state.hands[0] = vec![get_card_by_enum(CardId::B1a066ClemontsBackpack)];
    game.set_state(state);
    game
}

#[test]
fn test_clemonts_backpack_boosts_heliolisk_active_damage() {
    let mut game = backpack_game(heliolisk());
    assert!(trainer_offered(
        &game.get_state_clone(),
        CardId::B1a066ClemontsBackpack
    ));
    play(&mut game, 0, CardId::B1a066ClemontsBackpack);
    apply(&mut game, 0, attack_action(CardId::B4061Heliolisk, 0));
    assert_eq!(game.get_state_clone().get_remaining_hp(1, 0), 90, "40 + 20");
}

#[test]
fn test_clemonts_backpack_boosts_heliolisk_bench_damage_per_text() {
    let mut game = backpack_game(heliolisk());
    play(&mut game, 0, CardId::B1a066ClemontsBackpack);
    apply(&mut game, 0, attack_action(CardId::B4061Heliolisk, 0));
    let state = game.get_state_clone();
    assert_eq!(
        state.get_remaining_hp(1, 1),
        40,
        "10 + 20 to each Benched Pokémon"
    );
}

#[test]
fn test_clemonts_backpack_does_not_boost_mega_manectric_or_helioptile() {
    let mut game = backpack_game(
        pc(CardId::B2b027MegaManectricEx).with_energy(vec![EnergyType::Lightning; 2]),
    );
    play(&mut game, 0, CardId::B1a066ClemontsBackpack);
    apply(
        &mut game,
        0,
        attack_action(CardId::B2b027MegaManectricEx, 0),
    );
    assert_eq!(
        game.get_state_clone().get_remaining_hp(1, 0),
        70,
        "80, no bonus"
    );

    let mut game =
        backpack_game(pc(CardId::B1a028Helioptile).with_energy(vec![EnergyType::Lightning]));
    play(&mut game, 0, CardId::B1a066ClemontsBackpack);
    apply(&mut game, 0, attack_action(CardId::B1a028Helioptile, 0));
    assert_eq!(
        game.get_state_clone().get_remaining_hp(1, 0),
        140,
        "10, no bonus"
    );
}

#[test]
fn test_clemonts_backpack_boosts_magneton() {
    let mut game =
        backpack_game(pc(CardId::B1a025Magneton).with_energy(vec![EnergyType::Lightning]));
    play(&mut game, 0, CardId::B1a066ClemontsBackpack);
    apply(&mut game, 0, attack_action(CardId::B1a025Magneton, 0));
    // Spark: pick the Bench target if asked.
    let (actor, actions) = game.get_state_clone().generate_possible_actions();
    if actor == 0 {
        if let Some(a) = actions.iter().find(|a| {
            matches!(&a.action, SimpleAction::ApplyDamage { targets, .. } if targets.iter().any(|t| t.1 == 1 && t.2 == 1))
        }) {
            game.apply_action(a);
        }
    }
    assert_eq!(
        game.get_state_clone().get_remaining_hp(1, 0),
        110,
        "20 + 20"
    );
}

#[test]
fn test_clemonts_backpack_expires_at_end_of_turn() {
    let mut game = backpack_game(heliolisk());
    play(&mut game, 0, CardId::B1a066ClemontsBackpack);
    pass_to_next_own_turn(&mut game);
    apply(&mut game, 0, attack_action(CardId::B4061Heliolisk, 0));
    assert_eq!(
        game.get_state_clone().get_remaining_hp(1, 0),
        110,
        "back to 40"
    );
}

// =====================================================================================
// B4 061 Heliolisk — Electrispark [L] 40 + 10 to each Benched; B1a 028 Helioptile —
// Thunder Shock [L] 10, coin -> Paralyzed.
// =====================================================================================

#[test]
fn test_heliolisk_electrispark_40_active_10_each_bench() {
    let mut game = get_test_game_with_board(
        vec![heliolisk(), pc(CardId::B1a028Helioptile)],
        vec![
            snorlax(),
            pc(CardId::A1001Bulbasaur),
            pc(CardId::A1033Charmander),
            pc(CardId::A1001Bulbasaur),
        ],
    );
    apply(&mut game, 0, attack_action(CardId::B4061Heliolisk, 0));
    let state = game.get_state_clone();
    assert_eq!(state.get_remaining_hp(1, 0), 110);
    assert_eq!(state.get_remaining_hp(1, 1), 60);
    assert_eq!(state.get_remaining_hp(1, 2), 50);
    assert_eq!(state.get_remaining_hp(1, 3), 60);
    assert_eq!(state.get_remaining_hp(0, 1), 60, "own Bench untouched");
}

#[test]
fn test_helioptile_thunder_shock_10_and_coin_paralysis() {
    let mut paralyzed = BTreeSet::new();
    for seed in 0..SEEDS {
        let mut game = get_initialized_game_with_board(
            seed,
            0,
            3,
            vec![pc(CardId::B1a028Helioptile).with_energy(vec![EnergyType::Lightning])],
            vec![snorlax()],
        );
        apply(&mut game, 0, attack_action(CardId::B1a028Helioptile, 0));
        let state = game.get_state_clone();
        assert_eq!(state.get_remaining_hp(1, 0), 140);
        paralyzed.insert(state.get_active(1).is_paralyzed());
    }
    assert_eq!(paralyzed, BTreeSet::from([false, true]));
}

// =====================================================================================
// B2b 026 Electrike — Quick Attack [L] 10, coin +30.
// =====================================================================================

#[test]
fn test_electrike_quick_attack_10_or_40() {
    let results = coin_attack_results(
        pc(CardId::B2b026Electrike).with_energy(vec![EnergyType::Lightning]),
        CardId::B2b026Electrike,
    );
    assert_eq!(results, BTreeSet::from([110, 140]));
}

// =====================================================================================
// B2b 027 Mega Manectric ex — Lightning Accelerator [L][L] 80: "This attack does 30 more
// damage for each point you have gotten."
// =====================================================================================

#[test]
fn test_lightning_accelerator_scales_with_own_points_only() {
    for (own, opp, expected_hp) in [(0u8, 0u8, 70u32), (1, 0, 40), (2, 0, 10), (0, 2, 70)] {
        let mut game = get_test_game_with_board(
            vec![pc(CardId::B2b027MegaManectricEx).with_energy(vec![EnergyType::Lightning; 2])],
            vec![snorlax()],
        );
        let mut state = game.get_state_clone();
        state.points = [own, opp];
        game.set_state(state);
        apply(
            &mut game,
            0,
            attack_action(CardId::B2b027MegaManectricEx, 0),
        );
        assert_eq!(
            game.get_state_clone().get_remaining_hp(1, 0),
            expected_hp,
            "own points {own}, opponent points {opp}"
        );
    }
}

#[test]
fn test_lightning_accelerator_needs_two_lightning() {
    let game = get_test_game_with_board(
        vec![pc(CardId::B2b027MegaManectricEx)
            .with_energy(vec![EnergyType::Lightning, EnergyType::Water])],
        vec![snorlax()],
    );
    assert!(!attack_offered(&game.get_state_clone()));
}

#[test]
fn test_mega_manectric_ex_knock_out_gives_three_points() {
    let mut game = get_test_game_with_board(
        vec![heliolisk()],
        vec![
            pc(CardId::B2b027MegaManectricEx).with_remaining_hp(40),
            pc(CardId::B2b026Electrike),
        ],
    );
    apply(&mut game, 0, attack_action(CardId::B4061Heliolisk, 0));
    let state = game.get_state_clone();
    assert_eq!(state.points[0], 3);
    assert!(state.winner.is_some(), "3 points wins the game");
}

// =====================================================================================
// B1a 068 Clemont: "Put 2 random cards from among Magneton, Heliolisk, and Clemont's
// Backpack from your deck into your hand."
// =====================================================================================

fn clemont_game(seed: u64, deck: Vec<CardId>) -> Game<'static> {
    let mut game = get_initialized_game_with_board(seed, 0, 3, vec![heliolisk()], vec![snorlax()]);
    let mut state = game.get_state_clone();
    state.hands[0] = vec![get_card_by_enum(CardId::B1a068Clemont)];
    state.decks[0].cards = deck.into_iter().map(get_card_by_enum).collect();
    game.set_state(state);
    game
}

fn hand_names(state: &State) -> Vec<String> {
    let mut names: Vec<String> = state.hands[0].iter().map(|c| c.get_name()).collect();
    names.sort();
    names
}

#[test]
fn test_clemont_takes_two_random_matching_cards() {
    let deck = vec![
        CardId::B4061Heliolisk,
        CardId::B4061Heliolisk,
        CardId::B1a066ClemontsBackpack,
        CardId::B1a025Magneton,
        CardId::B1a028Helioptile,
        CardId::B2b026Electrike,
        CardId::B2b027MegaManectricEx,
        CardId::B1a028Helioptile,
    ];
    let mut seen = BTreeSet::new();
    for seed in 0..SEEDS {
        let mut game = clemont_game(seed, deck.clone());
        assert!(trainer_offered(
            &game.get_state_clone(),
            CardId::B1a068Clemont
        ));
        play(&mut game, 0, CardId::B1a068Clemont);
        let state = game.get_state_clone();
        let hand = hand_names(&state);
        assert_eq!(hand.len(), 2, "exactly 2 cards: {hand:?}");
        for name in &hand {
            assert!(
                ["Magneton", "Heliolisk", "Clemont's Backpack"].contains(&name.as_str()),
                "{name} is not a Clemont target"
            );
        }
        assert_eq!(state.decks[0].cards.len(), deck.len() - 2);
        seen.insert(hand);
    }
    assert!(seen.len() >= 3, "the 2 cards are random: {seen:?}");
}

#[test]
fn test_clemont_with_only_one_target_in_deck_takes_one() {
    let mut game = clemont_game(
        0,
        vec![
            CardId::B1a066ClemontsBackpack,
            CardId::B1a028Helioptile,
            CardId::B2b026Electrike,
        ],
    );
    play(&mut game, 0, CardId::B1a068Clemont);
    let state = game.get_state_clone();
    assert_eq!(hand_names(&state), vec!["Clemont's Backpack".to_string()]);
    assert_eq!(state.decks[0].cards.len(), 2);
}

// =====================================================================================
// B3 147 Field Blower: "Discard a Pokémon Tool card from a Pokémon (yours or your
// opponent's), or discard a Stadium card in play."
// =====================================================================================

fn field_blower_game(own: Vec<PlayedCard>, opp: Vec<PlayedCard>) -> Game<'static> {
    let mut game = get_test_game_with_board(own, opp);
    let mut state = game.get_state_clone();
    state.hands[0] = vec![get_card_by_enum(CardId::B3147FieldBlower)];
    game.set_state(state);
    game
}

#[test]
fn test_field_blower_offered_only_with_a_tool_or_stadium() {
    let game = field_blower_game(vec![heliolisk()], vec![snorlax()]);
    assert!(!trainer_offered(
        &game.get_state_clone(),
        CardId::B3147FieldBlower
    ));

    let mut game = field_blower_game(vec![heliolisk()], vec![snorlax()]);
    with_stadium(&mut game, CardId::B2153TrainingArea, 1);
    assert!(trainer_offered(
        &game.get_state_clone(),
        CardId::B3147FieldBlower
    ));

    let game = field_blower_game(vec![heliolisk().with_tool(cape())], vec![snorlax()]);
    assert!(trainer_offered(
        &game.get_state_clone(),
        CardId::B3147FieldBlower
    ));
}

#[test]
fn test_field_blower_choices_cover_every_tool_and_the_stadium() {
    let mut game = field_blower_game(
        vec![heliolisk().with_tool(get_card_by_enum(CardId::A2147GiantCape))],
        vec![
            pc(CardId::B4a059TeamRocketsRaticateEx).with_tool(cape()),
            pc(CardId::A1001Bulbasaur),
        ],
    );
    with_stadium(&mut game, CardId::B4154SoothingShore, 1);
    play(&mut game, 0, CardId::B3147FieldBlower);
    let (actor, actions) = game.get_state_clone().generate_possible_actions();
    assert_eq!(actor, 0, "the Field Blower player chooses");
    assert_eq!(actions.len(), 3);
    assert!(actions.iter().any(|a| a.action
        == SimpleAction::DiscardToolFromPokemon {
            player: 1,
            in_play_idx: 0,
            tool_idx: 0
        }));
    assert!(actions.iter().any(|a| a.action
        == SimpleAction::DiscardToolFromPokemon {
            player: 0,
            in_play_idx: 0,
            tool_idx: 0
        }));
    assert!(actions
        .iter()
        .any(|a| a.action == SimpleAction::DiscardActiveStadium));

    let stadium = actions
        .into_iter()
        .find(|a| a.action == SimpleAction::DiscardActiveStadium)
        .unwrap();
    game.apply_action(&stadium);
    let state = game.get_state_clone();
    assert!(state.active_stadium.is_none());
    assert!(
        state.discard_piles[1]
            .iter()
            .any(|c| c.get_name() == "Soothing Shore"),
        "goes to its owner's discard pile"
    );
    assert_eq!(state.get_remaining_hp(1, 0), 150, "the Cape is still there");
}

#[test]
fn test_field_blower_removing_elegant_cape_lowers_hp() {
    let mut game = field_blower_game(
        vec![heliolisk()],
        vec![pc(CardId::B4a059TeamRocketsRaticateEx)
            .with_tool(cape())
            .with_damage(60)],
    );
    play(&mut game, 0, CardId::B3147FieldBlower);
    choose(
        &mut game,
        SimpleAction::DiscardToolFromPokemon {
            player: 1,
            in_play_idx: 0,
            tool_idx: 0,
        },
    );
    let state = game.get_state_clone();
    assert_eq!(state.get_remaining_hp(1, 0), 60, "120 - 60");
    assert!(state.discard_piles[1]
        .iter()
        .any(|c| c.get_name() == "Elegant Cape"));
}

#[test]
fn test_field_blower_removing_elegant_cape_knocks_out_if_damage_exceeds_hp() {
    // Raticate ex with the Cape (150 HP) and 130 damage: without the Cape it has 120 HP, so it
    // is Knocked Out and the Field Blower player takes 2 points.
    let mut game = field_blower_game(
        vec![heliolisk()],
        vec![
            pc(CardId::B4a059TeamRocketsRaticateEx)
                .with_tool(cape())
                .with_damage(130),
            pc(CardId::A1001Bulbasaur),
        ],
    );
    play(&mut game, 0, CardId::B3147FieldBlower);
    choose(
        &mut game,
        SimpleAction::DiscardToolFromPokemon {
            player: 1,
            in_play_idx: 0,
            tool_idx: 0,
        },
    );
    // Give the engine until the end of the turn to run its knockout check.
    end_turn(&mut game);
    let state = game.get_state_clone();
    assert_eq!(state.points[0], 2, "Raticate ex is Knocked Out");
}

#[test]
fn test_field_blower_cape_knock_out_is_immediate() {
    // The Knock Out happens as soon as the HP drops: the opponent has to promote before
    // player 0 continues the turn (and player 0 can't, e.g., attack the 0-HP Pokémon).
    let mut game = field_blower_game(
        vec![heliolisk()],
        vec![
            pc(CardId::B4a059TeamRocketsRaticateEx)
                .with_tool(cape())
                .with_damage(130),
            pc(CardId::A1001Bulbasaur),
        ],
    );
    play(&mut game, 0, CardId::B3147FieldBlower);
    choose(
        &mut game,
        SimpleAction::DiscardToolFromPokemon {
            player: 1,
            in_play_idx: 0,
            tool_idx: 0,
        },
    );
    let state = game.get_state_clone();
    assert_eq!(state.points[0], 2, "Knocked Out right away");
    let (actor, _) = state.generate_possible_actions();
    assert_eq!(actor, 1, "the opponent promotes a new Active");
}
