//! Audit of the partner cards played next to Greninja in the meta decks
//! (Mega Sceptile ex + Greninja, Mega Altaria ex + Greninja, Mega Blaziken ex + Greninja).
//! Each test checks the engine against the card text in database.json: exact damage, targets,
//! conditions, durations, and whether the move generator offers the action to the bot.
use std::collections::BTreeSet;

use deckgym::{
    actions::{expected_attack_damage, Action, SimpleAction},
    card_ids::CardId,
    database::get_card_by_enum,
    models::{EnergyType, PlayedCard, StatusCondition},
    state::State,
    test_support::{
        attack_action, get_initialized_game_with_board, get_test_game_with_board, nth_attack,
        trainer_from_id,
    },
    Game,
};

// ------------------------------------------------------------------------------------------------
// Helpers
// ------------------------------------------------------------------------------------------------

fn pc(id: CardId) -> PlayedCard {
    PlayedCard::from_id(id)
}

fn apply(game: &mut Game<'static>, actor: usize, action: SimpleAction) {
    game.apply_action(&Action {
        actor,
        action,
        is_stack: false,
    });
}

fn offered(state: &State, pred: impl Fn(&SimpleAction) -> bool) -> bool {
    let (actor, actions) = state.generate_possible_actions();
    actor == state.current_player && actions.iter().any(|a| pred(&a.action))
}

fn attack_offered(state: &State, card: CardId, idx: usize) -> bool {
    let attack = nth_attack(card, idx);
    offered(
        state,
        |a| matches!(a, SimpleAction::Attack(x) if *x == attack),
    )
}

fn any_attack_offered(state: &State) -> bool {
    offered(state, |a| matches!(a, SimpleAction::Attack(_)))
}

fn trainer_offered(state: &State, card: CardId) -> bool {
    let trainer = trainer_from_id(card);
    offered(
        state,
        |a| matches!(a, SimpleAction::Play { trainer_card } if *trainer_card == trainer),
    )
}

/// The pending choices (from the move-generation stack) as (actor, actions).
fn pending(game: &Game<'static>) -> (usize, Vec<Action>) {
    game.get_state_clone().generate_possible_actions()
}

/// The single-target `ApplyDamage` choices offered right now, as sorted (player, idx, damage)
/// lists (one list per choice).
fn damage_choices(game: &Game<'static>) -> Vec<Vec<(usize, usize, u32)>> {
    let (_, actions) = pending(game);
    let mut out: Vec<Vec<(usize, usize, u32)>> = actions
        .iter()
        .filter_map(|a| match &a.action {
            SimpleAction::ApplyDamage { targets, .. } => {
                let mut t: Vec<_> = targets.iter().map(|(d, p, i)| (*p, *i, *d)).collect();
                t.sort();
                Some(t)
            }
            _ => None,
        })
        .collect();
    out.sort();
    out
}

/// Picks the `ApplyDamage` choice that hits `(player, idx)` (besides, possibly, the Active).
fn choose_damage_on(game: &mut Game<'static>, player: usize, idx: usize) {
    let (_, actions) = pending(game);
    let choice = actions
        .into_iter()
        .find(|a| match &a.action {
            SimpleAction::ApplyDamage { targets, .. } => {
                targets
                    .iter()
                    .any(|(_, p, i)| *p == player && *i == idx && *i != 0)
                    || (idx == 0 && targets.iter().all(|(_, p, i)| *p == player && *i == 0))
            }
            _ => false,
        })
        .expect("a damage choice on the requested Pokémon should be offered");
    game.apply_action(&choice);
}

/// Ends the current player's turn and lets the next player draw (if a draw is pending).
fn end_turn(game: &mut Game<'static>) {
    let state = game.get_state_clone();
    let actor = state.current_player;
    assert!(
        state.move_generation_stack.is_empty(),
        "no pending choices before ending the turn"
    );
    apply(game, actor, SimpleAction::EndTurn);
    let (next, actions) = pending(game);
    assert_eq!(next, 1 - actor, "the other player is now to act");
    if let Some(draw) = actions
        .iter()
        .find(|a| matches!(a.action, SimpleAction::DrawCard { .. }))
    {
        game.apply_action(draw);
    }
}

fn hp(game: &Game<'static>, player: usize, idx: usize) -> u32 {
    game.get_state_clone().get_remaining_hp(player, idx)
}

fn set_hand(game: &mut Game<'static>, player: usize, cards: Vec<CardId>) {
    let mut state = game.get_state_clone();
    state.hands[player] = cards.into_iter().map(get_card_by_enum).collect();
    game.set_state(state);
}

fn hiking_trail_game(p0: Vec<PlayedCard>, p1: Vec<PlayedCard>, owner: usize) -> Game<'static> {
    let mut game = get_test_game_with_board(p0, p1);
    let mut state = game.get_state_clone();
    state.active_stadium = Some(get_card_by_enum(CardId::B2b069HikingTrail));
    state.active_stadium_owner = Some(owner);
    game.set_state(state);
    game
}

fn rare_candy_game(basic: CardId, stage2: CardId) -> Game<'static> {
    let mut game = get_test_game_with_board(vec![pc(basic)], vec![pc(CardId::A1211Snorlax)]);
    set_hand(&mut game, 0, vec![CardId::A3144RareCandy, stage2]);
    game
}

fn rare_candy_then_evolve(game: &mut Game<'static>) {
    apply(
        game,
        0,
        SimpleAction::Play {
            trainer_card: trainer_from_id(CardId::A3144RareCandy),
        },
    );
    let (_, actions) = pending(game);
    let evolve = actions
        .into_iter()
        .find(|a| matches!(&a.action, SimpleAction::Evolve { in_play_idx: 0, .. }))
        .expect("Rare Candy offers the Stage 2 onto the Basic");
    game.apply_action(&evolve);
}

fn evolve_offered(state: &State, in_play_idx: usize, name: &str) -> bool {
    offered(state, |a| {
        matches!(a, SimpleAction::Evolve { evolution, in_play_idx: i, .. }
            if *i == in_play_idx && evolution.get_name() == name)
    })
}

