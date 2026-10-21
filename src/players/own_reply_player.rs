// Jugador o<n>: la busqueda de e<n> mas el turno del rival (step 2.3, ver docs/search-design.md).
//
// Fase 1: la busqueda de e<n> puntua todas las acciones legales y quedan las `top` mejores.
// Fase 2: cada candidata se juega `samples` veces sobre una determinizacion de la informacion
// oculta (mano y mazo del rival, orden de mi mazo): termino mi turno, el rival juega el suyo con
// su mejor accion segun la misma funcion de valor, y al empezar mi turno siguiente se evalua.
// Gana la candidata con mejor promedio.

use log::LevelFilter;
use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng};
use std::fmt::Debug;

use crate::actions::{apply_action, forecast_action, Action};
use crate::{Deck, State};

use super::{Player, ValueFunction};

pub(crate) type ValueFn<'a> = &'a dyn Fn(&State, usize) -> f64;

/// Muestras (determinizaciones) por candidata
pub const OPP_SAMPLES_ENV: &str = "DECKGYM_OPP_SAMPLES";
/// Candidatas de la fase 1 que se re-puntuan con el turno del rival
pub const OPP_TOP_ENV: &str = "DECKGYM_OPP_TOP";
/// Profundidad de la busqueda del rival en cada decision de su turno (1 = greedy de un ply)
pub const OPP_DEPTH_ENV: &str = "DECKGYM_OPP_DEPTH";

pub const DEFAULT_OPP_SAMPLES: usize = 2;
pub const DEFAULT_OPP_TOP: usize = 3;
pub const DEFAULT_OPP_DEPTH: usize = 1;

/// Tope de acciones por simulacion, por si alguna politica entra en un ciclo
const MAX_ROLLOUT_ACTIONS: usize = 80;

pub struct OpponentReplyPlayer {
    pub deck: Deck,
    /// Profundidad de la busqueda propia, como en e<n>
    pub max_depth: usize,
    pub samples: usize,
    pub top: usize,
    pub opponent_depth: usize,
    pub value_function: ValueFunction,
}

impl OpponentReplyPlayer {
    /// Con los parametros de las variables de entorno (o los defaults) y la funcion de valor de
    /// `DECKGYM_VALUE_PARAMS`.
    pub fn new(deck: Deck, max_depth: usize) -> Self {
        Self {
            deck,
            max_depth: max_depth.max(1),
            samples: env_usize(OPP_SAMPLES_ENV, DEFAULT_OPP_SAMPLES),
            top: env_usize(OPP_TOP_ENV, DEFAULT_OPP_TOP),
            opponent_depth: env_usize(OPP_DEPTH_ENV, DEFAULT_OPP_DEPTH).max(1),
            value_function: Box::new(super::value_functions::loaded_value_function),
        }
    }

    /// Promedio de la funcion de valor (para `myself`) al empezar mi turno siguiente, sobre las
    /// determinizaciones de `seeds`, despues de jugar `action`.
    fn reply_value(&self, state: &State, action: &Action, myself: usize, seeds: &[u64]) -> f64 {
        let total: f64 = seeds
            .iter()
            .map(|&seed| self.rollout(state, action, myself, seed))
            .sum();
        total / seeds.len() as f64
    }

