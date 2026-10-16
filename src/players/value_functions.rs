// Collection of value functions for ExpectiMiniMaxPlayer
//
// Each value function evaluates a game state from a player's perspective
// and returns a score (higher is better for that player)

use std::sync::OnceLock;

use log::trace;
use serde::{Deserialize, Serialize};

use crate::card_logic::get_highest_evolutions;
use crate::hooks::energy_missing;
use crate::models::{Card, EnergyType, PlayedCard};
use crate::state::GameOutcome;
use crate::State;

/// Environment variable with the path to a JSON file of [`ValueFunctionParams`]. When set, the
/// `e<depth>` and `v` players use those coefficients instead of [`ValueFunctionParams::baseline`].
/// Fields missing from the JSON keep their baseline value; unknown fields are an error.
pub const VALUE_PARAMS_ENV: &str = "DECKGYM_VALUE_PARAMS";

/// Coefficients for the parametric value function
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ValueFunctionParams {
    pub points: f64,
    pub pokemon_value: f64,
    pub hand_size: f64,
    pub deck_size: f64,
    pub active_retreat_cost: f64,
    pub active_pokemon_online_score: f64,
    pub active_safety: f64,
    pub active_has_tool: f64,
    pub is_winner: f64,
    pub turns_until_opponent_wins: f64,
    pub online_pokemon_count: f64,
    pub energy_distance_to_online: f64,
    pub opponent_discard_size: f64,
    // Features below are 0 in the baseline, so they don't change its behavior.
    /// Damage counters (in HP) on the opponent's active minus on mine.
    pub active_damage: f64,
    /// Remaining HP on my bench minus the opponent's.
    pub bench_hp: f64,
    /// Number of benched Pokémon, mine minus the opponent's.
    pub bench_count: f64,
    /// Sum of evolution stages of Pokémon in play, mine minus the opponent's.
    pub evolution_stage: f64,
    /// Pokémon in hand that can evolve something already in play, mine minus the opponent's.
    pub evolutions_in_hand: f64,
    /// Special conditions on the opponent's active minus on mine.
    pub active_status: f64,
    /// 1 if my active can knock out the opponent's active with an attack it can already pay
    /// for (fixed damage only), minus the same for the opponent.
    pub can_ko_active: f64,
    /// Total energy attached to Pokémon in play, mine minus the opponent's.
    pub attached_energy: f64,
}

impl Default for ValueFunctionParams {
    fn default() -> Self {
        Self::baseline()
    }
}

impl ValueFunctionParams {
    /// Baseline parameters (same as original baseline function)
    pub const fn baseline() -> Self {
        Self {
            points: 10_000.0,
            pokemon_value: 1.0,
            hand_size: 1.0,
            deck_size: 1.0,
            active_retreat_cost: 1.0,
            active_pokemon_online_score: 500.0,
            active_safety: 1.0,
            active_has_tool: 10.0,
            is_winner: 100_000.0,
            turns_until_opponent_wins: 100.0,
            online_pokemon_count: 0.0,
            energy_distance_to_online: 0.0,
            opponent_discard_size: 0.1,
            active_damage: 0.0,
            bench_hp: 0.0,
            bench_count: 0.0,
            evolution_stage: 0.0,
            evolutions_in_hand: 0.0,
            active_status: 0.0,
            can_ko_active: 0.0,
            attached_energy: 0.0,
        }
    }

    /// Variant parameters
    pub const fn variant() -> Self {
        Self::baseline()
    }

    pub fn from_json_file(path: &str) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
        serde_json::from_str(&text).map_err(|e| format!("{path}: {e}"))
    }

    /// Parameters from the JSON file named by `DECKGYM_VALUE_PARAMS`, or the baseline if the
    /// variable is unset or empty. Read once per process. Panics if the file can't be loaded, so a
    /// typo never silently falls back to the baseline.
    pub fn loaded() -> &'static Self {
        static LOADED: OnceLock<ValueFunctionParams> = OnceLock::new();
        LOADED.get_or_init(|| match std::env::var(VALUE_PARAMS_ENV) {
            Ok(path) if !path.is_empty() => Self::from_json_file(&path)
                .unwrap_or_else(|e| panic!("Invalid {VALUE_PARAMS_ENV}: {e}")),
            _ => Self::baseline(),
        })
    }
}