// ------------------------------------------------------------------------------------------------
// B3 005 Treecko / B3 006 Grovyle / B3 008 Mega Sceptile ex
// ------------------------------------------------------------------------------------------------

#[test]
fn treecko_pound_does_20() {
    let mut game = get_test_game_with_board(
        vec![pc(CardId::B3005Treecko).with_energy(vec![EnergyType::Grass])],
        vec![pc(CardId::A1211Snorlax)],
    );
    assert!(attack_offered(
        &game.get_state_clone(),
        CardId::B3005Treecko,
        0
    ));
    apply(&mut game, 0, attack_action(CardId::B3005Treecko, 0));
    assert_eq!(hp(&game, 1, 0), 130);
}

#[test]
fn treecko_evolves_into_grovyle_and_grovyle_into_mega_sceptile() {
    let mut game = get_test_game_with_board(
        vec![pc(CardId::B3005Treecko), pc(CardId::B3006Grovyle)],
        vec![pc(CardId::A1211Snorlax)],
    );
    set_hand(
        &mut game,
        0,
        vec![CardId::B3006Grovyle, CardId::B3008MegaSceptileEx],
    );
    let state = game.get_state_clone();
    assert!(evolve_offered(&state, 0, "Grovyle"));
    assert!(evolve_offered(&state, 1, "Mega Sceptile ex"));
    assert!(
        !evolve_offered(&state, 0, "Mega Sceptile ex"),
        "Mega Sceptile ex can't go straight onto Treecko without Rare Candy"
    );
}

#[test]
fn grovyle_slicing_snipe_hits_only_bench_for_exactly_50_without_weakness() {
    // Sandshrew is weak to [G]: Weakness only applies to the Active, so it takes exactly 50.
    let mut game = get_test_game_with_board(
        vec![pc(CardId::B3006Grovyle).with_energy(vec![EnergyType::Grass, EnergyType::Grass])],
        vec![
            pc(CardId::A1137Sandshrew),
            pc(CardId::A1137Sandshrew),
            pc(CardId::A1211Snorlax),
        ],
    );
    assert!(attack_offered(
        &game.get_state_clone(),
        CardId::B3006Grovyle,
        0
    ));
    apply(&mut game, 0, attack_action(CardId::B3006Grovyle, 0));
    assert_eq!(
        damage_choices(&game),
        vec![vec![(1, 1, 50)], vec![(1, 2, 50)]],
        "one choice per Benched Pokémon, never the Active"
    );
    choose_damage_on(&mut game, 1, 1);
    assert_eq!(hp(&game, 1, 1), 20, "70 - 50, no Weakness on the Bench");
    assert_eq!(hp(&game, 1, 0), 70, "the Active is not hit");
    assert_eq!(hp(&game, 1, 2), 150);
}

#[test]
fn grovyle_slicing_snipe_needs_two_grass() {
    let game = get_test_game_with_board(
        vec![pc(CardId::B3006Grovyle).with_energy(vec![EnergyType::Grass, EnergyType::Water])],
        vec![pc(CardId::A1211Snorlax), pc(CardId::A1211Snorlax)],
    );
    assert!(!attack_offered(
        &game.get_state_clone(),
        CardId::B3006Grovyle,
        0
    ));
}

#[test]
fn grovyle_slicing_snipe_on_benched_furfrou_is_reduced_by_fur_coat() {
    // Fur Coat: "This Pokémon takes -20 damage from attacks." Slicing Snipe is an attack.
    let mut game = get_test_game_with_board(
        vec![pc(CardId::B3006Grovyle).with_energy(vec![EnergyType::Grass, EnergyType::Grass])],
        vec![pc(CardId::A1211Snorlax), pc(CardId::B3b061Furfrou)],
    );
    apply(&mut game, 0, attack_action(CardId::B3006Grovyle, 0));
    choose_damage_on(&mut game, 1, 1);
    assert_eq!(hp(&game, 1, 1), 40, "70 - (50 - 20)");
}

#[test]
fn mega_sceptile_terminating_tail_needs_two_grass() {
    let state = get_test_game_with_board(
        vec![pc(CardId::B3008MegaSceptileEx)
            .with_energy(vec![EnergyType::Grass, EnergyType::Colorless])],
        vec![pc(CardId::A1211Snorlax)],
    )
    .get_state_clone();
    assert!(!attack_offered(&state, CardId::B3008MegaSceptileEx, 0));

    let state = get_test_game_with_board(
        vec![
            pc(CardId::B3008MegaSceptileEx).with_energy(vec![EnergyType::Grass, EnergyType::Grass])
        ],
        vec![pc(CardId::A1211Snorlax)],
    )
    .get_state_clone();
    assert!(attack_offered(&state, CardId::B3008MegaSceptileEx, 0));
}

#[test]
fn mega_sceptile_terminating_tail_130_discards_one_grass_and_poisons() {
    let mut game = get_test_game_with_board(
        vec![pc(CardId::B3008MegaSceptileEx).with_energy(vec![
            EnergyType::Grass,
            EnergyType::Grass,
            EnergyType::Grass,
        ])],
        vec![pc(CardId::A1211Snorlax)],
    );
    apply(&mut game, 0, attack_action(CardId::B3008MegaSceptileEx, 0));
    let state = game.get_state_clone();
    assert_eq!(state.get_remaining_hp(1, 0), 20, "150 - 130");
    assert!(state.get_active(1).is_poisoned());
    assert_eq!(
        state.get_active(0).attached_energy,
        vec![EnergyType::Grass, EnergyType::Grass],
        "exactly one [G] is discarded"
    );
    assert_eq!(state.discard_energies[0], vec![EnergyType::Grass]);

    // Poison: 10 at the Checkup ending this turn.
    end_turn(&mut game);
    assert_eq!(hp(&game, 1, 0), 10);
}

