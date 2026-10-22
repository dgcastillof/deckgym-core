// Jugador t<n>: cada jugada se puntua al final de mi turno (step 2.7, ver docs/search-design.md).
//
// e<n> cuenta acciones, no turnos: con e2, usar Water Shuriken y elegir el objetivo ya gastan las
// dos y esa hoja se evalua a mitad de turno, sin el ataque que todavia falta, mientras que "atacar
// ya" se evalua con el dano completo. El bot ataca, el turno termina y la habilidad se pierde
// (efecto horizonte). t<n> juega cada accion legal y completa el resto de mi turno con la busqueda
// que le quedaba a e<n> (greedy de un ply para t2) antes de evaluar, asi todas las candidatas se
// comparan en el mismo punto: cuando le toca al rival.

use log::LevelFilter;
use rand::rngs::StdRng;
use rand::Rng;
use std::fmt::Debug;

use crate::actions::Action;
use crate::{Deck, State};

use super::own_reply_player::{
    argmax_last, decide, expectiminimax_scores, forecast_outcomes, ValueFn,
};
use super::{Player, ValueFunction};

/// Tope de acciones al completar un turno, por si alguna politica entra en un ciclo
const MAX_COMPLETION_ACTIONS: usize = 40;

/// Ramas de azar que se siguen en paralelo al completar un turno (las de la candidata cuentan);
/// pasado el tope, cada accion con azar sigue por una sola rama sorteada con su probabilidad
const MAX_COMPLETION_BRANCHES: usize = 8;

pub struct TurnCompletionPlayer {
    pub deck: Deck,
    /// Profundidad de la busqueda propia, como en e<n>: la accion `j` del resto del turno se elige
    /// con una busqueda de profundidad `max(1, max_depth - 1 - j)`
    pub max_depth: usize,
    pub value_function: ValueFunction,
    /// Puntaje de cada accion legal en la ultima decision (para los replays: el cuestionario de
    /// posiciones del step 3.6 saca de aca el margen entre la mejor y la segunda)
    last_scores: Option<Vec<f64>>,
}

impl TurnCompletionPlayer {
    /// Con la funcion de valor de `DECKGYM_VALUE_PARAMS`
    pub fn new(deck: Deck, max_depth: usize) -> Self {
        Self::with_value_function(
            deck,
            max_depth,
            Box::new(super::value_functions::loaded_value_function),
        )
    }

    /// Con otra funcion de valor (p<n> usa la de preparacion)
    pub fn with_value_function(
        deck: Deck,
        max_depth: usize,
        value_function: ValueFunction,
    ) -> Self {
        Self {
            deck,
            max_depth: max_depth.max(1),
            value_function,
            last_scores: None,
        }
    }

    /// El puntaje de cada accion: valor esperado al final de mi turno
    pub(crate) fn completed_scores(
        &self,
        rng: &mut StdRng,
        state: &State,
        actions: &[Action],
        myself: usize,
    ) -> Vec<f64> {
        actions
            .iter()
            .map(|action| self.completed_value(rng, state, action, myself))
            .collect()
    }

    /// Valor esperado (para `myself`) de jugar `action` y completar mi turno
    fn completed_value(
        &self,
        rng: &mut StdRng,
        state: &State,
        action: &Action,
        myself: usize,
    ) -> f64 {
        let outcomes = forecast_outcomes(rng, state, action);
        let width = outcomes.len();
        outcomes
            .into_iter()
            .map(|(prob, outcome)| {
                prob * self.complete_turn(rng, outcome, myself, state.turn_count, 1, width)
            })
            .sum()
    }

    /// Sigue mi turno `turn` desde `state` hasta que le toque al rival y evalua ahi. Las ramas de
    /// azar se promedian mientras haya menos de `MAX_COMPLETION_BRANCHES` abiertas (`width`).
    fn complete_turn(
        &self,
        rng: &mut StdRng,
        mut state: State,
        myself: usize,
        turn: u8,
        mut own_plies: usize,
        width: usize,
    ) -> f64 {
        let vf: ValueFn = &self.value_function;
        for _ in 0..MAX_COMPLETION_ACTIONS {
            if state.is_game_over() || state.current_player != myself || state.turn_count != turn {
                break;
            }
            let (actor, actions) = state.generate_possible_actions();
            if actions.is_empty() {
                break;
            }
            let chosen = if actions.len() == 1 {
                0
            } else if actor == myself {
                let depth = self.max_depth.saturating_sub(own_plies).max(1);
                decide(rng, &state, &actions, actor, depth, vf)
            } else {
                // Una eleccion del rival dentro de mi turno (promover tras un KO): su greedy
                decide(rng, &state, &actions, actor, 1, vf)
            };
            if actor == myself {
                own_plies += 1;
            }
            let mut outcomes = forecast_outcomes(rng, &state, &actions[chosen]);
            if outcomes.len() > 1 && width * outcomes.len() <= MAX_COMPLETION_BRANCHES {
                let width = width * outcomes.len();
                return outcomes
                    .into_iter()
                    .map(|(prob, outcome)| {
                        prob * self.complete_turn(rng, outcome, myself, turn, own_plies, width)
                    })
                    .sum();
            }
            let pick = sample_branch(rng, &outcomes);
            state = outcomes.swap_remove(pick).1;
        }
        vf(&state, myself)
    }
}

