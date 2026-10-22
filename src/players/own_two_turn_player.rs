// Jugador d<n>: t<n> que mira dos turnos propios (step 2.9, ver docs/search-design.md).
//
// t<n> evalua cada jugada al final de mi turno, asi que preparar (unir energia al Treecko de la
// banca con Rare Candy en la mano) solo vale lo que la funcion de valor le da a un Treecko
// cargado, y atacar ya vale el dano entero. d<n> re-puntua las `top` mejores jugadas de t<n>
// jugando, sobre `samples` determinizaciones como o<n>: el resto de mi turno, el turno del rival
// con su greedy y mi turno siguiente con greedy, y evalua cuando le vuelve a tocar al rival. Asi
// una jugada de preparacion se cobra el ataque que habilita.

use log::LevelFilter;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use std::fmt::Debug;

use crate::actions::{apply_action, Action};
use crate::{Deck, State};

use super::own_reply_player::{argmax_last, decide, determinize, env_usize, ValueFn};
use super::own_turn_player::TurnCompletionPlayer;
use super::{Player, ValueFunction};

/// Muestras (determinizaciones) por candidata
pub const TWO_SAMPLES_ENV: &str = "DECKGYM_TWO_SAMPLES";
/// Candidatas de t<n> que se re-puntuan jugando dos turnos propios
pub const TWO_TOP_ENV: &str = "DECKGYM_TWO_TOP";
/// Profundidad de la busqueda en cada decision de mi turno siguiente (1 = greedy)
pub const TWO_DEPTH_ENV: &str = "DECKGYM_TWO_DEPTH";

pub const DEFAULT_TWO_SAMPLES: usize = 2;
pub const DEFAULT_TWO_TOP: usize = 3;
pub const DEFAULT_TWO_DEPTH: usize = 2;

/// Tope de acciones por simulacion (mi turno, el del rival y el siguiente mio)
const MAX_ROLLOUT_ACTIONS: usize = 120;

pub struct TwoTurnPlayer {
    /// t<n>: puntua todas las jugadas y elige las candidatas
    pub inner: TurnCompletionPlayer,
    pub samples: usize,
    pub top: usize,
    /// Profundidad de la busqueda en mi turno siguiente: con 2 encuentra "retirar y atacar con el
    /// que se preparo", que a un ply parece peor que pasar
    pub next_depth: usize,
}

impl TwoTurnPlayer {
    /// Con los parametros de las variables de entorno (o los defaults) y la funcion de valor dada
    pub fn new(deck: Deck, max_depth: usize, value_function: ValueFunction) -> Self {
        Self {
            inner: TurnCompletionPlayer::with_value_function(deck, max_depth, value_function),
            samples: env_usize(TWO_SAMPLES_ENV, DEFAULT_TWO_SAMPLES),
            top: env_usize(TWO_TOP_ENV, DEFAULT_TWO_TOP),
            next_depth: env_usize(TWO_DEPTH_ENV, DEFAULT_TWO_DEPTH).max(1),
        }
    }

    /// Juega `action`, el resto de mi turno, el turno del rival y mi turno siguiente sobre una
    /// determinizacion sembrada con `seed`, y evalua cuando le vuelve a tocar al rival (o termina
    /// la partida)
    fn rollout(&self, state: &State, action: &Action, myself: usize, seed: u64) -> f64 {
        let vf: ValueFn = &self.inner.value_function;
        let mut rng = StdRng::seed_from_u64(seed);
        let mut state = state.clone();
        // Con elecciones pendientes no se rearma nada (ver o<n>)
        if state.move_generation_stack.is_empty() {
            determinize(&mut state, myself, &mut rng);
        }
        let start_turn = state.turn_count;
        apply_action(&mut rng, &mut state, action);
        let mut own_plies = 1;

        for _ in 0..MAX_ROLLOUT_ACTIONS {
            if state.is_game_over() {
                break;
            }
            // Termino mi turno siguiente: aca se evalua
            if state.turn_count >= start_turn + 3 {
                break;
            }
            let (actor, actions) = state.generate_possible_actions();
            if actions.is_empty() {
                break;
            }
            let this_turn = state.current_player == myself && state.turn_count == start_turn;
            let chosen = if actions.len() == 1 {
                0
            } else if actor == myself && this_turn {
                // El resto de este turno con el horizonte que le quedaba a t<n>
                let depth = self.inner.max_depth.saturating_sub(own_plies).max(1);
                decide(&mut rng, &state, &actions, actor, depth, vf)
            } else if actor == myself && state.current_player == myself {
                // Mi turno siguiente
                decide(&mut rng, &state, &actions, actor, self.next_depth, vf)
            } else {
                // El turno del rival (y mis elecciones dentro de el), con greedy
                decide(&mut rng, &state, &actions, actor, 1, vf)
            };
            if actor == myself && this_turn {
                own_plies += 1;
            }
            apply_action(&mut rng, &mut state, &actions[chosen]);
        }
        vf(&state, myself)
    }
}