#[test]
fn mega_sceptile_terminating_tail_applies_weakness_to_active() {
    let mut game = get_test_game_with_board(
        vec![
            pc(CardId::B3008MegaSceptileEx).with_energy(vec![EnergyType::Grass, EnergyType::Grass])
        ],
        vec![pc(CardId::A1211Snorlax)],
    );
    // Snorlax isn't weak to [G]; use a Grass-weak target with 200 HP (a Sandshrew) instead.
    let mut state = game.get_state_clone();
    state.in_play_pokemon[1][0] = Some(PlayedCard::new(
        get_card_by_enum(CardId::A1137Sandshrew),
        0,
        200,
        vec![],
        false,
        vec![],
    ));
    game.set_state(state);
    apply(&mut game, 0, attack_action(CardId::B3008MegaSceptileEx, 0));
    assert_eq!(hp(&game, 1, 0), 50, "200 - (130 + 20 Weakness)");
}

#[test]
fn rare_candy_treecko_to_mega_sceptile_ex_is_offered_and_works() {
    let mut game = rare_candy_game(CardId::B3005Treecko, CardId::B3008MegaSceptileEx);
    assert!(trainer_offered(
        &game.get_state_clone(),
        CardId::A3144RareCandy
    ));
    rare_candy_then_evolve(&mut game);
    let state = game.get_state_clone();
    assert_eq!(state.get_active(0).get_name(), "Mega Sceptile ex");
    assert_eq!(state.get_active(0).get_remaining_hp(), 210);
}

#[test]
fn knocking_out_a_mega_ex_gives_three_points() {
    // Mega Blaziken ex (Fire) KOs Mega Sceptile ex (weak to Fire): 120 + 20 = 140.
    let mut game = get_test_game_with_board(
        vec![pc(CardId::B1036MegaBlazikenEx).with_energy(vec![EnergyType::Fire, EnergyType::Fire])],
        vec![
            pc(CardId::B3008MegaSceptileEx).with_remaining_hp(140),
            pc(CardId::B3005Treecko),
        ],
    );
    apply(&mut game, 0, attack_action(CardId::B1036MegaBlazikenEx, 0));
    let state = game.get_state_clone();
    assert_eq!(state.points[0], 3, "a Mega ex is worth 3 points");
    assert!(state.winner.is_some(), "3 points wins the game");
}

#[test]
fn knocking_out_mega_altaria_ex_gives_three_points() {
    let mut game = get_test_game_with_board(
        vec![
            pc(CardId::B3008MegaSceptileEx).with_energy(vec![EnergyType::Grass, EnergyType::Grass])
        ],
        vec![
            pc(CardId::B1102MegaAltariaEx).with_remaining_hp(130),
            pc(CardId::B1196Swablu),
        ],
    );
    apply(&mut game, 0, attack_action(CardId::B3008MegaSceptileEx, 0));
    assert_eq!(game.get_state_clone().points[0], 3);
}

// ------------------------------------------------------------------------------------------------
// A3a 007 Pheromosa
// ------------------------------------------------------------------------------------------------

#[test]
fn pheromosa_jump_blues_20_active_and_20_to_one_benched() {
    // Both opponent Pokémon are Sandshrew (weak to [G]): Weakness only on the Active.
    let mut game = get_test_game_with_board(
        vec![pc(CardId::A3a007Pheromosa).with_energy(vec![EnergyType::Grass])],
        vec![
            pc(CardId::A1137Sandshrew),
            pc(CardId::A1211Snorlax),
            pc(CardId::A1137Sandshrew),
        ],
    );
    assert!(attack_offered(
        &game.get_state_clone(),
        CardId::A3a007Pheromosa,
        0
    ));
    apply(&mut game, 0, attack_action(CardId::A3a007Pheromosa, 0));
    assert_eq!(
        damage_choices(&game),
        vec![vec![(1, 0, 20), (1, 1, 20)], vec![(1, 0, 20), (1, 2, 20)]]
    );
    choose_damage_on(&mut game, 1, 2);
    assert_eq!(hp(&game, 1, 0), 30, "70 - (20 + 20 Weakness)");
    assert_eq!(hp(&game, 1, 2), 50, "70 - 20, no Weakness on the Bench");
    assert_eq!(hp(&game, 1, 1), 150);
}

#[test]
fn pheromosa_jump_blues_without_bench_still_hits_active() {
    let mut game = get_test_game_with_board(
        vec![pc(CardId::A3a007Pheromosa).with_energy(vec![EnergyType::Grass])],
        vec![pc(CardId::A1211Snorlax)],
    );
    apply(&mut game, 0, attack_action(CardId::A3a007Pheromosa, 0));
    assert!(damage_choices(&game).is_empty());
    assert_eq!(hp(&game, 1, 0), 130);
}

// ------------------------------------------------------------------------------------------------
// B2b 069 Hiking Trail
// ------------------------------------------------------------------------------------------------

#[test]
fn hiking_trail_tops_up_both_players_at_end_of_their_turn() {
    // Owned by player 1, still affects player 0 ("each player").
    let mut game = hiking_trail_game(
        vec![pc(CardId::A1211Snorlax)],
        vec![pc(CardId::A1211Snorlax)],
        1,
    );
    set_hand(&mut game, 0, vec![CardId::A1001Bulbasaur]);
    set_hand(&mut game, 1, vec![]);
    let deck0 = game.get_state_clone().decks[0].cards.len();
    let deck1 = game.get_state_clone().decks[1].cards.len();

    end_turn(&mut game); // player 0 ends: 1 -> 3; player 1 draws 1 at turn start
    let state = game.get_state_clone();
    assert_eq!(state.hands[0].len(), 3);
    assert_eq!(state.decks[0].cards.len(), deck0 - 2);
    assert_eq!(
        state.hands[1].len(),
        1,
        "player 1 only drew for the turn so far"
    );

    end_turn(&mut game); // player 1 ends: 1 -> 3
    let state = game.get_state_clone();
    assert_eq!(state.hands[1].len(), 3);
    assert_eq!(state.decks[1].cards.len(), deck1 - 3);
    // Player 0 drew 1 at the start of their turn (3 -> 4); Hiking Trail didn't touch them at the
    // end of player 1's turn.
    assert_eq!(state.hands[0].len(), 4);
}