pub fn baseline_value_function(state: &State, myself: usize) -> f64 {
    parametric_value_function(state, myself, &ValueFunctionParams::baseline())
}

/// A variant of the baseline value function
pub fn variant_value_function(state: &State, myself: usize) -> f64 {
    parametric_value_function(state, myself, &ValueFunctionParams::variant())
}

/// The parametric value function with [`ValueFunctionParams::loaded`] coefficients
pub fn loaded_value_function(state: &State, myself: usize) -> f64 {
    parametric_value_function(state, myself, ValueFunctionParams::loaded())
}

/// Parametric value function that uses the provided coefficients
pub fn parametric_value_function(
    state: &State,
    myself: usize,
    params: &ValueFunctionParams,
) -> f64 {
    let opponent = (myself + 1) % 2;
    let (my, opp) = (
        extract_features(state, myself, params),
        extract_features(state, opponent, params),
    );
    let score = (my.points - opp.points) * params.points
        + (my.pokemon_value - opp.pokemon_value) * params.pokemon_value
        + (my.hand_size - opp.hand_size) * params.hand_size
        + (opp.deck_size - my.deck_size) * params.deck_size
        + (-my.active_retreat_cost) * params.active_retreat_cost
        + (my.active_pokemon_online_score - opp.active_pokemon_online_score)
            * params.active_pokemon_online_score
        + (my.active_safety - opp.active_safety) * params.active_safety
        + (my.active_has_tool - opp.active_has_tool) * params.active_has_tool
        + (my.is_winner - opp.is_winner) * params.is_winner
        + (my.turns_until_opponent_wins - opp.turns_until_opponent_wins)
            * params.turns_until_opponent_wins
        + (my.online_pokemon_count - opp.online_pokemon_count) * params.online_pokemon_count
        + (my.energy_distance_to_online - opp.energy_distance_to_online)
            * params.energy_distance_to_online
        + opp.discard_size * params.opponent_discard_size
        + (opp.active_damage - my.active_damage) * params.active_damage
        + (my.bench_hp - opp.bench_hp) * params.bench_hp
        + (my.bench_count - opp.bench_count) * params.bench_count
        + (my.evolution_stage - opp.evolution_stage) * params.evolution_stage
        + (my.evolutions_in_hand - opp.evolutions_in_hand) * params.evolutions_in_hand
        + (opp.active_status - my.active_status) * params.active_status
        + (my.can_ko_active - opp.can_ko_active) * params.can_ko_active
        + (my.attached_energy - opp.attached_energy) * params.attached_energy;
    trace!("parametric_value_function: {score} (params: {params:?}, my: {my:?}, opp: {opp:?})");
    score
}

/// Features extracted from a player's game state
#[derive(Debug)]
struct Features {
    points: f64,
    pokemon_value: f64,
    hand_size: f64,
    deck_size: f64,
    active_retreat_cost: f64,
    online_pokemon_count: f64,
    energy_distance_to_online: f64,
    active_pokemon_online_score: f64,
    active_safety: f64,
    active_has_tool: f64,
    is_winner: f64,
    turns_until_opponent_wins: f64,
    discard_size: f64,
    active_damage: f64,
    bench_hp: f64,
    bench_count: f64,
    evolution_stage: f64,
    evolutions_in_hand: f64,
    active_status: f64,
    can_ko_active: f64,
    attached_energy: f64,
}

/// Computes `f` only when its coefficient is used, since the score ignores it otherwise.
fn if_used(coefficient: f64, f: impl FnOnce() -> f64) -> f64 {
    if coefficient != 0.0 {
        f()
    } else {
        0.0
    }
}