    fn rollout(&self, state: &State, action: &Action, myself: usize, seed: u64) -> f64 {
        let vf: ValueFn = &self.value_function;
        let mut rng = StdRng::seed_from_u64(seed);
        let mut state = state.clone();
        // Con elecciones pendientes en la pila no se rearma nada: pueden nombrar cartas de una
        // mano revelada (Mega Absol ex descarta un Partidario de la mano rival) o del mazo
        if state.move_generation_stack.is_empty() {
            determinize(&mut state, myself, &mut rng);
        }
        let start_turn = state.turn_count;
        apply_action(&mut rng, &mut state, action);
        // Acciones mias que ya van en este turno (la candidata es la primera)
        let mut own_plies = 1;

        for _ in 0..MAX_ROLLOUT_ACTIONS {
            if state.is_game_over() {
                break;
            }
            // Empezo mi turno siguiente: aca se evalua
            if state.current_player == myself && state.turn_count > start_turn {
                break;
            }
            let (actor, actions) = state.generate_possible_actions();
            if actions.is_empty() {
                break;
            }
            let my_turn = state.current_player == myself && state.turn_count == start_turn;
            let chosen = if actions.len() == 1 {
                0
            } else if actor == myself && my_turn {
                // El resto de mi turno con el horizonte que le quedaba a e<n>
                let depth = self.max_depth.saturating_sub(own_plies).max(1);
                decide(&mut rng, &state, &actions, actor, depth, vf)
            } else if actor == myself {
                decide(&mut rng, &state, &actions, actor, 1, vf)
            } else {
                decide(&mut rng, &state, &actions, actor, self.opponent_depth, vf)
            };
            if actor == myself && my_turn {
                own_plies += 1;
            }
            apply_action(&mut rng, &mut state, &actions[chosen]);
        }
        vf(&state, myself)
    }
}

/// Indice de la mejor accion para `actor` con una busqueda de profundidad `depth` (1 = greedy de
/// un ply con las elecciones encadenadas resueltas, como `v`; mas = la busqueda de e<depth>).
pub(crate) fn decide(
    rng: &mut StdRng,
    state: &State,
    actions: &[Action],
    actor: usize,
    depth: usize,
    value_function: ValueFn,
) -> usize {
    if depth <= 1 {
        return greedy_action_index(rng, state, actions, actor, value_function);
    }
    let scores = expectiminimax_scores(rng, state, actions, depth, actor, value_function);
    argmax_last(&scores)
}

// La busqueda de e<n> y el greedy de v, copiados de expectiminimax_player.rs y
// value_function_player.rs (sin el arbol de debug) porque los jugadores de upstream no se tocan
// (regla 1 de docs/FORK.md). Mismo criterio: esperanza sobre las ramas de forecast_action, max en
// mis nodos, min en los del rival, corte cuando el turno pasa al otro.

/// El puntaje de e<depth> de cada accion para `myself` (lo que maximiza e<depth>)
pub(crate) fn expectiminimax_scores(
    rng: &mut StdRng,
    state: &State,
    actions: &[Action],
    depth: usize,
    myself: usize,
    value_function: ValueFn,
) -> Vec<f64> {
    actions
        .iter()
        .map(|action| expected_value(rng, state, action, depth - 1, myself, value_function))
        .collect()
}

fn expected_value(
    rng: &mut StdRng,
    state: &State,
    action: &Action,
    depth: usize,
    myself: usize,
    value_function: ValueFn,
) -> f64 {
    // Primero todas las ramas y despues la recursion, en el mismo orden que e<n> (asi consume el
    // rng igual y con la misma semilla elige lo mismo)
    let outcomes = forecast_outcomes(rng, state, action);
    outcomes
        .iter()
        .map(|(prob, outcome)| prob * expectiminimax(rng, outcome, depth, myself, value_function))
        .sum()
}

pub(crate) fn forecast_outcomes(
    rng: &mut StdRng,
    state: &State,
    action: &Action,
) -> Vec<(f64, State)> {
    let (probabilities, mutations) = forecast_action(state, action).into_branches();
    probabilities
        .into_iter()
        .zip(mutations)
        .map(|(prob, mutation)| {
            let mut outcome = state.clone();
            mutation(rng, &mut outcome, action);
            (prob, outcome)
        })
        .collect()
}

fn expectiminimax(
    rng: &mut StdRng,
    state: &State,
    depth: usize,
    myself: usize,
    value_function: ValueFn,
) -> f64 {
    if state.is_game_over() || depth == 0 || state.current_player != myself {
        return value_function(state, myself);
    }
    let (actor, actions) = state.generate_possible_actions();
    let scores = actions
        .iter()
        .map(|action| expected_value(rng, state, action, depth - 1, myself, value_function));
    if actor == myself {
        scores.fold(f64::NEG_INFINITY, f64::max)
    } else {
        scores.fold(f64::INFINITY, f64::min)
    }
}