#[test]
fn hiking_trail_draws_nothing_with_three_or_more_cards() {
    let mut game = hiking_trail_game(
        vec![pc(CardId::A1211Snorlax)],
        vec![pc(CardId::A1211Snorlax)],
        0,
    );
    set_hand(
        &mut game,
        0,
        vec![
            CardId::A1001Bulbasaur,
            CardId::A1001Bulbasaur,
            CardId::A1001Bulbasaur,
        ],
    );
    let deck0 = game.get_state_clone().decks[0].cards.len();
    end_turn(&mut game);
    let state = game.get_state_clone();
    assert_eq!(state.hands[0].len(), 3);
    assert_eq!(state.decks[0].cards.len(), deck0);
}

#[test]
fn hiking_trail_with_empty_deck_does_not_crash() {
    let mut game = hiking_trail_game(
        vec![pc(CardId::A1211Snorlax)],
        vec![pc(CardId::A1211Snorlax)],
        0,
    );
    let mut state = game.get_state_clone();
    state.hands[0].clear();
    state.decks[0].cards.clear();
    game.set_state(state);
    end_turn(&mut game);
    let state = game.get_state_clone();
    assert_eq!(state.hands[0].len(), 0);
    assert_eq!(state.current_player, 1);
    assert!(state.winner.is_none());
}

#[test]
fn hiking_trail_also_tops_up_after_an_attack_ends_the_turn() {
    let mut game = hiking_trail_game(
        vec![pc(CardId::A1087Froakie).with_energy(vec![EnergyType::Water])],
        vec![pc(CardId::A1211Snorlax)],
        0,
    );
    set_hand(&mut game, 0, vec![]);
    apply(&mut game, 0, attack_action(CardId::A1087Froakie, 0));
    end_turn(&mut game);
    assert_eq!(game.get_state_clone().hands[0].len(), 3);
}

// ------------------------------------------------------------------------------------------------
// B3b 061 Furfrou
// ------------------------------------------------------------------------------------------------

#[test]
fn furfrou_fur_coat_reduces_active_attack_damage_by_20() {
    let mut game = get_test_game_with_board(
        vec![pc(CardId::A1001Bulbasaur).with_energy(vec![EnergyType::Grass, EnergyType::Grass])],
        vec![pc(CardId::B3b061Furfrou)],
    );
    apply(&mut game, 0, attack_action(CardId::A1001Bulbasaur, 0));
    assert_eq!(hp(&game, 1, 0), 50, "70 - (40 - 20)");
}

#[test]
fn furfrou_fur_coat_is_applied_after_weakness() {
    // Machop (Fighting) Knuckle Punch 20, Furfrou weak to [F]: 20 + 20 - 20 = 20.
    let mut game = get_test_game_with_board(
        vec![pc(CardId::A1143Machop).with_energy(vec![EnergyType::Fighting])],
        vec![pc(CardId::B3b061Furfrou)],
    );
    apply(&mut game, 0, attack_action(CardId::A1143Machop, 0));
    assert_eq!(hp(&game, 1, 0), 50);
}

#[test]
fn furfrou_fur_coat_does_not_reduce_poison() {
    let mut game = get_test_game_with_board(
        vec![pc(CardId::A1211Snorlax)],
        vec![pc(CardId::B3b061Furfrou)],
    );
    let mut state = game.get_state_clone();
    state.apply_status_condition(1, 0, StatusCondition::Poisoned);
    game.set_state(state);
    end_turn(&mut game);
    assert_eq!(hp(&game, 1, 0), 60, "Poison is not attack damage");
}

#[test]
fn furfrou_double_headbutt_does_0_30_or_60() {
    let mut seen = BTreeSet::new();
    for seed in 0..40 {
        let mut game = get_initialized_game_with_board(
            seed,
            0,
            3,
            vec![pc(CardId::B3b061Furfrou).with_energy(vec![EnergyType::Colorless])],
            vec![pc(CardId::A1211Snorlax)],
        );
        assert!(attack_offered(
            &game.get_state_clone(),
            CardId::B3b061Furfrou,
            0
        ));
        apply(&mut game, 0, attack_action(CardId::B3b061Furfrou, 0));
        seen.insert(150 - hp(&game, 1, 0));
    }
    assert_eq!(seen, BTreeSet::from([0, 30, 60]));

    let state = get_test_game_with_board(
        vec![pc(CardId::B3b061Furfrou).with_energy(vec![EnergyType::Colorless])],
        vec![pc(CardId::A1211Snorlax)],
    )
    .get_state_clone();
    let expected = expected_attack_damage(&state, &nth_attack(CardId::B3b061Furfrou, 0)).unwrap();
    assert!(
        (expected - 30.0).abs() < 1e-9,
        "E = 2 * 0.5 * 30, got {expected}"
    );
}

// ------------------------------------------------------------------------------------------------
// B3b 064 Small Balloon
// ------------------------------------------------------------------------------------------------

fn retreat_offered(state: &State) -> bool {
    offered(state, |a| matches!(a, SimpleAction::Retreat(_)))
}

fn balloon() -> deckgym::models::Card {
    get_card_by_enum(CardId::B3b064SmallBalloon)
}

#[test]
fn small_balloon_basic_furfrou_retreats_for_free_and_keeps_energy() {
    let mut game = get_test_game_with_board(
        vec![
            pc(CardId::B3b061Furfrou)
                .with_tool(balloon())
                .with_energy(vec![EnergyType::Colorless]),
            pc(CardId::A1087Froakie),
        ],
        vec![pc(CardId::A1211Snorlax)],
    );
    assert!(retreat_offered(&game.get_state_clone()));
    apply(&mut game, 0, SimpleAction::Retreat(1));
    let state = game.get_state_clone();
    assert_eq!(state.get_active(0).get_name(), "Froakie");
    assert_eq!(
        state.in_play_pokemon[0][1]
            .as_ref()
            .unwrap()
            .attached_energy,
        vec![EnergyType::Colorless],
        "Retreat Cost 1 - 1 = 0: no Energy discarded"
    );
}

