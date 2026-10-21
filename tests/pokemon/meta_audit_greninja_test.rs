//! Audit of Greninja (A1 089) as played in the meta decks (Mega Sceptile ex / Mega Altaria ex /
//! Mega Blaziken ex + Greninja): Froakie -> Greninja through Rare Candy and Water Shuriken
//! ("Once during your turn, you may do 20 damage to 1 of your opponent's Pokémon.").
use deckgym::{
    actions::{Action, SimpleAction},
    card_ids::CardId,
    database::get_card_by_enum,
    models::{EnergyType, PlayedCard},
    state::State,
    test_support::{attack_action, get_test_game_with_board, trainer_from_id},
    Game,
};

fn ability_offered(state: &State, in_play_idx: usize) -> bool {
    let (actor, actions) = state.generate_possible_actions();
    actor == state.current_player
        && actions
            .iter()
            .any(|a| a.action == SimpleAction::UseAbility { in_play_idx })
}

fn apply(game: &mut Game<'static>, actor: usize, action: SimpleAction) {
    game.apply_action(&Action {
        actor,
        action,
        is_stack: false,
    });
}

/// Uses Water Shuriken of the Greninja at `greninja_idx` and returns the targets offered as
/// (player, in_play_idx, damage); then picks `target_idx` of the opponent.
fn water_shuriken(
    game: &mut Game<'static>,
    greninja_idx: usize,
    target_idx: usize,
) -> Vec<(usize, usize, u32)> {
    apply(
        game,
        0,
        SimpleAction::UseAbility {
            in_play_idx: greninja_idx,
        },
    );
    let (actor, actions) = game.get_state_clone().generate_possible_actions();
    assert_eq!(actor, 0, "the Greninja player chooses the target");
    let mut offered = vec![];
    let mut chosen = None;
    for action in actions {
        if let SimpleAction::ApplyDamage { targets, .. } = &action.action {
            assert_eq!(targets.len(), 1, "Water Shuriken hits exactly 1 Pokémon");
            let (damage, player, idx) = targets[0];
            offered.push((player, idx, damage));
            if player == 1 && idx == target_idx {
                chosen = Some(action.clone());
            }
        }
    }
    game.apply_action(&chosen.expect("the requested target should be offered"));
    offered.sort();
    offered
}

/// Player 0 ends the turn, the opponent draws and passes, and player 0 draws.
fn pass_to_next_own_turn(game: &mut Game<'static>) {
    let mut state = game.get_state_clone();
    state.move_generation_stack.clear();
    game.set_state(state);
    for actor in [0, 1] {
        apply(game, actor, SimpleAction::EndTurn);
        let (next, actions) = game.get_state_clone().generate_possible_actions();
        assert_eq!(next, 1 - actor);
        if let Some(draw) = actions
            .iter()
            .find(|a| matches!(a.action, SimpleAction::DrawCard { .. }))
        {
            game.apply_action(draw);
        }
    }
}

fn greninja() -> PlayedCard {
    PlayedCard::from_id(CardId::A1089Greninja)
}

#[test]
fn test_water_shuriken_offers_every_opponent_pokemon_for_20() {
    let mut game = get_test_game_with_board(
        vec![PlayedCard::from_id(CardId::B3008MegaSceptileEx), greninja()],
        vec![
            PlayedCard::from_id(CardId::A1033Charmander),
            PlayedCard::from_id(CardId::A1001Bulbasaur),
            PlayedCard::from_id(CardId::A1033Charmander),
        ],
    );
    assert!(
        ability_offered(&game.get_state_clone(), 1),
        "Water Shuriken works from the Bench"
    );

    let offered = water_shuriken(&mut game, 1, 2);
    assert_eq!(offered, vec![(1, 0, 20), (1, 1, 20), (1, 2, 20)]);

    let state = game.get_state_clone();
    assert_eq!(
        state.get_remaining_hp(1, 2),
        40,
        "20 damage to the chosen Benched Pokémon"
    );
    assert_eq!(
        state.get_remaining_hp(1, 0),
        60,
        "the other Pokémon are not hit"
    );
    assert_eq!(state.get_remaining_hp(1, 1), 70);
}

