use rand::rngs::StdRng;
use std::fmt::Debug;

use crate::actions::{forecast_action, Action};
use crate::{Deck, State};

use super::value_functions::loaded_value_function;
use super::Player;

/// Greedy one-ply player: picks the action whose expected resulting state scores best under the
/// parametric value function (`DECKGYM_VALUE_PARAMS`, or the baseline coefficients).
pub struct ValueFunctionPlayer {
    pub deck: Deck,
}

impl Player for ValueFunctionPlayer {
    fn decision_fn(
        &mut self,
        rng: &mut StdRng,
        state: &State,
        possible_actions: &[Action],
    ) -> Action {
        // Get value for the possible actions
        let myself = possible_actions[0].actor;
        let scores: Vec<f64> = possible_actions
            .iter()
            .map(|action| expected_value_function(rng, state, action, myself))
            .collect();

        // Select the one with best score
        let best_idx = scores
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap())
            .unwrap()
            .0;

        possible_actions[best_idx].clone()
    }

    fn get_deck(&self) -> Deck {
        self.deck.clone()
    }
}

/// How many of its own pending follow-up choices (see `state_value`) the player resolves before
/// scoring a state.
const MAX_FOLLOW_UP_CHOICES: usize = 3;

fn expected_value_function(rng: &mut StdRng, state: &State, action: &Action, myself: usize) -> f64 {
    expected_value_with_follow_ups(rng, state, action, myself, MAX_FOLLOW_UP_CHOICES)
}

fn expected_value_with_follow_ups(
    rng: &mut StdRng,
    state: &State,
    action: &Action,
    myself: usize,
    follow_ups_left: usize,
) -> f64 {
    let (probabilities, mutations) = forecast_action(state, action).into_branches();
    let mut outcomes: Vec<State> = vec![];
    for mutation in mutations {
        let mut state = state.clone();
        mutation(rng, &mut state, action);
        outcomes.push(state);
    }
    outcomes
        .iter()
        .zip(probabilities.iter())
        .map(|(outcome, prob)| state_value(rng, outcome, myself, follow_ups_left) * prob)
        .sum()
}

/// Scores `state`, but when the action just taken left a pending choice of mine on the move
/// generation stack (e.g. Rare Candy or Poké Ball asking which card to pick), resolves it with
/// the best follow-up first. Otherwise a card whose payoff comes only in that follow-up choice
/// looks like "one card fewer in hand" and never gets played.
fn state_value(rng: &mut StdRng, state: &State, myself: usize, follow_ups_left: usize) -> f64 {
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
                    expected_value_with_follow_ups(rng, state, action, myself, follow_ups_left - 1)
                })
                .fold(f64::NEG_INFINITY, f64::max);
        }
    }
    loaded_value_function(state, myself)
}

impl Debug for ValueFunctionPlayer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ValueFunctionPlayer")
    }
}
