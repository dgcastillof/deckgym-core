//! Audit of the cards in the two meta decks that the simulator mis-estimates:
//! - Mega Lucario ex (B3 081) + Lucario (A2 092) with Riolu, Hitmonlee, Korrina, Arena of
//!   Antiquity and Lucky Ice Pop (overestimated vs. real winrates).
//! - Dragonair (B4 117) + Mega Rayquaza ex (B4 120) with Dratini, Gouging Fire, Rainbow Cave,
//!   Professor Sada and Ancient Booster Energy Capsule (underestimated).
//!
//! Each test builds the smallest board that exercises one clause of the card text.
use deckgym::{
    actions::{Action, SimpleAction},
    card_ids::CardId,
    database::get_card_by_enum,
    models::{EnergyType, PlayedCard},
    state::{GameOutcome, State},
    test_support::{attack_action, get_test_game_with_board, trainer_from_id},
    Game,
};

use EnergyType::{Fighting as F, Fire as R, Grass as G, Lightning as L, Water as W};

// ---------------------------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------------------------

fn apply(game: &mut Game<'static>, actor: usize, action: SimpleAction) {
    game.apply_action(&Action {
        actor,
        action,
        is_stack: false,
    });
}

fn pc(id: CardId) -> PlayedCard {
    PlayedCard::from_id(id)
}

/// A Pokémon with an artificially large HP so damage can be read off without a Knock Out.
fn big(id: CardId, hp: u32) -> PlayedCard {
    PlayedCard::new(get_card_by_enum(id), 0, hp, vec![], false, vec![])
}

fn offered(state: &State, pred: impl Fn(&SimpleAction) -> bool) -> bool {
    let (actor, actions) = state.generate_possible_actions();
    actor == state.current_player && actions.iter().any(|a| pred(&a.action))
}

fn attack_offered(state: &State) -> bool {
    offered(state, |a| matches!(a, SimpleAction::Attack(_)))
}

fn ability_offered(state: &State, in_play_idx: usize) -> bool {
    offered(
        state,
        |a| matches!(a, SimpleAction::UseAbility { in_play_idx: i } if *i == in_play_idx),
    )
}

fn trainer_offered(state: &State, id: CardId) -> bool {
    let tc = trainer_from_id(id);
    offered(
        state,
        |a| matches!(a, SimpleAction::Play { trainer_card } if *trainer_card == tc),
    )
}

fn stadium_offered(state: &State) -> bool {
    offered(state, |a| matches!(a, SimpleAction::UseStadium))
}

fn play(game: &mut Game<'static>, actor: usize, id: CardId) {
    apply(
        game,
        actor,
        SimpleAction::Play {
            trainer_card: trainer_from_id(id),
        },
    );
}