/// Cuantas elecciones propias encadenadas (Pokeball, Rare Candy) resuelve el greedy antes de
/// puntuar, como `v`
const MAX_FOLLOW_UP_CHOICES: usize = 3;

/// La decision de `v`: la accion cuyo estado esperado puntua mejor para `myself` (el ultimo
/// maximo en caso de empate, como `max_by`)
fn greedy_action_index(
    rng: &mut StdRng,
    state: &State,
    actions: &[Action],
    myself: usize,
    value_function: ValueFn,
) -> usize {
    let scores: Vec<f64> = actions
        .iter()
        .map(|action| {
            greedy_expected_value(
                rng,
                state,
                action,
                myself,
                MAX_FOLLOW_UP_CHOICES,
                value_function,
            )
        })
        .collect();
    argmax_last(&scores)
}

fn greedy_expected_value(
    rng: &mut StdRng,
    state: &State,
    action: &Action,
    myself: usize,
    follow_ups_left: usize,
    value_function: ValueFn,
) -> f64 {
    let outcomes = forecast_outcomes(rng, state, action);
    outcomes
        .iter()
        .map(|(prob, outcome)| {
            prob * greedy_state_value(rng, outcome, myself, follow_ups_left, value_function)
        })
        .sum()
}

/// Si la accion dejo una eleccion mia pendiente en la pila, la resuelve con la mejor antes de
/// puntuar (si no, una carta que rinde en esa eleccion parece "una carta menos en la mano")
fn greedy_state_value(
    rng: &mut StdRng,
    state: &State,
    myself: usize,
    follow_ups_left: usize,
    value_function: ValueFn,
) -> f64 {
    if follow_ups_left > 0
        && !state.is_game_over()
        && state.current_player == myself
        && !state.move_generation_stack.is_empty()
    {
        let (actor, actions) = state.generate_possible_actions();
        if actor == myself && !actions.is_empty() {
            return actions
                .iter()
                .map(|action| {
                    greedy_expected_value(
                        rng,
                        state,
                        action,
                        myself,
                        follow_ups_left - 1,
                        value_function,
                    )
                })
                .fold(f64::NEG_INFINITY, f64::max);
        }
    }
    value_function(state, myself)
}

/// Rearma la informacion oculta para `myself` de forma consistente con lo visible: la mano y el
/// mazo del rival se juntan, se mezclan y se reparte una mano del mismo tamano (el resto queda de
/// mazo); mi mazo se mezcla. Mesa, descartes, energias y tamanos no cambian.
pub(crate) fn determinize(state: &mut State, myself: usize, rng: &mut StdRng) {
    let opponent = 1 - myself;
    let hand_size = state.hands[opponent].len();
    let mut pool = std::mem::take(&mut state.hands[opponent]);
    pool.append(&mut state.decks[opponent].cards);
    pool.shuffle(rng);
    state.decks[opponent].cards = pool.split_off(hand_size);
    state.hands[opponent] = pool;
    state.decks[myself].cards.shuffle(rng);
}

/// Como `max_by` sobre f64: el ultimo maximo en caso de empate (asi elige e<n>)
pub(crate) fn argmax_last(scores: &[f64]) -> usize {
    scores
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap())
        .map(|(idx, _)| idx)
        .unwrap()
}

pub(crate) fn env_usize(name: &str, default: usize) -> usize {
    match std::env::var(name) {
        Ok(value) if !value.is_empty() => value
            .parse()
            .unwrap_or_else(|_| panic!("Invalid {name}: {value} (se espera un entero)")),
        _ => default,
    }
}