#[test]
fn test_water_shuriken_ignores_weakness() {
    // Charmander is weak to [W]; Weakness only applies to attacks, so it takes exactly 20.
    let mut game = get_test_game_with_board(
        vec![greninja()],
        vec![PlayedCard::from_id(CardId::A1033Charmander)],
    );
    water_shuriken(&mut game, 0, 0);
    assert_eq!(game.get_state_clone().get_remaining_hp(1, 0), 40);
}

#[test]
fn test_water_shuriken_once_per_turn_per_greninja_and_resets_next_turn() {
    let mut game = get_test_game_with_board(
        vec![greninja(), greninja()],
        vec![PlayedCard::from_id(CardId::A1211Snorlax)],
    );
    water_shuriken(&mut game, 0, 0);
    let state = game.get_state_clone();
    assert!(
        !ability_offered(&state, 0),
        "the same Greninja can't use it twice"
    );
    assert!(
        ability_offered(&state, 1),
        "a second Greninja has its own use"
    );

    water_shuriken(&mut game, 1, 0);
    assert_eq!(game.get_state_clone().get_remaining_hp(1, 0), 110);
    assert!(!ability_offered(&game.get_state_clone(), 1));

    // Next own turn: available again
    pass_to_next_own_turn(&mut game);
    let state = game.get_state_clone();
    assert_eq!(state.current_player, 0);
    assert!(ability_offered(&state, 0));
    assert!(ability_offered(&state, 1));
}

#[test]
fn test_water_shuriken_knock_out_on_bench_gives_a_point() {
    let mut game = get_test_game_with_board(
        vec![greninja()],
        vec![
            PlayedCard::from_id(CardId::A1211Snorlax),
            PlayedCard::from_id(CardId::A1033Charmander).with_remaining_hp(20),
        ],
    );
    water_shuriken(&mut game, 0, 1);
    let state = game.get_state_clone();
    assert!(
        state.in_play_pokemon[1][1].is_none(),
        "the Benched Pokémon is Knocked Out"
    );
    assert_eq!(state.points[0], 1);
}

#[test]
fn test_water_shuriken_goes_through_attack_only_protections() {
    // Oricorio's Safeguard (attacks from ex) and Furfrou's Fur Coat (-20 from attacks) only cover
    // attacks: Water Shuriken still does its 20.
    let mut game = get_test_game_with_board(
        vec![greninja()],
        vec![
            PlayedCard::from_id(CardId::A3066Oricorio),
            PlayedCard::from_id(CardId::B3b061Furfrou),
        ],
    );
    water_shuriken(&mut game, 0, 0);
    water_shuriken_second_turn(&mut game, 1);
    let state = game.get_state_clone();
    assert_eq!(state.get_remaining_hp(1, 0), 50);
    assert_eq!(state.get_remaining_hp(1, 1), 50);
}

/// Resets "used this turn" so a single Greninja can hit a second target in the same test.
fn water_shuriken_second_turn(game: &mut Game<'static>, target_idx: usize) {
    let mut state = game.get_state_clone();
    state.in_play_pokemon[0][0].as_mut().unwrap().ability_used = false;
    game.set_state(state);
    water_shuriken(game, 0, target_idx);
}

#[test]
fn test_water_shuriken_blocked_by_protective_poncho_on_bench() {
    // Protective Poncho: "prevent all damage done to that Pokémon by your opponent's attacks and
    // Abilities" while Benched.
    let mut game = get_test_game_with_board(
        vec![greninja()],
        vec![
            PlayedCard::from_id(CardId::A1211Snorlax),
            PlayedCard::from_id(CardId::B3b024DedenneEx)
                .with_tool(get_card_by_enum(CardId::B2147ProtectivePoncho)),
        ],
    );
    water_shuriken(&mut game, 0, 1);
    assert_eq!(game.get_state_clone().get_remaining_hp(1, 1), 120);
}