/// Damage the active of `player` took from `f`.
fn damage_taken(
    game: &mut Game<'static>,
    player: usize,
    f: impl FnOnce(&mut Game<'static>),
) -> u32 {
    let before = game.get_state_clone().get_remaining_hp(player, 0);
    f(game);
    before - game.get_state_clone().get_remaining_hp(player, 0)
}

/// Player 0 uses the first attack of `attacker` and returns the damage the opponent's Active took.
fn p0_attack(game: &mut Game<'static>, attacker: CardId) -> u32 {
    damage_taken(game, 1, |g| apply(g, 0, attack_action(attacker, 0)))
}

/// `actor` ends the turn and the next player draws.
fn end_turn(game: &mut Game<'static>, actor: usize) {
    let mut state = game.get_state_clone();
    state.move_generation_stack.clear();
    game.set_state(state);
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

fn with_stadium(game: &mut Game<'static>, id: CardId) {
    let mut state = game.get_state_clone();
    state.active_stadium = Some(get_card_by_enum(id));
    game.set_state(state);
}

fn with_hand(game: &mut Game<'static>, player: usize, cards: Vec<CardId>) {
    let mut state = game.get_state_clone();
    state.hands[player] = cards.into_iter().map(get_card_by_enum).collect();
    game.set_state(state);
}

fn with_discard_energy(game: &mut Game<'static>, player: usize, energies: Vec<EnergyType>) {
    let mut state = game.get_state_clone();
    state.discard_energies[player] = energies;
    game.set_state(state);
}

fn mega_lucario(energy: Vec<EnergyType>) -> PlayedCard {
    pc(CardId::B3081MegaLucarioEx).with_energy(energy)
}

/// Non-ex, not weak to [F] (Bulbasaur is weak to [R]).
fn plain_target() -> PlayedCard {
    big(CardId::A1001Bulbasaur, 500)
}

/// Pokémon ex, not weak to [F] (Venusaur ex is weak to [R]).
fn ex_target() -> PlayedCard {
    big(CardId::A1004VenusaurEx, 500)
}

/// Pokémon ex weak to [F].
fn ex_weak_to_fighting() -> PlayedCard {
    big(CardId::A1096PikachuEx, 500)
}

// =============================================================================================
// Mega Lucario ex (B3 081) — Fighting Pulse [F][F] 90: "If this Pokémon has at least 1 extra [F]
// Energy attached, this attack does 50 more damage."
// =============================================================================================

#[test]
fn test_fighting_pulse_90_with_exactly_ff() {
    let mut game = get_test_game_with_board(vec![mega_lucario(vec![F, F])], vec![plain_target()]);
    assert_eq!(p0_attack(&mut game, CardId::B3081MegaLucarioEx), 90);
}

#[test]
fn test_fighting_pulse_140_with_fff() {
    let mut game =
        get_test_game_with_board(vec![mega_lucario(vec![F, F, F])], vec![plain_target()]);
    assert_eq!(p0_attack(&mut game, CardId::B3081MegaLucarioEx), 140);
}

#[test]
fn test_fighting_pulse_extra_non_fighting_energy_does_not_count() {
    let mut game =
        get_test_game_with_board(vec![mega_lucario(vec![F, F, W, G])], vec![plain_target()]);
    assert_eq!(
        p0_attack(&mut game, CardId::B3081MegaLucarioEx),
        90,
        "extra energy must be [F]"
    );
}

#[test]
fn test_fighting_pulse_needs_two_fighting() {
    let game = get_test_game_with_board(vec![mega_lucario(vec![F, W, W])], vec![plain_target()]);
    assert!(!attack_offered(&game.get_state_clone()));
    let game = get_test_game_with_board(vec![mega_lucario(vec![F, F])], vec![plain_target()]);
    assert!(attack_offered(&game.get_state_clone()));
}

#[test]
fn test_knocking_out_mega_lucario_ex_gives_3_points_and_lucario_1() {
    // Riolu's Fighting Fist: 10 + 30 vs ex = 40.
    let mut game = get_test_game_with_board(
        vec![pc(CardId::B3079Riolu).with_energy(vec![F])],
        vec![
            pc(CardId::B3081MegaLucarioEx).with_remaining_hp(10),
            pc(CardId::A2092Lucario),
        ],
    );
    apply(&mut game, 0, attack_action(CardId::B3079Riolu, 0));
    let state = game.get_state_clone();
    assert_eq!(state.points[0], 3);
    assert_eq!(state.winner, Some(GameOutcome::Win(0)));

    let mut game = get_test_game_with_board(
        vec![pc(CardId::B3079Riolu).with_energy(vec![F])],
        vec![
            pc(CardId::A2092Lucario).with_remaining_hp(10),
            pc(CardId::A2092Lucario),
        ],
    );
    apply(&mut game, 0, attack_action(CardId::B3079Riolu, 0));
    assert_eq!(game.get_state_clone().points[0], 1);
}

#[test]
fn test_riolu_b3_evolves_into_mega_lucario_ex_and_lucario_a2() {
    let mut game = get_test_game_with_board(
        vec![pc(CardId::B3079Riolu), pc(CardId::B3079Riolu)],
        vec![plain_target()],
    );
    with_hand(
        &mut game,
        0,
        vec![CardId::B3081MegaLucarioEx, CardId::A2092Lucario],
    );
    let state = game.get_state_clone();
    for id in [CardId::B3081MegaLucarioEx, CardId::A2092Lucario] {
        let card = get_card_by_enum(id);
        for idx in [0, 1] {
            assert!(
                offered(&state, |a| matches!(a,
                    SimpleAction::Evolve { evolution, in_play_idx, .. }
                        if *evolution == card && *in_play_idx == idx)),
                "{id:?} should be offered on Riolu at {idx}"
            );
        }
    }
}

// =============================================================================================
// Lucario (A2 092) — Fighting Coach: "Attacks used by your [F] Pokémon do +20 damage to your
// opponent's Active Pokémon." Submarine Blow [F][F] 40.
// =============================================================================================

#[test]
fn test_submarine_blow_40() {
    let mut game = get_test_game_with_board(
        vec![big(CardId::A2092Lucario, 100).with_energy(vec![F, F])],
        vec![plain_target()],
    );
    // 40 + 20 (its own Fighting Coach).
    assert_eq!(p0_attack(&mut game, CardId::A2092Lucario), 60);
}

#[test]
fn test_fighting_coach_one_benched_lucario_adds_20() {
    let mut game = get_test_game_with_board(
        vec![mega_lucario(vec![F, F]), pc(CardId::A2092Lucario)],
        vec![plain_target()],
    );
    assert_eq!(p0_attack(&mut game, CardId::B3081MegaLucarioEx), 110);
}

#[test]
fn test_fighting_coach_stacks_per_lucario() {
    let mut game = get_test_game_with_board(
        vec![
            mega_lucario(vec![F, F]),
            pc(CardId::A2092Lucario),
            pc(CardId::A2092Lucario),
        ],
        vec![plain_target()],
    );
    assert_eq!(p0_attack(&mut game, CardId::B3081MegaLucarioEx), 130);
}

#[test]
fn test_fighting_coach_of_opponent_does_not_boost_me() {
    let mut game = get_test_game_with_board(
        vec![mega_lucario(vec![F, F])],
        vec![plain_target(), pc(CardId::A2092Lucario)],
    );
    assert_eq!(p0_attack(&mut game, CardId::B3081MegaLucarioEx), 90);
}

#[test]
fn test_fighting_coach_does_not_boost_non_fighting_attacker() {
    let mut game = get_test_game_with_board(
        vec![
            pc(CardId::A1001Bulbasaur).with_energy(vec![G, G]),
            pc(CardId::A2092Lucario),
        ],
        vec![plain_target()],
    );
    assert_eq!(p0_attack(&mut game, CardId::A1001Bulbasaur), 40);
}

// =============================================================================================
// Hitmonlee (A1 154) — Stretch Kick [F]: "This attack does 30 damage to 1 of your opponent's
// Benched Pokémon." Fighting Coach / Arena / Korrina only add to the opponent's *Active*.
// =============================================================================================

fn stretch_kick_targets(game: &mut Game<'static>) -> Vec<(u32, usize, usize)> {
    apply(game, 0, attack_action(CardId::A1154Hitmonlee, 0));
    let (_, actions) = game.get_state_clone().generate_possible_actions();
    actions
        .iter()
        .filter_map(|a| match &a.action {
            SimpleAction::ApplyDamage { targets, .. } => Some(targets[0]),
            _ => None,
        })
        .collect()
}

#[test]
fn test_stretch_kick_30_to_bench_only_without_any_bonus() {
    let mut game = get_test_game_with_board(
        vec![
            pc(CardId::A1154Hitmonlee).with_energy(vec![F]),
            pc(CardId::A2092Lucario),
            pc(CardId::A2092Lucario),
        ],
        vec![ex_weak_to_fighting(), ex_weak_to_fighting()],
    );
    with_stadium(&mut game, CardId::B3154ArenaofAntiquity);
    with_hand(&mut game, 0, vec![CardId::B3149Korrina]);
    play(&mut game, 0, CardId::B3149Korrina);

    let targets = stretch_kick_targets(&mut game);
    assert_eq!(
        targets,
        vec![(30, 1, 1)],
        "only the Benched Pokémon is a target"
    );
    let (_, actions) = game.get_state_clone().generate_possible_actions();
    game.apply_action(&actions[0]);
    let state = game.get_state_clone();
    assert_eq!(
        state.get_remaining_hp(1, 1),
        470,
        "exactly 30: no Fighting Coach, Arena, Korrina nor Weakness on the Bench"
    );
    assert_eq!(state.get_remaining_hp(1, 0), 500, "the Active is not hit");
}

#[test]
fn test_stretch_kick_with_no_bench_is_usable_and_does_nothing() {
    let mut game = get_test_game_with_board(
        vec![pc(CardId::A1154Hitmonlee).with_energy(vec![F])],
        vec![plain_target()],
    );
    assert!(attack_offered(&game.get_state_clone()));
    let targets = stretch_kick_targets(&mut game);
    assert!(targets.is_empty());
    assert_eq!(game.get_state_clone().get_remaining_hp(1, 0), 500);
}

// =============================================================================================
// Riolu (B3 079) — Fighting Fist [F] 10: "If your opponent's Active Pokémon is a Pokémon ex,
// this attack does 30 more damage."
// =============================================================================================

#[test]
fn test_fighting_fist_10_vs_non_ex_40_vs_ex() {
    let riolu = || pc(CardId::B3079Riolu).with_energy(vec![F]);
    let mut game = get_test_game_with_board(vec![riolu()], vec![plain_target()]);
    assert_eq!(p0_attack(&mut game, CardId::B3079Riolu), 10);
    let mut game = get_test_game_with_board(vec![riolu()], vec![ex_target()]);
    assert_eq!(p0_attack(&mut game, CardId::B3079Riolu), 40);
    // With a benched Lucario: +20.
    let mut game =
        get_test_game_with_board(vec![riolu(), pc(CardId::A2092Lucario)], vec![ex_target()]);
    assert_eq!(p0_attack(&mut game, CardId::B3079Riolu), 60);
}

// =============================================================================================
// Arena of Antiquity (B3 154): "Attacks used by each [F] Pokémon in play (both yours and your
// opponent's) do +20 damage to the opponent's Active Pokémon ex."
// =============================================================================================

#[test]
fn test_arena_plus_20_only_vs_active_ex() {
    let mut game = get_test_game_with_board(vec![mega_lucario(vec![F, F])], vec![ex_target()]);
    with_stadium(&mut game, CardId::B3154ArenaofAntiquity);
    assert_eq!(p0_attack(&mut game, CardId::B3081MegaLucarioEx), 110);

    let mut game = get_test_game_with_board(vec![mega_lucario(vec![F, F])], vec![plain_target()]);
    with_stadium(&mut game, CardId::B3154ArenaofAntiquity);
    assert_eq!(p0_attack(&mut game, CardId::B3081MegaLucarioEx), 90);
}

#[test]
fn test_arena_does_not_boost_non_fighting_vs_ex() {
    let mut game = get_test_game_with_board(
        vec![pc(CardId::A1001Bulbasaur).with_energy(vec![G, G])],
        vec![ex_target()],
    );
    with_stadium(&mut game, CardId::B3154ArenaofAntiquity);
    assert_eq!(p0_attack(&mut game, CardId::A1001Bulbasaur), 40);
}

#[test]
fn test_arena_boosts_the_opponents_fighting_pokemon_too() {
    // Player 1's Riolu attacks player 0's ex: 10 + 30 (Fighting Fist vs ex) + 20 (Arena).
    let mut game = get_test_game_with_board(
        vec![ex_target()],
        vec![pc(CardId::B3079Riolu).with_energy(vec![F])],
    );
    with_stadium(&mut game, CardId::B3154ArenaofAntiquity);
    let mut state = game.get_state_clone();
    state.current_player = 1;
    state.turn_count = 4;
    game.set_state(state);
    let dealt = damage_taken(&mut game, 0, |g| {
        apply(g, 1, attack_action(CardId::B3079Riolu, 0))
    });
    assert_eq!(dealt, 60);
}

// =============================================================================================
// Korrina (B3 149): "During this turn, attacks used by your [F] Pokémon do +30 damage to your
// opponent's Active Pokémon ex."
// =============================================================================================

#[test]
fn test_korrina_plus_30_vs_active_ex_this_turn_only() {
    let mut game = get_test_game_with_board(
        vec![mega_lucario(vec![F, F])],
        vec![ex_target().with_energy(vec![G, G, G])],
    );
    with_hand(&mut game, 0, vec![CardId::B3149Korrina]);
    assert!(trainer_offered(
        &game.get_state_clone(),
        CardId::B3149Korrina
    ));
    play(&mut game, 0, CardId::B3149Korrina);
    assert_eq!(p0_attack(&mut game, CardId::B3081MegaLucarioEx), 120);

    // Next own turn: no Korrina bonus any more.
    end_turn(&mut game, 0);
    end_turn(&mut game, 1);
    assert_eq!(game.get_state_clone().current_player, 0);
    assert_eq!(p0_attack(&mut game, CardId::B3081MegaLucarioEx), 90);
}

#[test]
fn test_korrina_does_not_boost_vs_non_ex_nor_non_fighting() {
    let mut game = get_test_game_with_board(vec![mega_lucario(vec![F, F])], vec![plain_target()]);
    with_hand(&mut game, 0, vec![CardId::B3149Korrina]);
    play(&mut game, 0, CardId::B3149Korrina);
    assert_eq!(p0_attack(&mut game, CardId::B3081MegaLucarioEx), 90);

    let mut game = get_test_game_with_board(
        vec![pc(CardId::A1001Bulbasaur).with_energy(vec![G, G])],
        vec![ex_target()],
    );
    with_hand(&mut game, 0, vec![CardId::B3149Korrina]);
    play(&mut game, 0, CardId::B3149Korrina);
    assert_eq!(p0_attack(&mut game, CardId::A1001Bulbasaur), 40);
}

#[test]
fn test_korrina_does_not_boost_the_opponents_fighting_attack_next_turn() {
    // Player 0 plays Korrina, ends the turn; player 1's Riolu hits player 0's ex for 10 + 30 only.
    let mut game = get_test_game_with_board(
        vec![ex_target()],
        vec![pc(CardId::B3079Riolu).with_energy(vec![F])],
    );
    with_hand(&mut game, 0, vec![CardId::B3149Korrina]);
    play(&mut game, 0, CardId::B3149Korrina);
    end_turn(&mut game, 0);
    let dealt = damage_taken(&mut game, 0, |g| {
        apply(g, 1, attack_action(CardId::B3079Riolu, 0))
    });
    assert_eq!(dealt, 40);
}

// =============================================================================================
// Combined: Mega Lucario ex with [F][F][F], 1 Lucario on the Bench, Arena of Antiquity and
// Korrina, against an Active ex weak to [F]:
//   140 (Fighting Pulse + extra [F]) + 20 (Fighting Coach) + 20 (Arena) + 30 (Korrina) = 210,
//   + 20 Weakness (flat in Pocket) = 230.
// Engine order (src/hooks/core.rs modify_damage): attacker-side bonuses summed first, then
// Weakness (+20 flat), then defender-side reductions; with a flat +20 the order doesn't matter.
// =============================================================================================

fn full_combo_game(target: PlayedCard) -> Game<'static> {
    let mut game = get_test_game_with_board(
        vec![mega_lucario(vec![F, F, F]), pc(CardId::A2092Lucario)],
        vec![target, pc(CardId::A1001Bulbasaur)],
    );
    with_stadium(&mut game, CardId::B3154ArenaofAntiquity);
    with_hand(&mut game, 0, vec![CardId::B3149Korrina]);
    play(&mut game, 0, CardId::B3149Korrina);
    game
}

#[test]
fn test_full_lucario_combo_vs_ex_weak_to_fighting_is_230() {
    let mut game = full_combo_game(ex_weak_to_fighting());
    assert_eq!(p0_attack(&mut game, CardId::B3081MegaLucarioEx), 230);
}

#[test]
fn test_full_lucario_combo_vs_ex_without_weakness_is_210() {
    let mut game = full_combo_game(big(CardId::B4120MegaRayquazaEx, 500));
    assert_eq!(p0_attack(&mut game, CardId::B3081MegaLucarioEx), 210);
}

#[test]
fn test_mega_lucario_fff_lucario_arena_one_shots_mega_rayquaza_ex() {
    // 140 + 20 + 20 = 180 = Mega Rayquaza ex's HP, without Korrina: 3 points.
    let mut game = get_test_game_with_board(
        vec![mega_lucario(vec![F, F, F]), pc(CardId::A2092Lucario)],
        vec![pc(CardId::B4120MegaRayquazaEx), pc(CardId::B2b051Dratini)],
    );
    with_stadium(&mut game, CardId::B3154ArenaofAntiquity);
    apply(&mut game, 0, attack_action(CardId::B3081MegaLucarioEx, 0));
    let state = game.get_state_clone();
    assert_eq!(state.points[0], 3);
    assert_eq!(state.winner, Some(GameOutcome::Win(0)));
}

// =============================================================================================
// Lucky Ice Pop (B2 145): "Heal 20 damage from your Active Pokémon. If you healed any damage in
// this way, flip a coin. If heads, put this Lucky Ice Pop into your hand instead of the discard
// pile."
// =============================================================================================

#[test]
fn test_lucky_ice_pop_not_offered_when_active_is_undamaged() {
    let mut game = get_test_game_with_board(
        vec![
            pc(CardId::A1001Bulbasaur),
            pc(CardId::A1001Bulbasaur).with_damage(30),
        ],
        vec![plain_target()],
    );
    with_hand(&mut game, 0, vec![CardId::B2145LuckyIcePop]);
    assert!(!trainer_offered(
        &game.get_state_clone(),
        CardId::B2145LuckyIcePop
    ));
}

#[test]
fn test_lucky_ice_pop_heals_only_active_and_partially() {
    let mut heads = 0;
    for seed in 0..60u64 {
        let mut game = deckgym::test_support::get_initialized_game_with_board(
            seed,
            0,
            3,
            vec![
                pc(CardId::A1001Bulbasaur).with_damage(10),
                pc(CardId::A1001Bulbasaur).with_damage(30),
            ],
            vec![plain_target()],
        );
        with_hand(&mut game, 0, vec![CardId::B2145LuckyIcePop]);
        assert!(trainer_offered(
            &game.get_state_clone(),
            CardId::B2145LuckyIcePop
        ));
        play(&mut game, 0, CardId::B2145LuckyIcePop);
        let state = game.get_state_clone();
        assert_eq!(state.get_remaining_hp(0, 0), 70, "heals the 10 damage");
        assert_eq!(state.get_remaining_hp(0, 1), 40, "bench untouched");
        let card = get_card_by_enum(CardId::B2145LuckyIcePop);
        let in_hand = state.hands[0].contains(&card);
        let in_discard = state.discard_piles[0].contains(&card);
        assert!(in_hand ^ in_discard);
        if in_hand {
            heads += 1;
        }
    }
    assert!(heads > 10 && heads < 50, "coin flip, got {heads}/60 heads");
}

// =============================================================================================
// Mega Rayquaza ex (B4 120) — Mega Burst [R][L] 50x: "Discard all [R] and [L] Energy from this
// Pokémon, and this attack does 50 damage for each Energy you discarded in this way."
// =============================================================================================

#[test]
fn test_mega_burst_rlrl_is_200_and_keeps_other_energy() {
    let mut game = get_test_game_with_board(
        vec![pc(CardId::B4120MegaRayquazaEx).with_energy(vec![R, L, W, R, L])],
        vec![plain_target()],
    );
    assert_eq!(p0_attack(&mut game, CardId::B4120MegaRayquazaEx), 200);
    let state = game.get_state_clone();
    assert_eq!(state.get_active(0).attached_energy, vec![W]);
    let mut discarded = state.discard_energies[0].clone();
    discarded.sort();
    assert_eq!(discarded, vec![R, R, L, L]);
}

#[test]
fn test_mega_burst_needs_fire_and_lightning() {
    for energy in [vec![R, R, R], vec![L, L, W], vec![R, W]] {
        let game = get_test_game_with_board(
            vec![pc(CardId::B4120MegaRayquazaEx).with_energy(energy.clone())],
            vec![plain_target()],
        );
        assert!(!attack_offered(&game.get_state_clone()), "{energy:?}");
    }
    let game = get_test_game_with_board(
        vec![pc(CardId::B4120MegaRayquazaEx).with_energy(vec![L, R])],
        vec![plain_target()],
    );
    assert!(attack_offered(&game.get_state_clone()));
}

#[test]
fn test_mega_burst_rl_100_vs_lucario_no_bonus() {
    // Mega Lucario ex is weak to [P], Mega Rayquaza ex is [N]: plain 100.
    let mut game = get_test_game_with_board(
        vec![pc(CardId::B4120MegaRayquazaEx).with_energy(vec![R, L])],
        vec![big(CardId::B3081MegaLucarioEx, 500)],
    );
    assert_eq!(p0_attack(&mut game, CardId::B4120MegaRayquazaEx), 100);
}

// =============================================================================================
// Dratini (B2b 051) — Tail Strike [C] 10, +20 on heads. Dragonair (B4 117) — Draconic Whip
// [C][C] 40; Dragon's Blessing: "Once during your turn, if this Pokémon is on your Bench, you may
// attach an Energy from your discard pile to your Active [N] Pokémon."
// =============================================================================================

#[test]
fn test_dratini_tail_strike_10_or_30() {
    let mut seen = std::collections::BTreeSet::new();
    for seed in 0..40u64 {
        let mut game = deckgym::test_support::get_initialized_game_with_board(
            seed,
            0,
            3,
            vec![pc(CardId::B2b051Dratini).with_energy(vec![R])],
            vec![plain_target()],
        );
        seen.insert(p0_attack(&mut game, CardId::B2b051Dratini));
    }
    assert_eq!(seen.into_iter().collect::<Vec<_>>(), vec![10, 30]);
}

#[test]
fn test_dratini_evolves_into_dragonair_b4() {
    let mut game = get_test_game_with_board(vec![pc(CardId::B2b051Dratini)], vec![plain_target()]);
    with_hand(&mut game, 0, vec![CardId::B4117Dragonair]);
    let card = get_card_by_enum(CardId::B4117Dragonair);
    assert!(offered(&game.get_state_clone(), |a| matches!(a,
        SimpleAction::Evolve { evolution, in_play_idx: 0, .. } if *evolution == card)));
}

#[test]
fn test_draconic_whip_40() {
    let mut game = get_test_game_with_board(
        vec![pc(CardId::B4117Dragonair).with_energy(vec![R, L])],
        vec![plain_target()],
    );
    assert_eq!(p0_attack(&mut game, CardId::B4117Dragonair), 40);
}

fn dragons_blessing(game: &mut Game<'static>, dragonair_idx: usize, energy: EnergyType) {
    apply(
        game,
        0,
        SimpleAction::UseAbility {
            in_play_idx: dragonair_idx,
        },
    );
    let (actor, actions) = game.get_state_clone().generate_possible_actions();
    assert_eq!(actor, 0);
    let choice = actions
        .into_iter()
        .find(|a| {
            matches!(a.action, SimpleAction::AttachTypedFromDiscard { in_play_idx: 0, energy_type, count: 1 }
                if energy_type == energy)
        })
        .expect("attach choice for the requested type");
    game.apply_action(&choice);
}

#[test]
fn test_dragons_blessing_attaches_to_active_mega_rayquaza_once_per_dragonair_per_turn() {
    let mut game = get_test_game_with_board(
        vec![
            pc(CardId::B4120MegaRayquazaEx),
            pc(CardId::B4117Dragonair),
            pc(CardId::B4117Dragonair),
        ],
        vec![plain_target()],
    );
    with_discard_energy(&mut game, 0, vec![R, L, L, R]);
    assert!(ability_offered(&game.get_state_clone(), 1));
    assert!(ability_offered(&game.get_state_clone(), 2));

    dragons_blessing(&mut game, 1, R);
    let state = game.get_state_clone();
    assert!(!ability_offered(&state, 1), "once per turn");
    assert!(
        ability_offered(&state, 2),
        "the other Dragonair has its own use"
    );
    dragons_blessing(&mut game, 2, L);
    let state = game.get_state_clone();
    assert_eq!(state.get_active(0).attached_energy, vec![R, L]);
    let mut discard = state.discard_energies[0].clone();
    discard.sort();
    assert_eq!(discard, vec![R, L]);
    assert!(!ability_offered(&state, 2));

    // The two energies make Mega Burst usable right away: 100.
    assert!(attack_offered(&state));

    // Available again next own turn.
    end_turn(&mut game, 0);
    end_turn(&mut game, 1);
    let state = game.get_state_clone();
    assert!(ability_offered(&state, 1));
    assert!(ability_offered(&state, 2));
}

#[test]
fn test_dragons_blessing_works_on_gouging_fire_which_is_dragon() {
    let mut game = get_test_game_with_board(
        vec![pc(CardId::B3a054GougingFire), pc(CardId::B4117Dragonair)],
        vec![plain_target()],
    );
    with_discard_energy(&mut game, 0, vec![L]);
    dragons_blessing(&mut game, 1, L);
    assert_eq!(
        game.get_state_clone().get_active(0).attached_energy,
        vec![L]
    );
}

#[test]
fn test_dragons_blessing_not_offered_from_active_or_with_empty_discard() {
    let mut game = get_test_game_with_board(
        vec![pc(CardId::B4117Dragonair), pc(CardId::B4120MegaRayquazaEx)],
        vec![plain_target()],
    );
    with_discard_energy(&mut game, 0, vec![R]);
    assert!(
        !ability_offered(&game.get_state_clone(), 0),
        "not while Active"
    );

    let mut game = get_test_game_with_board(
        vec![pc(CardId::B4120MegaRayquazaEx), pc(CardId::B4117Dragonair)],
        vec![plain_target()],
    );
    with_discard_energy(&mut game, 0, vec![]);
    assert!(
        !ability_offered(&game.get_state_clone(), 1),
        "no Energy in discard"
    );
}

#[test]
fn test_dragons_blessing_not_offered_when_active_is_not_dragon() {
    let mut game = get_test_game_with_board(
        vec![pc(CardId::A1001Bulbasaur), pc(CardId::B4117Dragonair)],
        vec![plain_target()],
    );
    with_discard_energy(&mut game, 0, vec![R, L]);
    assert!(
        !ability_offered(&game.get_state_clone(), 1),
        "text: 'attach an Energy ... to your Active [N] Pokémon'"
    );
}

// =============================================================================================
// Gouging Fire (B3a 054) — Scorching Interruption [R][L][C] 100: "Discard 2 Energy from this
// Pokémon. During your opponent's next turn, this Pokémon takes -30 damage from attacks."
// =============================================================================================

#[test]
fn test_scorching_interruption_needs_r_and_l() {
    let game = get_test_game_with_board(
        vec![pc(CardId::B3a054GougingFire).with_energy(vec![R, R, R])],
        vec![plain_target()],
    );
    assert!(!attack_offered(&game.get_state_clone()));
    let game = get_test_game_with_board(
        vec![pc(CardId::B3a054GougingFire).with_energy(vec![R, L, W])],
        vec![plain_target()],
    );
    assert!(attack_offered(&game.get_state_clone()));
}

#[test]
fn test_scorching_interruption_100_discards_exactly_2_and_minus_30_only_next_opponent_turn() {
    let mut game = get_test_game_with_board(
        vec![pc(CardId::B3a054GougingFire).with_energy(vec![R, L, R, L])],
        vec![big(CardId::A1033Charmander, 500).with_energy(vec![R, R, R])],
    );
    assert_eq!(p0_attack(&mut game, CardId::B3a054GougingFire), 100);
    let state = game.get_state_clone();
    assert_eq!(state.get_active(0).attached_energy.len(), 2);
    assert_eq!(state.discard_energies[0].len(), 2);

    // Opponent's next turn: Ember (30) is reduced to 0.
    end_turn(&mut game, 0);
    let dealt = damage_taken(&mut game, 0, |g| {
        apply(g, 1, attack_action(CardId::A1033Charmander, 0))
    });
    assert_eq!(dealt, 0);

    // The turn after (opponent's following turn): full 30 again.
    end_turn(&mut game, 1);
    end_turn(&mut game, 0);
    let dealt = damage_taken(&mut game, 0, |g| {
        apply(g, 1, attack_action(CardId::A1033Charmander, 0))
    });
    assert_eq!(dealt, 30);
}

// =============================================================================================
// Rainbow Cave (B4 155): "Once during each player's turn, that player may discard the Energy that
// has been generated in their Energy Zone. If they do, the next Energy is produced."
// =============================================================================================

fn rainbow_cave_game() -> Game<'static> {
    let mut game = get_test_game_with_board(
        vec![pc(CardId::B4120MegaRayquazaEx), pc(CardId::B4117Dragonair)],
        vec![plain_target(), pc(CardId::A1001Bulbasaur)],
    );
    with_stadium(&mut game, CardId::B4155RainbowCave);
    let mut state = game.get_state_clone();
    state.energy_zone[0].current = Some(W);
    state.energy_zone[0].next = Some(R);
    state.discard_energies[0].clear();
    game.set_state(state);
    game
}