impl Player for OpponentReplyPlayer {
    fn decision_fn(
        &mut self,
        rng: &mut StdRng,
        state: &State,
        possible_actions: &[Action],
    ) -> Action {
        let myself = possible_actions[0].actor;
        let original_level = log::max_level();
        log::set_max_level(LevelFilter::Info); // Silencia los logs de debug y trace de la busqueda

        // Fase 1: la busqueda de e<n>
        let scores = expectiminimax_scores(
            rng,
            state,
            possible_actions,
            self.max_depth,
            myself,
            &self.value_function,
        );
        let e_choice = argmax_last(&scores);

        // En el setup no hay turno rival que simular
        let mut chosen = e_choice;
        if state.turn_count > 0 && self.samples > 0 && self.top > 1 && possible_actions.len() > 1 {
            // Candidatas: la de e<n> primero y despues por puntaje
            let mut order: Vec<usize> = (0..possible_actions.len()).collect();
            order.sort_by(|&a, &b| scores[b].partial_cmp(&scores[a]).unwrap());
            order.retain(|&i| i != e_choice);
            order.insert(0, e_choice);
            order.truncate(self.top);

            // Fase 2: las mismas semillas para todas las candidatas (numeros aleatorios comunes)
            let seeds: Vec<u64> = (0..self.samples).map(|_| rng.gen()).collect();
            let mut best_value = f64::NEG_INFINITY;
            for &idx in &order {
                let value = self.reply_value(state, &possible_actions[idx], myself, &seeds);
                // Estricto: un empate lo gana la que venia antes en la fase 1
                if value > best_value {
                    best_value = value;
                    chosen = idx;
                }
            }
        }

        log::set_max_level(original_level);
        possible_actions[chosen].clone()
    }

    fn get_deck(&self) -> Deck {
        self.deck.clone()
    }
}