#[test]
fn test_mist_slash_does_60_for_water_and_colorless() {
    let mut game = get_test_game_with_board(
        vec![greninja().with_energy(vec![EnergyType::Water, EnergyType::Grass])],
        vec![PlayedCard::from_id(CardId::A1211Snorlax)],
    );
    apply(&mut game, 0, attack_action(CardId::A1089Greninja, 0));
    assert_eq!(game.get_state_clone().get_remaining_hp(1, 0), 90);
}

#[test]
fn test_mist_slash_needs_a_water_energy() {
    let game = get_test_game_with_board(
        vec![greninja().with_energy(vec![EnergyType::Grass, EnergyType::Grass])],
        vec![PlayedCard::from_id(CardId::A1211Snorlax)],
    );
    let (_, actions) = game.get_state_clone().generate_possible_actions();
    assert!(!actions
        .iter()
        .any(|a| matches!(a.action, SimpleAction::Attack(_))));
}

/// Froakie in the Active Spot (with damage and energy), Rare Candy and Greninja in hand.
fn froakie_with_rare_candy(turn_count: u8, played_this_turn: bool) -> Game<'static> {
    let mut froakie = PlayedCard::from_id(CardId::A1087Froakie)
        .with_energy(vec![EnergyType::Water])
        .with_damage(30);
    froakie.played_this_turn = played_this_turn;
    let mut game = get_test_game_with_board(
        vec![froakie],
        vec![PlayedCard::from_id(CardId::A1211Snorlax)],
    );
    let mut state = game.get_state_clone();
    state.turn_count = turn_count;
    state.hands[0] = vec![
        get_card_by_enum(CardId::A3144RareCandy),
        get_card_by_enum(CardId::A1089Greninja),
    ];
    game.set_state(state);
    game
}

fn rare_candy_offered(state: &State) -> bool {
    let (_, actions) = state.generate_possible_actions();
    actions.iter().any(|a| {
        matches!(&a.action, SimpleAction::Play { trainer_card }
            if trainer_card == &trainer_from_id(CardId::A3144RareCandy))
    })
}

#[test]
fn test_rare_candy_froakie_to_greninja_keeps_damage_and_energy() {
    let mut game = froakie_with_rare_candy(3, false);
    assert!(rare_candy_offered(&game.get_state_clone()));
    apply(
        &mut game,
        0,
        SimpleAction::Play {
            trainer_card: trainer_from_id(CardId::A3144RareCandy),
        },
    );
    let (_, actions) = game.get_state_clone().generate_possible_actions();
    let evolve = actions
        .into_iter()
        .find(|a| matches!(&a.action, SimpleAction::Evolve { in_play_idx: 0, .. }))
        .expect("Rare Candy offers Greninja onto Froakie");
    game.apply_action(&evolve);

    let state = game.get_state_clone();
    let active = state.get_active(0);
    assert_eq!(active.get_name(), "Greninja");
    assert_eq!(
        active.get_remaining_hp(),
        90,
        "120 HP minus the 30 damage Froakie had"
    );
    assert_eq!(active.attached_energy, vec![EnergyType::Water]);
    assert!(
        ability_offered(&state, 0),
        "Water Shuriken can be used the turn Greninja evolves"
    );
}

#[test]
fn test_rare_candy_not_on_first_turn_nor_on_froakie_played_this_turn() {
    // Turn 1 is player 0's first turn.
    assert!(!rare_candy_offered(
        &froakie_with_rare_candy(1, false).get_state_clone()
    ));
    assert!(!rare_candy_offered(
        &froakie_with_rare_candy(3, true).get_state_clone()
    ));
}