/// Extract features for a single player
fn extract_features(state: &State, player: usize, params: &ValueFunctionParams) -> Features {
    let active_factor = 1.0;
    let points = state.points[player] as f64;
    let pokemon_value = if_used(params.pokemon_value, || {
        calculate_pokemon_value(state, player, active_factor)
    });
    let hand_size = state.hands[player].len() as f64;
    let deck_size = state.decks[player].cards.len() as f64;
    let active_retreat_cost = get_active_retreat_cost(state, player) as f64;
    let (online_pokemon_count, energy_distance_to_online) =
        if params.online_pokemon_count != 0.0 || params.energy_distance_to_online != 0.0 {
            calculate_online_metrics(state, player, active_factor)
        } else {
            (0.0, 0.0)
        };
    let active_pokemon_online_score = if_used(params.active_pokemon_online_score, || {
        calculate_active_pokemon_online_score(state, player)
    });
    let active_safety = calculate_active_safety(state, player);
    let active_has_tool = get_active_has_tool(state, player);
    let is_winner = check_is_winner(state, player);
    let turns_until_opponent_wins = if_used(params.turns_until_opponent_wins, || {
        calculate_turns_until_opponent_wins(state, player)
    });
    let discard_size = state.discard_piles[player].len() as f64;
    let active = state.maybe_get_active(player);
    let active_damage = active.map_or(0.0, |a| a.get_damage_counters() as f64);
    let bench_hp = state
        .enumerate_bench_pokemon(player)
        .map(|(_, card)| card.get_remaining_hp() as f64)
        .sum();
    let bench_count = state.enumerate_bench_pokemon(player).count() as f64;
    let evolution_stage = state
        .enumerate_in_play_pokemon(player)
        .map(|(_, card)| match &card.card {
            Card::Pokemon(pokemon) => pokemon.stage as f64,
            Card::Trainer(_) => 0.0,
        })
        .sum();
    let evolutions_in_hand = if_used(params.evolutions_in_hand, || {
        count_evolutions_in_hand(state, player)
    });
    let active_status = active.map_or(0.0, |a| a.count_status_conditions() as f64);
    let can_ko_active = if_used(params.can_ko_active, || {
        can_ko_opponent_active(state, player)
    });
    let attached_energy = state
        .enumerate_in_play_pokemon(player)
        .map(|(_, card)| card.attached_energy.len() as f64)
        .sum();

    Features {
        points,
        pokemon_value,
        hand_size,
        deck_size,
        active_retreat_cost,
        online_pokemon_count,
        energy_distance_to_online,
        active_pokemon_online_score,
        active_safety,
        active_has_tool,
        is_winner,
        turns_until_opponent_wins,
        discard_size,
        active_damage,
        bench_hp,
        bench_count,
        evolution_stage,
        evolutions_in_hand,
        active_status,
        can_ko_active,
        attached_energy,
    }
}

/// Pokémon cards in hand that can evolve at least one of the player's Pokémon in play
fn count_evolutions_in_hand(state: &State, player: usize) -> f64 {
    state.hands[player]
        .iter()
        .filter(|hand_card| {
            state
                .enumerate_in_play_pokemon(player)
                .any(|(_, in_play)| in_play.card.can_evolve_into(hand_card))
        })
        .count() as f64
}

/// 1.0 if the player's active has an attack it can already pay for whose fixed damage reaches
/// the opponent's active remaining HP (ignores weakness and damage modifiers)
fn can_ko_opponent_active(state: &State, player: usize) -> f64 {
    let opponent = (player + 1) % 2;
    let (Some(mine), Some(theirs)) = (
        state.maybe_get_active(player),
        state.maybe_get_active(opponent),
    ) else {
        return 0.0;
    };
    let target_hp = theirs.get_remaining_hp();
    let can_ko = mine.card.get_attacks().iter().any(|atk| {
        atk.fixed_damage > 0
            && atk.fixed_damage >= target_hp
            && energy_missing(mine, &atk.energy_required, state, player).is_empty()
    });
    if can_ko {
        1.0
    } else {
        0.0
    }
}

fn get_active_retreat_cost(state: &State, player: usize) -> usize {
    state
        .maybe_get_active(player)
        .map(|card| card.card.get_retreat_cost().map(|rc| rc.len()).unwrap_or(5))
        .unwrap_or(0)
}