impl Player for TwoTurnPlayer {
    fn decision_fn(
        &mut self,
        rng: &mut StdRng,
        state: &State,
        possible_actions: &[Action],
    ) -> Action {
        if possible_actions.len() == 1 {
            return possible_actions[0].clone();
        }
        let myself = possible_actions[0].actor;
        // En el setup y en las decisiones mias durante el turno rival decide t<n> (que ahi usa la
        // busqueda de e<n>)
        if state.turn_count == 0 || state.current_player != myself || self.samples == 0 {
            return self.inner.decision_fn(rng, state, possible_actions);
        }
        let original_level = log::max_level();
        log::set_max_level(LevelFilter::Info); // Silencia los logs de debug y trace de la busqueda

        let scores = self.inner.completed_scores(rng, state, possible_actions, myself);
        let t_choice = argmax_last(&scores);
        let mut chosen = t_choice;
        if self.top > 1 {
            // Candidatas: la de t<n> primero y despues por puntaje
            let mut order: Vec<usize> = (0..possible_actions.len()).collect();
            order.sort_by(|&a, &b| scores[b].partial_cmp(&scores[a]).unwrap());
            order.retain(|&i| i != t_choice);
            order.insert(0, t_choice);
            order.truncate(self.top);

            // Las mismas semillas para todas las candidatas (numeros aleatorios comunes)
            let seeds: Vec<u64> = (0..self.samples).map(|_| rng.gen()).collect();
            let mut best_value = f64::NEG_INFINITY;
            for &idx in &order {
                let total: f64 = seeds
                    .iter()
                    .map(|&seed| self.rollout(state, &possible_actions[idx], myself, seed))
                    .sum();
                let value = total / seeds.len() as f64;
                // Estricto: un empate lo gana la que venia antes en el orden de t<n>
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
        self.inner.deck.clone()
    }
}

impl Debug for TwoTurnPlayer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TwoTurnPlayer")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card_ids::CardId;
    use crate::players::own_value::tests::{board, candy_state, d2, energy_on, play_turn, t2};

    #[test]
    fn d2_attacks_now_in_the_sharp_fang_position() {
        // La posicion de own_value::tests::p2_charges_the_treecko_instead_of_attacking_now: cargar
        // al Treecko gana solo si el turno siguiente se juega Rare Candy, [G], retirar y Terminating
        // Tail. Con 2 muestras y busqueda de profundidad 2 en ese turno, d2 no lo encuentra en las
        // suficientes muestras y ataca ya, como t2
        for seed in 0..3 {
            let (end, attacked) = play_turn(&mut d2(), candy_state(CardId::B1206Furfrou), seed);
            assert!(attacked, "seed {seed}: {:?}", board(&end));
            assert_eq!(energy_on(&end, "Treecko"), 0, "seed {seed}: {:?}", board(&end));
        }
    }

    #[test]
    fn plays_whole_games_against_t2() {
        // Partidas enteras sin panico ni ciclos, con decisiones siempre legales
        for seed in 0..2 {
            let game = crate::test_support::get_initialized_game(seed);
            let mut state = game.get_state_clone();
            let mut rng = StdRng::seed_from_u64(seed);
            let (mut ours, mut theirs) = (d2(), t2());
            let mut ticks = 0;
            while !state.is_game_over() && ticks < 2000 {
                let (actor, actions) = state.generate_possible_actions();
                let player: &mut dyn Player = if actor == 0 { &mut ours } else { &mut theirs };
                let action = player.decision_fn(&mut rng, &state, &actions);
                assert!(actions.contains(&action), "seed {seed} tick {ticks}");
                apply_action(&mut rng, &mut state, &action);
                ticks += 1;
            }
            assert!(state.is_game_over(), "seed {seed}: {ticks} acciones sin terminar");
        }
    }
}