#[test]
fn test_rainbow_cave_offered_once_per_turn_to_each_player() {
    let mut game = rainbow_cave_game();
    assert!(stadium_offered(&game.get_state_clone()));
    apply(&mut game, 0, SimpleAction::UseStadium);
    let state = game.get_state_clone();
    assert_eq!(
        state.energy_zone[0].current,
        Some(R),
        "the next Energy is produced"
    );
    assert_eq!(state.discard_energies[0], vec![W]);
    assert!(!stadium_offered(&state), "once per turn");

    // Opponent's turn: they may use it on their own generated Energy.
    end_turn(&mut game, 0);
    let state = game.get_state_clone();
    assert_eq!(state.current_player, 1);
    assert!(state.energy_zone[1].current.is_some());
    assert!(
        stadium_offered(&state),
        "the opponent may use it during their turn"
    );

    // Back to player 0: available again.
    end_turn(&mut game, 1);
    assert!(stadium_offered(&game.get_state_clone()));
}

#[test]
fn test_rainbow_cave_not_offered_after_attaching_the_turn_energy() {
    let mut game = rainbow_cave_game();
    apply(
        &mut game,
        0,
        SimpleAction::Attach {
            attachments: vec![(1, W, 0)],
            is_turn_energy: true,
        },
    );
    assert!(!stadium_offered(&game.get_state_clone()));
}