/// Check if active pokemon has a tool attached
fn get_active_has_tool(state: &State, player: usize) -> f64 {
    state
        .maybe_get_active(player)
        .map(|card| if card.has_tool_attached() { 1.0 } else { 0.0 })
        .unwrap_or(0.0)
}

/// Check if the player has won the game
fn check_is_winner(state: &State, player: usize) -> f64 {
    match state.winner {
        Some(GameOutcome::Win(winner)) if winner == player => 1.0,
        _ => 0.0,
    }
}

/// Calculate expected turns until opponent wins
/// Uses opponent's active damage and simulates KOs until opponent reaches 3 points
fn calculate_turns_until_opponent_wins(state: &State, player: usize) -> f64 {
    let opponent = (player + 1) % 2;

    // Find the closest pokemon to being able to deal damage (by energy requirements)
    let best_threat = state
        .enumerate_in_play_pokemon(opponent)
        .filter_map(|(_, pokemon)| {
            let best_attack = pokemon
                .card
                .get_attacks()
                .iter()
                .filter(|atk| atk.fixed_damage > 0) // Only consider attacks that deal damage
                .map(|atk| {
                    let missing = energy_missing(pokemon, &atk.energy_required, state, opponent);
                    (atk.fixed_damage, missing.len())
                })
                .min_by_key(|(damage, missing)| (*missing, u32::MAX - damage)); // Prioritize by missing energy, then by damage

            best_attack
        })
        .min_by_key(|(damage, missing)| (*missing, u32::MAX - damage)); // Find pokemon with least missing energy
    let (max_damage, missing_energy) = match best_threat {
        Some((damage, missing)) => (damage as f64, missing),
        None => return 30.0, // No pokemon can deal damage
    };

    let mut total_turns = 0.0;
    let mut opp_points = state.points[opponent];

    // If the best threat still can't attack (missing energy), factor that into the calculation
    total_turns += missing_energy as f64;

    // Calculate turns to KO my active pokemon
    if let Some(my_active) = state.maybe_get_active(player) {
        let turns_to_ko = (my_active.get_remaining_hp() as f64 / max_damage).ceil();
        total_turns += turns_to_ko;
        opp_points += my_active.card.get_knockout_points();
    }

    // Simulate KOing bench pokemon until opponent has 3+ points
    while opp_points < 3 {
        // Find the safest bench pokemon (highest hp / ko_points)
        let safest_bench = state
            .enumerate_bench_pokemon(player)
            .max_by_key(|(_, card)| {
                // if missing 1 point, just do by HP. if missing more than 1 point,
                // do by point yield hp / ko_points
                if opp_points == 2 {
                    card.get_remaining_hp()
                } else {
                    let ko_points = card.card.get_knockout_points().max(1) as u32;
                    card.get_remaining_hp() * 1000 / ko_points
                }
            });

        let Some((_, safest_pokemon)) = safest_bench else {
            break; // No more bench pokemon
        };

        let turns_to_ko = (safest_pokemon.get_remaining_hp() as f64 / max_damage).ceil();
        total_turns += turns_to_ko;
        opp_points += safest_pokemon.card.get_knockout_points();
    }

    total_turns
}

/// Calculate online pokemon metrics: (count of online pokemon, total energy distance to online)
/// A pokemon is "online" if it can use at least one attack
/// Applies active_factor bonus to the active pokemon (position 0)
fn calculate_online_metrics(state: &State, player: usize, active_factor: f64) -> (f64, f64) {
    let (online_count, total_distance) = state
        .enumerate_in_play_pokemon(player)
        .map(|(pos, card)| {
            let min_distance = card
                .card
                .get_attacks()
                .iter()
                .map(|atk| {
                    let missing = energy_missing(card, &atk.energy_required, state, player);
                    missing.len()
                })
                .min()
                .unwrap_or(0);

            let position_factor = if pos == 0 { active_factor } else { 1.0 };

            if min_distance == 0 {
                (position_factor, 0.0) // Online pokemon with position bonus
            } else {
                (0.0, min_distance as f64 * position_factor) // Offline pokemon with weighted distance
            }
        })
        .fold((0.0, 0.0), |(count, dist), (c, d)| (count + c, dist + d));

    (online_count, total_distance)
}