#[test]
fn small_balloon_does_nothing_on_evolved_pokemon() {
    for evolved in [
        CardId::B3006Grovyle,
        CardId::B3008MegaSceptileEx,
        CardId::B1102MegaAltariaEx,
    ] {
        let state = get_test_game_with_board(
            vec![pc(evolved).with_tool(balloon()), pc(CardId::A1087Froakie)],
            vec![pc(CardId::A1211Snorlax)],
        )
        .get_state_clone();
        assert!(
            !retreat_offered(&state),
            "{evolved:?} is not Basic: Retreat Cost stays 1 with no Energy"
        );
    }
}

// ------------------------------------------------------------------------------------------------
// B1 102 Mega Altaria ex / B1 196 Swablu
// ------------------------------------------------------------------------------------------------

#[test]
fn mega_altaria_mega_harmony_40_plus_30_per_own_benched() {
    for bench in 0..=3usize {
        let mut mine = vec![pc(CardId::B1102MegaAltariaEx)
            .with_energy(vec![EnergyType::Psychic, EnergyType::Psychic])];
        for _ in 0..bench {
            mine.push(pc(CardId::B1196Swablu));
        }
        // The opponent's Bench doesn't count.
        let mut game = get_test_game_with_board(
            mine,
            vec![
                pc(CardId::A1211Snorlax),
                pc(CardId::A1087Froakie),
                pc(CardId::A1087Froakie),
            ],
        );
        assert!(attack_offered(
            &game.get_state_clone(),
            CardId::B1102MegaAltariaEx,
            0
        ));
        apply(&mut game, 0, attack_action(CardId::B1102MegaAltariaEx, 0));
        assert_eq!(
            150 - hp(&game, 1, 0),
            40 + 30 * bench as u32,
            "with {bench} Benched Pokémon"
        );
    }
}

#[test]
fn swablu_sing_puts_active_to_sleep_without_damage() {
    let mut game = get_test_game_with_board(
        vec![pc(CardId::B1196Swablu).with_energy(vec![EnergyType::Water])],
        vec![
            pc(CardId::A1001Bulbasaur).with_energy(vec![EnergyType::Grass, EnergyType::Grass]),
            pc(CardId::A1087Froakie),
        ],
    );
    assert!(attack_offered(
        &game.get_state_clone(),
        CardId::B1196Swablu,
        0
    ));
    apply(&mut game, 0, attack_action(CardId::B1196Swablu, 0));
    let state = game.get_state_clone();
    assert!(state.get_active(1).is_asleep());
    assert_eq!(state.get_remaining_hp(1, 0), 70);
    assert!(!state.in_play_pokemon[1][1].as_ref().unwrap().is_asleep());
}

#[test]
fn asleep_pokemon_cannot_attack_or_retreat() {
    // Find a seed where Bulbasaur stays Asleep through the Checkup (tails).
    for seed in 0..30 {
        let mut game = get_initialized_game_with_board(
            seed,
            0,
            3,
            vec![pc(CardId::B1196Swablu).with_energy(vec![EnergyType::Water])],
            vec![
                pc(CardId::A1001Bulbasaur).with_energy(vec![EnergyType::Grass, EnergyType::Grass]),
                pc(CardId::A1087Froakie),
            ],
        );
        apply(&mut game, 0, attack_action(CardId::B1196Swablu, 0));
        end_turn(&mut game);
        let state = game.get_state_clone();
        if state.get_active(1).is_asleep() {
            assert!(!any_attack_offered(&state));
            assert!(!retreat_offered(&state));
            return;
        }
    }
    panic!("never stayed Asleep in 30 seeds");
}

// ------------------------------------------------------------------------------------------------
// B4 078 Oricorio
// ------------------------------------------------------------------------------------------------