#[test]
fn test_rainbow_cave_feeds_dragons_blessing_for_two_energies_in_one_turn() {
    let mut game = rainbow_cave_game();
    let mut state = game.get_state_clone();
    state.energy_zone[0].current = Some(R);
    state.energy_zone[0].next = Some(L);
    game.set_state(state);
    assert!(
        !ability_offered(&game.get_state_clone(), 1),
        "discard empty yet"
    );

    apply(&mut game, 0, SimpleAction::UseStadium);
    dragons_blessing(&mut game, 1, R);
    apply(
        &mut game,
        0,
        SimpleAction::Attach {
            attachments: vec![(1, L, 0)],
            is_turn_energy: true,
        },
    );
    let state = game.get_state_clone();
    assert_eq!(state.get_active(0).attached_energy, vec![R, L]);
    assert!(attack_offered(&state), "Mega Burst ready the same turn");
}

// =============================================================================================
// Professor Sada (B3a 072): "Attach 3 different types of Energy from your discard pile to your
// Ancient Pokémon in any way you like." Ancient Booster Energy Capsule (B3a 069): "The Ancient
// Pokémon this card is attached to gets +40 HP."
// =============================================================================================

#[test]
fn test_sada_not_offered_without_ancient_or_without_discard_energy() {
    // Mega Rayquaza ex and Dragonair are not Ancient.
    let mut game = get_test_game_with_board(
        vec![pc(CardId::B4120MegaRayquazaEx), pc(CardId::B4117Dragonair)],
        vec![plain_target()],
    );
    with_hand(&mut game, 0, vec![CardId::B3a072ProfessorSada]);
    with_discard_energy(&mut game, 0, vec![R, L, W]);
    assert!(!trainer_offered(
        &game.get_state_clone(),
        CardId::B3a072ProfessorSada
    ));

    let mut game = get_test_game_with_board(
        vec![
            pc(CardId::B4120MegaRayquazaEx),
            pc(CardId::B3a054GougingFire),
        ],
        vec![plain_target()],
    );
    with_hand(&mut game, 0, vec![CardId::B3a072ProfessorSada]);
    with_discard_energy(&mut game, 0, vec![]);
    assert!(!trainer_offered(
        &game.get_state_clone(),
        CardId::B3a072ProfessorSada
    ));
    with_discard_energy(&mut game, 0, vec![R]);
    assert!(trainer_offered(
        &game.get_state_clone(),
        CardId::B3a072ProfessorSada
    ));
}