/// Calculate total pokemon value (HP * Energy) for a player
fn calculate_pokemon_value(state: &State, player: usize, active_factor: f64) -> f64 {
    state
        .enumerate_in_play_pokemon(player)
        .map(|(pos, card)| {
            let relevant_energy = get_relevant_energy(state, player, card);
            let hp_energy_product = card.get_remaining_hp() as f64 * (relevant_energy + 1.0);
            if pos == 0 {
                hp_energy_product * active_factor
            } else {
                hp_energy_product
            }
        })
        .sum()
}

/// Helper function to calculate relevant energy for a Pokemon
fn get_relevant_energy(state: &State, player: usize, card: &PlayedCard) -> f64 {
    let most_expensive_attack_cost: Vec<EnergyType> = card
        .card
        .get_attacks()
        .iter()
        .map(|atk| atk.energy_required.clone())
        .max()
        .unwrap_or_default();

    let missing = energy_missing(card, &most_expensive_attack_cost, state, player);

    let total = most_expensive_attack_cost.len() as f64;
    total - missing.len() as f64
}

/// Calculate active safety
/// Defined as remaining HP divided by knockout points
fn calculate_active_safety(state: &State, player: usize) -> f64 {
    let Some(active_pokemon) = state.maybe_get_active(player) else {
        return 0.0; // No safety if no active pokemon
    };

    let ko_points = active_pokemon.card.get_knockout_points() as f64;
    let hp = active_pokemon.get_remaining_hp() as f64;

    hp / ko_points.max(1.0)
}

/// Calculate online score for active pokemon (0.0 to 1.0)
/// Returns 1.0 if the active pokemon has enough energy to use the highest attack
/// of its highest evolution available in deck+hand
fn calculate_active_pokemon_online_score(state: &State, player: usize) -> f64 {
    let Some(active_pokemon) = state.maybe_get_active(player) else {
        return 0.0;
    };

    // Get all cards available in deck + hand
    let mut available_cards: Vec<Card> = state.decks[player].cards.to_vec();
    available_cards.extend(state.hands[player].iter().cloned());

    // Find the highest evolution available
    let highest_evolutions = get_highest_evolutions(&active_pokemon.card, &available_cards);

    // If no evolutions found, use the current card
    let target_card = if highest_evolutions.is_empty() {
        &active_pokemon.card
    } else {
        // Use the first highest evolution (they should all be same stage)
        &highest_evolutions[0]
    };

    // Get the highest attack energy cost from the target card
    let most_expensive_attack_cost: Vec<EnergyType> = target_card
        .get_attacks()
        .iter()
        .map(|atk| atk.energy_required.clone())
        .max()
        .unwrap_or_default();

    if most_expensive_attack_cost.is_empty() {
        return 1.0; // No attack requirements, fully online
    }

    // Calculate how much energy we have vs need
    let missing = energy_missing(active_pokemon, &most_expensive_attack_cost, state, player);
    let total_needed = most_expensive_attack_cost.len() as f64;
    let have = total_needed - missing.len() as f64;

    // Return ratio (0.0 to 1.0)
    (have / total_needed).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn params_json_fills_missing_fields_with_baseline() {
        let params: ValueFunctionParams =
            serde_json::from_str(r#"{"hand_size": 5.0, "bench_hp": 0.5}"#).unwrap();
        assert_eq!(
            params,
            ValueFunctionParams {
                hand_size: 5.0,
                bench_hp: 0.5,
                ..ValueFunctionParams::baseline()
            }
        );
    }

    #[test]
    fn params_json_rejects_unknown_fields() {
        assert!(serde_json::from_str::<ValueFunctionParams>(r#"{"hand_sise": 5.0}"#).is_err());
    }

    #[test]
    fn baseline_round_trips_through_json() {
        let json = serde_json::to_string(&ValueFunctionParams::baseline()).unwrap();
        let back: ValueFunctionParams = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ValueFunctionParams::baseline());
    }
}