/// Indice de una rama sorteada con su probabilidad
fn sample_branch(rng: &mut StdRng, outcomes: &[(f64, State)]) -> usize {
    if outcomes.len() == 1 {
        return 0;
    }
    let mut left = rng.gen::<f64>() * outcomes.iter().map(|(p, _)| p).sum::<f64>();
    for (i, (prob, _)) in outcomes.iter().enumerate() {
        left -= prob;
        if left < 0.0 {
            return i;
        }
    }
    outcomes.len() - 1
}

impl Player for TurnCompletionPlayer {
    fn decision_fn(
        &mut self,
        rng: &mut StdRng,
        state: &State,
        possible_actions: &[Action],
    ) -> Action {
        if possible_actions.len() == 1 {
            self.last_scores = None;
            return possible_actions[0].clone();
        }
        let myself = possible_actions[0].actor;
        let original_level = log::max_level();
        log::set_max_level(LevelFilter::Info); // Silencia los logs de debug y trace de la busqueda

        // En el setup y en las decisiones mias durante el turno rival (promover tras un KO) no
        // hay turno propio que completar: ahi decide la busqueda de e<n>
        let scores = if state.turn_count == 0 || state.current_player != myself {
            expectiminimax_scores(
                rng,
                state,
                possible_actions,
                self.max_depth,
                myself,
                &self.value_function,
            )
        } else {
            self.completed_scores(rng, state, possible_actions, myself)
        };

        log::set_max_level(original_level);
        let chosen = argmax_last(&scores);
        self.last_scores = Some(scores);
        possible_actions[chosen].clone()
    }

    fn get_deck(&self) -> Deck {
        self.deck.clone()
    }

    fn last_scores(&self) -> Option<Vec<f64>> {
        self.last_scores.clone()
    }
}