#[test]
fn test_sada_attaches_one_of_each_type_only_to_ancient_pokemon() {
    let mut game = get_test_game_with_board(
        vec![
            pc(CardId::B4120MegaRayquazaEx),
            pc(CardId::B3a054GougingFire),
        ],
        vec![plain_target()],
    );
    with_hand(&mut game, 0, vec![CardId::B3a072ProfessorSada]);
    with_discard_energy(&mut game, 0, vec![R, L, R, L, W]);
    play(&mut game, 0, CardId::B3a072ProfessorSada);
    let (actor, actions) = game.get_state_clone().generate_possible_actions();
    assert_eq!(actor, 0);
    let assignments: Vec<Vec<(EnergyType, usize)>> = actions
        .iter()
        .filter_map(|a| match &a.action {
            SimpleAction::SadaAttach { assignments } => Some(assignments.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(assignments.len(), actions.len());
    assert_eq!(assignments.len(), 1, "one Ancient slot, exactly 3 types");
    let mut types: Vec<EnergyType> = assignments[0].iter().map(|(e, _)| *e).collect();
    types.sort();
    assert_eq!(types, vec![R, W, L]);
    assert!(assignments[0].iter().all(|(_, idx)| *idx == 1));

    game.apply_action(&actions[0]);
    let state = game.get_state_clone();
    let mut gf = state.in_play_pokemon[0][1]
        .as_ref()
        .unwrap()
        .attached_energy
        .clone();
    gf.sort();
    assert_eq!(gf, vec![R, W, L]);
    assert!(state.get_active(0).attached_energy.is_empty());
    let mut discard = state.discard_energies[0].clone();
    discard.sort();
    assert_eq!(discard, vec![R, L]);
}

#[test]
fn test_sada_with_two_types_in_discard_attaches_both() {
    // The Rayquaza deck only runs [R] and [L]: Sada can attach at most 2 (one of each).
    let mut game = get_test_game_with_board(
        vec![pc(CardId::B3a054GougingFire), pc(CardId::B3a054GougingFire)],
        vec![plain_target()],
    );
    with_hand(&mut game, 0, vec![CardId::B3a072ProfessorSada]);
    with_discard_energy(&mut game, 0, vec![R, R, L]);
    play(&mut game, 0, CardId::B3a072ProfessorSada);
    let (_, actions) = game.get_state_clone().generate_possible_actions();
    assert_eq!(actions.len(), 4, "2 types x 2 Ancient slots");
    let both_on_active = actions
        .iter()
        .find(|a| {
            matches!(&a.action, SimpleAction::SadaAttach { assignments }
            if assignments.iter().all(|(_, i)| *i == 0))
        })
        .expect("both on the Active")
        .clone();
    game.apply_action(&both_on_active);
    let state = game.get_state_clone();
    let mut active = state.get_active(0).attached_energy.clone();
    active.sort();
    assert_eq!(active, vec![R, L]);
    assert_eq!(state.discard_energies[0], vec![R]);
}

#[test]
fn test_ancient_booster_capsule_only_for_ancient_holder() {
    let capsule = || get_card_by_enum(CardId::B3a069AncientBoosterEnergyCapsule);
    let game = get_test_game_with_board(
        vec![
            pc(CardId::B3a054GougingFire).with_tool(capsule()),
            pc(CardId::B4120MegaRayquazaEx).with_tool(capsule()),
        ],
        vec![plain_target()],
    );
    let state = game.get_state_clone();
    assert_eq!(state.get_remaining_hp(0, 0), 150, "Gouging Fire 110 + 40");
    assert_eq!(
        state.get_remaining_hp(0, 1),
        180,
        "Mega Rayquaza ex is not Ancient"
    );
}

#[test]
fn test_ancient_booster_capsule_attachable_from_hand_to_gouging_fire() {
    let mut game = get_test_game_with_board(
        vec![
            pc(CardId::B4120MegaRayquazaEx),
            pc(CardId::B3a054GougingFire),
        ],
        vec![plain_target()],
    );
    with_hand(
        &mut game,
        0,
        vec![CardId::B3a069AncientBoosterEnergyCapsule],
    );
    play(&mut game, 0, CardId::B3a069AncientBoosterEnergyCapsule);
    let (_, actions) = game.get_state_clone().generate_possible_actions();
    let attach = actions
        .into_iter()
        .find(|a| matches!(a.action, SimpleAction::AttachTool { in_play_idx: 1, .. }))
        .expect("tool can go on the Benched Gouging Fire");
    game.apply_action(&attach);
    assert_eq!(game.get_state_clone().get_remaining_hp(0, 1), 150);
}