impl Debug for OpponentReplyPlayer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "OpponentReplyPlayer")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card_ids::CardId;
    use crate::models::{EnergyType, PlayedCard};
    use crate::players::{parametric_value_function, ExpectiMiniMaxPlayer, ValueFunctionParams};
    use crate::test_support::get_test_game_with_board;

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

    /// Mi turno 3: mi Bulbasaur activo tiene 30 HP y una energia (paga el retiro), en la banca hay
    /// otro Bulbasaur sano. El Charmander rival tiene su energia de fuego: Ember (30, Bulbasaur
    /// es debil a fuego) me noquea el activo el turno siguiente, pero no al de la banca.
    fn threatened_state() -> State {
        let game = get_test_game_with_board(
            vec![
                PlayedCard::from_id(CardId::A1001Bulbasaur)
                    .with_energy(vec![EnergyType::Grass])
                    .with_remaining_hp(30),
                PlayedCard::from_id(CardId::A1001Bulbasaur),
            ],
            vec![PlayedCard::from_id(CardId::A1033Charmander).with_energy(vec![EnergyType::Fire])],
        );
        let mut state = game.get_state_clone();
        state.hands[0].clear();
        state.hands[1].clear();
        state.move_generation_stack.clear();
        state
    }

    /// Juega el turno 3 de player 0 con `player` y devuelve el HP restante de su activo al pasar
    fn active_hp_after_turn(player: &mut dyn Player, seed: u64) -> u32 {
        let mut rng = StdRng::seed_from_u64(seed);
        let mut state = threatened_state();
        while !state.is_game_over() && state.current_player == 0 && state.turn_count == 3 {
            let (actor, actions) = state.generate_possible_actions();
            let action = if actions.len() == 1 || actor != 0 {
                actions[0].clone()
            } else {
                player.decision_fn(&mut rng, &state, &actions)
            };
            apply_action(&mut rng, &mut state, &action);
        }
        state.get_remaining_hp(0, 0)
    }

    fn o2() -> OpponentReplyPlayer {
        OpponentReplyPlayer {
            deck: Deck::default(),
            max_depth: 2,
            samples: DEFAULT_OPP_SAMPLES,
            top: DEFAULT_OPP_TOP,
            opponent_depth: DEFAULT_OPP_DEPTH,
            value_function: Box::new(step_2_2),
        }
    }

    #[test]
    fn retreats_the_threatened_active_and_e2_does_not() {
        for seed in 0..3 {
            let mut e2 = ExpectiMiniMaxPlayer {
                deck: Deck::default(),
                max_depth: 2,
                write_debug_trees: false,
                value_function: Box::new(step_2_2),
            };
            assert_eq!(
                active_hp_after_turn(&mut e2, seed),
                30,
                "e2 no ve el Ember del rival y se queda con el activo herido (seed {seed})"
            );
            assert_eq!(
                active_hp_after_turn(&mut o2(), seed),
                70,
                "o2 ve que el rival noquea al activo y retira al Bulbasaur sano (seed {seed})"
            );
        }
    }

    #[test]
    fn without_samples_it_plays_like_e2() {
        let mut player = OpponentReplyPlayer { samples: 0, ..o2() };
        assert_eq!(active_hp_after_turn(&mut player, 0), 30);
    }

    #[test]
    fn the_copied_search_chooses_like_e2() {
        // Sin muestras, o2 es la copia de la busqueda de e2: misma eleccion con la misma semilla
        // en cada decision de una partida entre jugadores al azar
        let mut game = crate::test_support::get_initialized_game(11);
        let mut e2 = ExpectiMiniMaxPlayer {
            deck: Deck::default(),
            max_depth: 2,
            write_debug_trees: false,
            value_function: Box::new(step_2_2),
        };
        let mut copy = OpponentReplyPlayer { samples: 0, ..o2() };
        let mut checked = 0;
        for tick in 0..200u64 {
            if game.is_game_over() {
                break;
            }
            let state = game.get_state_clone();
            let (_, actions) = state.generate_possible_actions();
            if actions.len() > 1 {
                let a = e2.decision_fn(&mut StdRng::seed_from_u64(tick), &state, &actions);
                let b = copy.decision_fn(&mut StdRng::seed_from_u64(tick), &state, &actions);
                assert_eq!(a, b, "tick {tick}");
                checked += 1;
            }
            game.play_tick();
        }
        assert!(checked > 10, "solo {checked} decisiones");
    }

    #[test]
    fn a_pending_choice_on_the_opponent_hand_is_not_determinized() {
        // Mega Absol ex deja en la pila "descarta este Partidario de la mano rival": la mano ya
        // esta revelada y las acciones nombran sus cartas, asi que no se puede rearmar
        use crate::actions::SimpleAction;
        use crate::database::get_card_by_enum;
        let mut state = threatened_state();
        let full = get_test_game_with_board(vec![], vec![]).get_state_clone();
        let revealed = vec![
            get_card_by_enum(CardId::A1033Charmander),
            get_card_by_enum(CardId::A1189Rattata),
        ];
        state.hands[1] = revealed.clone();
        state.decks[1].cards = full.decks[1].cards.clone();
        let choices: Vec<SimpleAction> = revealed
            .iter()
            .map(|card| SimpleAction::DiscardOpponentSupporter {
                supporter_card: card.clone(),
            })
            .collect();
        state.move_generation_stack.push((0, choices));
        let (actor, actions) = state.generate_possible_actions();
        assert_eq!((actor, actions.len()), (0, 2));
        for seed in 0..10 {
            let action = o2().decision_fn(&mut StdRng::seed_from_u64(seed), &state, &actions);
            assert!(actions.contains(&action));
        }
    }

    #[test]
    fn determinize_keeps_what_is_visible() {
        let mut state = threatened_state();
        let game = get_test_game_with_board(vec![], vec![]);
        let full = game.get_state_clone();
        state.hands[1] = full.decks[1].cards[..4].to_vec();
        state.decks[1].cards = full.decks[1].cards[4..].to_vec();
        let sorted = |cards: &[crate::models::Card]| {
            let mut ids: Vec<String> = cards.iter().map(|c| c.get_id()).collect();
            ids.sort();
            ids
        };
        let before_pool = sorted(&[state.hands[1].clone(), state.decks[1].cards.clone()].concat());
        let before_mine = sorted(&state.decks[0].cards);
        let my_hand = state.hands[0].clone();

        let mut rng = StdRng::seed_from_u64(7);
        determinize(&mut state, 0, &mut rng);
        assert_eq!(state.hands[1].len(), 4);
        assert_eq!(
            sorted(&[state.hands[1].clone(), state.decks[1].cards.clone()].concat()),
            before_pool
        );
        assert_eq!(sorted(&state.decks[0].cards), before_mine);
        assert_eq!(state.hands[0], my_hand);
        assert_eq!(state.get_remaining_hp(0, 0), 30);
    }
}