#[test]
fn oricorio_supernatural_feather_discards_a_card_and_does_50() {
    let mut game = get_test_game_with_board(
        vec![pc(CardId::B4078Oricorio).with_energy(vec![EnergyType::Psychic])],
        vec![pc(CardId::A1211Snorlax)],
    );
    set_hand(
        &mut game,
        0,
        vec![CardId::A1001Bulbasaur, CardId::PA001Potion],
    );
    assert!(attack_offered(
        &game.get_state_clone(),
        CardId::B4078Oricorio,
        0
    ));
    apply(&mut game, 0, attack_action(CardId::B4078Oricorio, 0));

    let (actor, actions) = pending(&game);
    assert_eq!(actor, 0);
    let discards: Vec<_> = actions
        .iter()
        .filter_map(|a| match &a.action {
            SimpleAction::DiscardOwnCards { cards } => Some(cards.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(discards.len(), 2, "one choice per card in hand");
    assert!(discards.iter().all(|c| c.len() == 1), "exactly 1 card");
    let bulbasaur = get_card_by_enum(CardId::A1001Bulbasaur);
    let choice = actions
        .into_iter()
        .find(|a| matches!(&a.action, SimpleAction::DiscardOwnCards { cards } if cards == &vec![bulbasaur.clone()]))
        .unwrap();
    game.apply_action(&choice);

    let state = game.get_state_clone();
    assert_eq!(state.get_remaining_hp(1, 0), 100);
    assert_eq!(state.hands[0], vec![get_card_by_enum(CardId::PA001Potion)]);
    assert!(state.discard_piles[0].contains(&bulbasaur));
}

#[test]
fn oricorio_supernatural_feather_with_empty_hand_is_offered_but_does_nothing() {
    let mut game = get_test_game_with_board(
        vec![pc(CardId::B4078Oricorio).with_energy(vec![EnergyType::Psychic])],
        vec![pc(CardId::A1211Snorlax)],
    );
    set_hand(&mut game, 0, vec![]);
    assert!(
        attack_offered(&game.get_state_clone(), CardId::B4078Oricorio, 0),
        "the attack can still be used (it just does nothing)"
    );
    apply(&mut game, 0, attack_action(CardId::B4078Oricorio, 0));
    let state = game.get_state_clone();
    assert_eq!(state.get_remaining_hp(1, 0), 150, "does nothing");
    assert!(state.move_generation_stack.is_empty());
}

// ------------------------------------------------------------------------------------------------
// B3 078 Bonsly
// ------------------------------------------------------------------------------------------------

fn bonsly_game(p0_bench: Vec<PlayedCard>, p1: Vec<PlayedCard>) -> Game<'static> {
    let mut mine = vec![pc(CardId::B3078Bonsly)];
    mine.extend(p0_bench);
    get_test_game_with_board(mine, p1)
}

fn bulbasaur_charged() -> PlayedCard {
    pc(CardId::A1001Bulbasaur).with_energy(vec![EnergyType::Grass, EnergyType::Grass])
}

#[test]
fn bonsly_teary_attack_is_free_and_does_10() {
    // (Bulbasaur, not Snorlax: Snorlax is weak to Bonsly's [F].)
    let mut game = bonsly_game(vec![], vec![pc(CardId::A1001Bulbasaur)]);
    assert!(
        attack_offered(&game.get_state_clone(), CardId::B3078Bonsly, 0),
        "zero-cost attack offered with no Energy"
    );
    apply(&mut game, 0, attack_action(CardId::B3078Bonsly, 0));
    assert_eq!(hp(&game, 1, 0), 60);
}

#[test]
fn bonsly_teary_attack_reduction_lasts_only_opponents_next_turn() {
    // Bonsly at 30 HP vs Bulbasaur's Vine Whip (40).
    let mut game = bonsly_game(vec![pc(CardId::A1211Snorlax)], vec![bulbasaur_charged()]);
    apply(&mut game, 0, attack_action(CardId::B3078Bonsly, 0));
    end_turn(&mut game);

    // Opponent's next turn: 40 - 30 = 10.
    apply(&mut game, 1, attack_action(CardId::A1001Bulbasaur, 0));
    assert_eq!(hp(&game, 0, 0), 20);
    end_turn(&mut game);

    // Bonsly's turn passes without attacking.
    end_turn(&mut game);

    // The turn after: full 40 -> Bonsly (20 HP) is Knocked Out.
    apply(&mut game, 1, attack_action(CardId::A1001Bulbasaur, 0));
    let state = game.get_state_clone();
    assert_eq!(
        state.points[1], 1,
        "the reduction expired: Bonsly is Knocked Out"
    );
}

#[test]
#[ignore = "BUG: Teary Attack is modelled as 'Bonsly takes -30' (CardEffect::ReducedDamage on the attacker, effect_mechanic_map.rs:472) instead of a debuff on the Defending Pokemon's attacks"]
fn bonsly_teary_attack_reduces_the_defending_pokemon_even_against_another_target() {
    // "attacks used by the Defending Pokémon do -30 damage": the debuff is on the Defending
    // Pokémon. If Bonsly is switched out (opponent's Sabrina), the Defending Pokémon's attack on
    // the new Active still does -30.
    let mut game = bonsly_game(vec![pc(CardId::A1211Snorlax)], vec![bulbasaur_charged()]);
    apply(&mut game, 0, attack_action(CardId::B3078Bonsly, 0));
    end_turn(&mut game);

    set_hand(&mut game, 1, vec![CardId::A1225Sabrina]);
    apply(
        &mut game,
        1,
        SimpleAction::Play {
            trainer_card: trainer_from_id(CardId::A1225Sabrina),
        },
    );
    let (actor, actions) = pending(&game);
    assert_eq!(actor, 0, "Bonsly's owner picks the new Active");
    let activate = actions
        .into_iter()
        .find(|a| matches!(a.action, SimpleAction::Activate { in_play_idx: 1, .. }))
        .unwrap();
    game.apply_action(&activate);
    assert_eq!(game.get_state_clone().get_active(0).get_name(), "Snorlax");

    apply(&mut game, 1, attack_action(CardId::A1001Bulbasaur, 0));
    assert_eq!(
        hp(&game, 0, 0),
        140,
        "Snorlax should take 40 - 30 = 10 from the Defending Pokémon"
    );
}

#[test]
#[ignore = "BUG: Teary Attack is modelled as 'Bonsly takes -30' (CardEffect::ReducedDamage on the attacker, effect_mechanic_map.rs:472) instead of a debuff on the Defending Pokemon's attacks"]
fn bonsly_teary_attack_does_not_protect_bonsly_from_a_different_attacker() {
    // The Defending Pokémon retreats; a different Pokémon (not affected by Teary Attack) attacks
    // Bonsly for its full 40 -> Bonsly (30 HP) is Knocked Out.
    let mut game = bonsly_game(
        vec![pc(CardId::A1211Snorlax)],
        vec![
            pc(CardId::A1001Bulbasaur).with_energy(vec![EnergyType::Grass]),
            bulbasaur_charged(),
        ],
    );
    apply(&mut game, 0, attack_action(CardId::B3078Bonsly, 0));
    end_turn(&mut game);

    assert!(retreat_offered(&game.get_state_clone()));
    apply(&mut game, 1, SimpleAction::Retreat(1));
    apply(&mut game, 1, attack_action(CardId::A1001Bulbasaur, 0));
    let state = game.get_state_clone();
    assert_eq!(
        state.points[1],
        1,
        "the new attacker isn't the Defending Pokémon: full 40 should KO Bonsly (HP left: {})",
        state.get_remaining_hp(0, 0)
    );
}

// ------------------------------------------------------------------------------------------------
// B1 033 Torchic / B1 034 Combusken / B1 036 Mega Blaziken ex
// ------------------------------------------------------------------------------------------------

#[test]
fn torchic_peck_20_and_combusken_high_jump_kick_50() {
    let mut game = get_test_game_with_board(
        vec![pc(CardId::B1033Torchic).with_energy(vec![EnergyType::Fire])],
        vec![pc(CardId::A1211Snorlax)],
    );
    apply(&mut game, 0, attack_action(CardId::B1033Torchic, 0));
    assert_eq!(hp(&game, 1, 0), 130);

    let state = get_test_game_with_board(
        vec![pc(CardId::B1034Combusken).with_energy(vec![EnergyType::Fire, EnergyType::Grass])],
        vec![pc(CardId::A1211Snorlax)],
    )
    .get_state_clone();
    assert!(
        !attack_offered(&state, CardId::B1034Combusken, 0),
        "needs [R][R]"
    );

    let mut game = get_test_game_with_board(
        vec![pc(CardId::B1034Combusken).with_energy(vec![EnergyType::Fire, EnergyType::Fire])],
        vec![pc(CardId::A1211Snorlax)],
    );
    assert!(attack_offered(
        &game.get_state_clone(),
        CardId::B1034Combusken,
        0
    ));
    apply(&mut game, 0, attack_action(CardId::B1034Combusken, 0));
    assert_eq!(hp(&game, 1, 0), 100);
}

#[test]
fn torchic_line_evolutions_and_rare_candy_to_mega_blaziken_ex() {
    let mut game = get_test_game_with_board(
        vec![pc(CardId::B1033Torchic), pc(CardId::B1034Combusken)],
        vec![pc(CardId::A1211Snorlax)],
    );
    set_hand(
        &mut game,
        0,
        vec![CardId::B1034Combusken, CardId::B1036MegaBlazikenEx],
    );
    let state = game.get_state_clone();
    assert!(evolve_offered(&state, 0, "Combusken"));
    assert!(evolve_offered(&state, 1, "Mega Blaziken ex"));

    let mut game = rare_candy_game(CardId::B1033Torchic, CardId::B1036MegaBlazikenEx);
    assert!(trainer_offered(
        &game.get_state_clone(),
        CardId::A3144RareCandy
    ));
    rare_candy_then_evolve(&mut game);
    assert_eq!(
        game.get_state_clone().get_active(0).get_name(),
        "Mega Blaziken ex"
    );
}

#[test]
fn mega_blaziken_mega_burning_120_discards_one_fire_and_burns() {
    let mut game = get_test_game_with_board(
        vec![pc(CardId::B1036MegaBlazikenEx).with_energy(vec![
            EnergyType::Fire,
            EnergyType::Fire,
            EnergyType::Grass,
        ])],
        vec![pc(CardId::A1211Snorlax)],
    );
    assert!(attack_offered(
        &game.get_state_clone(),
        CardId::B1036MegaBlazikenEx,
        0
    ));
    apply(&mut game, 0, attack_action(CardId::B1036MegaBlazikenEx, 0));
    let state = game.get_state_clone();
    assert_eq!(state.get_remaining_hp(1, 0), 30, "150 - 120");
    assert!(state.get_active(1).is_burned());
    let mut left = state.get_active(0).attached_energy.clone();
    left.sort_by_key(|e| format!("{e:?}"));
    assert_eq!(
        left,
        vec![EnergyType::Fire, EnergyType::Grass],
        "exactly one [R] is discarded"
    );
    assert_eq!(state.discard_energies[0], vec![EnergyType::Fire]);
}

#[test]
fn mega_blaziken_mega_burning_needs_two_fire() {
    let state =
        get_test_game_with_board(
            vec![pc(CardId::B1036MegaBlazikenEx)
                .with_energy(vec![EnergyType::Fire, EnergyType::Grass])],
            vec![pc(CardId::A1211Snorlax)],
        )
        .get_state_clone();
    assert!(!attack_offered(&state, CardId::B1036MegaBlazikenEx, 0));
}

#[test]
fn burn_does_20_at_checkup_and_a_coin_decides_recovery() {
    let mut recovered = BTreeSet::new();
    for seed in 0..30 {
        let mut game = get_initialized_game_with_board(
            seed,
            0,
            3,
            vec![pc(CardId::B1036MegaBlazikenEx)
                .with_energy(vec![EnergyType::Fire, EnergyType::Fire])],
            vec![pc(CardId::A1211Snorlax)],
        );
        apply(&mut game, 0, attack_action(CardId::B1036MegaBlazikenEx, 0));
        end_turn(&mut game);
        let state = game.get_state_clone();
        assert_eq!(state.get_remaining_hp(1, 0), 10, "30 - 20 Burn");
        recovered.insert(!state.get_active(1).is_burned());
    }
    assert_eq!(
        recovered,
        BTreeSet::from([false, true]),
        "both coin results happen"
    );
}

// ------------------------------------------------------------------------------------------------
// B1 217 Flame Patch
// ------------------------------------------------------------------------------------------------

fn flame_patch_game(active: PlayedCard, discard: Vec<EnergyType>) -> Game<'static> {
    let mut game = get_test_game_with_board(
        vec![active, pc(CardId::B1033Torchic)],
        vec![pc(CardId::A1211Snorlax)],
    );
    let mut state = game.get_state_clone();
    state.hands[0] = vec![get_card_by_enum(CardId::B1217FlamePatch)];
    state.discard_energies[0] = discard;
    game.set_state(state);
    game
}

#[test]
fn flame_patch_attaches_fire_from_discard_to_active_fire() {
    let mut game = flame_patch_game(
        pc(CardId::B1036MegaBlazikenEx).with_energy(vec![EnergyType::Fire]),
        vec![EnergyType::Water, EnergyType::Fire],
    );
    assert!(trainer_offered(
        &game.get_state_clone(),
        CardId::B1217FlamePatch
    ));
    apply(
        &mut game,
        0,
        SimpleAction::Play {
            trainer_card: trainer_from_id(CardId::B1217FlamePatch),
        },
    );
    let state = game.get_state_clone();
    assert!(state.move_generation_stack.is_empty());
    assert_eq!(
        state.get_active(0).attached_energy,
        vec![EnergyType::Fire, EnergyType::Fire]
    );
    assert_eq!(state.discard_energies[0], vec![EnergyType::Water]);
    assert!(
        state.in_play_pokemon[0][1]
            .as_ref()
            .unwrap()
            .attached_energy
            .is_empty(),
        "Benched Fire Pokémon don't receive it"
    );
    assert!(state.hands[0].is_empty());
}

#[test]
fn flame_patch_not_offered_without_fire_in_discard() {
    let game = flame_patch_game(pc(CardId::B1033Torchic), vec![EnergyType::Water]);
    assert!(!trainer_offered(
        &game.get_state_clone(),
        CardId::B1217FlamePatch
    ));
}

#[test]
fn flame_patch_not_offered_with_non_fire_active() {
    // Benched Torchic is Fire, but only the Active counts.
    let game = flame_patch_game(pc(CardId::A1087Froakie), vec![EnergyType::Fire]);
    assert!(!trainer_offered(
        &game.get_state_clone(),
        CardId::B1217FlamePatch
    ));
}

#[test]
fn flame_patch_reattaches_the_fire_discarded_by_mega_burning() {
    let mut game = flame_patch_game(
        pc(CardId::B1036MegaBlazikenEx).with_energy(vec![EnergyType::Fire, EnergyType::Fire]),
        vec![],
    );
    assert!(!trainer_offered(
        &game.get_state_clone(),
        CardId::B1217FlamePatch
    ));
    apply(&mut game, 0, attack_action(CardId::B1036MegaBlazikenEx, 0));
    end_turn(&mut game);
    end_turn(&mut game);
    let mut state = game.get_state_clone();
    state.hands[0] = vec![get_card_by_enum(CardId::B1217FlamePatch)];
    game.set_state(state);
    assert!(trainer_offered(
        &game.get_state_clone(),
        CardId::B1217FlamePatch
    ));
}

// ------------------------------------------------------------------------------------------------
// B1 044 Heatmor / B3 024 Castform Sunny Form
// ------------------------------------------------------------------------------------------------

#[test]
fn heatmor_tongue_whip_30_to_one_benched_only() {
    // Benched Bulbasaur is weak to [R]: exactly 30 (no Weakness on the Bench).
    let mut game = get_test_game_with_board(
        vec![pc(CardId::B1044Heatmor).with_energy(vec![EnergyType::Fire])],
        vec![
            pc(CardId::A1001Bulbasaur),
            pc(CardId::A1001Bulbasaur),
            pc(CardId::A1211Snorlax),
        ],
    );
    assert!(attack_offered(
        &game.get_state_clone(),
        CardId::B1044Heatmor,
        0
    ));
    apply(&mut game, 0, attack_action(CardId::B1044Heatmor, 0));
    assert_eq!(
        damage_choices(&game),
        vec![vec![(1, 1, 30)], vec![(1, 2, 30)]]
    );
    choose_damage_on(&mut game, 1, 1);
    assert_eq!(hp(&game, 1, 1), 40);
    assert_eq!(hp(&game, 1, 0), 70);
}

#[test]
fn heatmor_tongue_whip_without_bench_does_nothing() {
    let mut game = get_test_game_with_board(
        vec![pc(CardId::B1044Heatmor).with_energy(vec![EnergyType::Fire])],
        vec![pc(CardId::A1001Bulbasaur)],
    );
    apply(&mut game, 0, attack_action(CardId::B1044Heatmor, 0));
    assert!(damage_choices(&game).is_empty());
    assert_eq!(hp(&game, 1, 0), 70);
}

#[test]
fn castform_sunny_scorching_30_no_burn_without_stadium() {
    let mut game = get_test_game_with_board(
        vec![pc(CardId::B3024CastformSunnyForm).with_energy(vec![EnergyType::Fire])],
        vec![pc(CardId::A1211Snorlax)],
    );
    assert!(game.get_state_clone().active_stadium.is_none());
    apply(
        &mut game,
        0,
        attack_action(CardId::B3024CastformSunnyForm, 0),
    );
    let state = game.get_state_clone();
    assert_eq!(state.get_remaining_hp(1, 0), 120);
    assert!(!state.get_active(1).is_burned());
}

#[test]
fn castform_sunny_scorching_burns_with_either_players_stadium() {
    for owner in [0, 1] {
        let mut game = hiking_trail_game(
            vec![pc(CardId::B3024CastformSunnyForm).with_energy(vec![EnergyType::Fire])],
            vec![pc(CardId::A1211Snorlax)],
            owner,
        );
        apply(
            &mut game,
            0,
            attack_action(CardId::B3024CastformSunnyForm, 0),
        );
        let state = game.get_state_clone();
        assert_eq!(state.get_remaining_hp(1, 0), 120);
        assert!(
            state.get_active(1).is_burned(),
            "stadium owned by player {owner}"
        );
    }
}

// ------------------------------------------------------------------------------------------------
// A1 087 Froakie
// ------------------------------------------------------------------------------------------------

#[test]
fn froakie_flop_does_10_for_any_energy() {
    let mut game = get_test_game_with_board(
        vec![pc(CardId::A1087Froakie).with_energy(vec![EnergyType::Grass])],
        vec![pc(CardId::A1211Snorlax)],
    );
    assert!(attack_offered(
        &game.get_state_clone(),
        CardId::A1087Froakie,
        0
    ));
    apply(&mut game, 0, attack_action(CardId::A1087Froakie, 0));
    assert_eq!(hp(&game, 1, 0), 140);

    let state = get_test_game_with_board(
        vec![pc(CardId::A1087Froakie)],
        vec![pc(CardId::A1211Snorlax)],
    )
    .get_state_clone();
    assert!(
        !attack_offered(&state, CardId::A1087Froakie, 0),
        "needs 1 Energy"
    );
}