impl Debug for TurnCompletionPlayer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TurnCompletionPlayer")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::{apply_action, SimpleAction};
    use crate::card_ids::CardId;
    use crate::models::{EnergyType, PlayedCard};
    use crate::players::{
        baseline_value_function, parametric_value_function, ExpectiMiniMaxPlayer,
        ValueFunctionParams,
    };
    use crate::test_support::get_test_game_with_board;
    use rand::SeedableRng;

    /// Los coeficientes de value_params/step-2-2.json
    fn step_2_2(state: &State, myself: usize) -> f64 {
        let params = ValueFunctionParams {
            real_damage: true,
            energy_damage: 2.0,
            active_pokemon_online_score: 0.0,
            active_damage: 1.0,
            ..ValueFunctionParams::baseline()
        };
        parametric_value_function(state, myself, &params)
    }

    const VALUE_FUNCTIONS: [(&str, fn(&State, usize) -> f64); 2] = [
        ("baseline", baseline_value_function),
        ("step-2-2", step_2_2),
    ];

    /// Mi turno 3: Bulbasaur activo con [G][G] (Vine Whip, 40) y Greninja en la banca (Water
    /// Shuriken: 20 a cualquier Pokemon rival, gratis, una vez por turno). El rival tiene un
    /// Charmander activo con 60 PS (no es debil a [G]) y un Rattata con `bench_hp` PS en la banca.
    /// Shuriken al Charmander mas Vine Whip lo noquea; atacar solo no.
    fn shuriken_state(bench_hp: u32) -> State {
        let game = get_test_game_with_board(
            vec![
                PlayedCard::from_id(CardId::A1001Bulbasaur)
                    .with_energy(vec![EnergyType::Grass, EnergyType::Grass]),
                PlayedCard::from_id(CardId::A1089Greninja),
            ],
            vec![
                PlayedCard::from_id(CardId::A1033Charmander),
                PlayedCard::from_id(CardId::A1189Rattata).with_remaining_hp(bench_hp),
            ],
        );
        let mut state = game.get_state_clone();
        state.hands[0].clear();
        state.hands[1].clear();
        state.move_generation_stack.clear();
        state
    }

    /// Juega el turno 3 de player 0 con `player`: (uso Water Shuriken, ataco, puntos al pasar)
    fn play_shuriken_turn(player: &mut dyn Player, bench_hp: u32, seed: u64) -> (bool, bool, u8) {
        let mut rng = StdRng::seed_from_u64(seed);
        let mut state = shuriken_state(bench_hp);
        let (mut shuriken, mut attacked) = (false, false);
        while !state.is_game_over() && state.current_player == 0 && state.turn_count == 3 {
            let (actor, actions) = state.generate_possible_actions();
            let action = if actions.len() == 1 || actor != 0 {
                actions[0].clone()
            } else {
                player.decision_fn(&mut rng, &state, &actions)
            };
            shuriken |= action.action == SimpleAction::UseAbility { in_play_idx: 1 };
            attacked |= matches!(action.action, SimpleAction::Attack(_));
            apply_action(&mut rng, &mut state, &action);
        }
        (shuriken, attacked, state.points[0])
    }

    fn e2(vf: fn(&State, usize) -> f64) -> ExpectiMiniMaxPlayer {
        ExpectiMiniMaxPlayer {
            deck: Deck::default(),
            max_depth: 2,
            write_debug_trees: false,
            value_function: Box::new(vf),
        }
    }

    fn t2(vf: fn(&State, usize) -> f64) -> TurnCompletionPlayer {
        TurnCompletionPlayer::with_value_function(Deck::default(), 2, Box::new(vf))
    }

    #[test]
    fn e2_attacks_and_loses_water_shuriken() {
        // El diagnostico del step 2.5: "habilidad primero" se evalua a mitad de turno (habilidad y
        // objetivo gastan las dos acciones), sin el Vine Whip que completa el KO
        for (name, vf) in VALUE_FUNCTIONS {
            for seed in 0..3 {
                assert_eq!(
                    play_shuriken_turn(&mut e2(vf), 40, seed),
                    (false, true, 0),
                    "e2 ataca sin tirar el shuriken ({name}, seed {seed})"
                );
            }
        }
    }

    #[test]
    fn t2_uses_water_shuriken_before_attacking() {
        for (name, vf) in VALUE_FUNCTIONS {
            for seed in 0..3 {
                assert_eq!(
                    play_shuriken_turn(&mut t2(vf), 40, seed),
                    (true, true, 1),
                    "t2 tira el shuriken, ataca y noquea ({name}, seed {seed})"
                );
            }
        }
    }

    #[test]
    fn e2_sees_a_water_shuriken_that_knocks_out_by_itself() {
        // Con el Rattata en 20 PS el KO entra en las dos acciones de e2: el horizonte solo tapa
        // al shuriken cuando lo que rinde depende del ataque que viene despues
        for (name, vf) in VALUE_FUNCTIONS {
            for seed in 0..3 {
                assert_eq!(
                    play_shuriken_turn(&mut e2(vf), 20, seed),
                    (true, true, 1),
                    "e2 {name}"
                );
                assert_eq!(
                    play_shuriken_turn(&mut t2(vf), 20, seed),
                    (true, true, 1),
                    "t2 {name}"
                );
            }
        }
    }

    #[test]
    fn plays_whole_games_against_e2() {
        // Partidas enteras sin panico ni ciclos, con decisiones siempre legales
        for seed in 0..3 {
            let game = crate::test_support::get_initialized_game(seed);
            let mut state = game.get_state_clone();
            let mut rng = StdRng::seed_from_u64(seed);
            let (mut ours, mut theirs) = (t2(baseline_value_function), e2(baseline_value_function));
            let mut ticks = 0;
            while !state.is_game_over() && ticks < 2000 {
                let (actor, actions) = state.generate_possible_actions();
                let player: &mut dyn Player = if actor == 0 { &mut ours } else { &mut theirs };
                let action = player.decision_fn(&mut rng, &state, &actions);
                assert!(actions.contains(&action), "seed {seed} tick {ticks}");
                apply_action(&mut rng, &mut state, &action);
                ticks += 1;
            }
            assert!(
                state.is_game_over(),
                "seed {seed}: {ticks} acciones sin terminar"
            );
        }
    }
}
