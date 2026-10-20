// Collection of value functions for ExpectiMiniMaxPlayer
//
// Each value function evaluates a game state from a player's perspective
// and returns a score (higher is better for that player)

use std::sync::OnceLock;

use log::trace;
use serde::{Deserialize, Serialize};

use crate::actions::expected_attack_damage;
use crate::card_logic::get_highest_evolutions;
use crate::hooks::energy_missing;
use crate::models::{Attack, Card, EnergyType, PlayedCard};
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
    /// Price attacks with the engine's expected damage (Weakness, bench scaling, Stadiums, Tools,
    /// coin flips...) instead of the printed damage, in `can_ko_active` and
    /// `turns_until_opponent_wins`. False in the baseline.
    pub real_damage: bool,
    // Step 2.2: energia, descarte, estados y banca. En 0 (o true en pokemon_value_energy) en el
    // baseline, asi que no cambian su comportamiento.
    /// Dano que cada Pokemon en juego puede hacer en su proximo ataque (ver [`reachable_damage`]),
    /// mio menos el del rival. Valua la energia por el dano que habilita (la 3a y 4a de Mega
    /// Rayquaza ex suman) y el descarte por el dano futuro que se pierde segun cuanto tarda en
    /// recargar. Tope: el HP restante mas alto del lado rival (el dano de mas no vale).
    pub energy_damage: f64,
    /// Peso de los Pokemon de la banca dentro de `energy_damage` (el activo pesa 1).
    pub energy_damage_bench: f64,
    /// Si es false, `pokemon_value` es solo HP (sin el factor de energia hasta el costo del ataque
    /// mas caro), para que la energia la value `energy_damage` y descartarla no cueste un fijo.
    pub pokemon_value_energy: bool,
    /// Dano esperado que el activo rival deja de hacer por estar dormido (moneda: 50%),
    /// paralizado (100%) o confundido (50%), menos el mismo calculo para mi activo.
    pub status_damage: f64,
    /// Pokemon de la banca que ya pueden atacar (1) o estan a una energia (0.5), mios menos los
    /// del rival: el bono de "online" de `active_pokemon_online_score` para la banca.
    pub bench_online: f64,
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
            real_damage: false,
            energy_damage: 0.0,
            energy_damage_bench: 0.5,
            pokemon_value_energy: true,
            status_damage: 0.0,
            bench_online: 0.0,
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
    let mut pricer = AttackPricer::new(state, params.real_damage);
    let (my, opp) = (
        extract_features(state, myself, params, &mut pricer),
        extract_features(state, opponent, params, &mut pricer),
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
        + (my.attached_energy - opp.attached_energy) * params.attached_energy
        + (my.energy_damage - opp.energy_damage) * params.energy_damage
        + (my.status_damage - opp.status_damage) * params.status_damage
        + (my.bench_online - opp.bench_online) * params.bench_online;
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
    energy_damage: f64,
    status_damage: f64,
    bench_online: f64,
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
fn extract_features(
    state: &State,
    player: usize,
    params: &ValueFunctionParams,
    pricer: &mut AttackPricer,
) -> Features {
    let active_factor = 1.0;
    let points = state.points[player] as f64;
    let pokemon_value = if_used(params.pokemon_value, || {
        calculate_pokemon_value(state, player, active_factor, params.pokemon_value_energy)
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
        calculate_turns_until_opponent_wins(state, player, pricer)
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
        can_ko_opponent_active(state, player, pricer)
    });
    let attached_energy = state
        .enumerate_in_play_pokemon(player)
        .map(|(_, card)| card.attached_energy.len() as f64)
        .sum();
    let energy_damage = if_used(params.energy_damage, || {
        calculate_energy_damage(state, player, params.energy_damage_bench, pricer)
    });
    let status_damage = if_used(params.status_damage, || {
        calculate_status_damage(state, player, pricer)
    });
    let bench_online = if_used(params.bench_online, || {
        calculate_bench_online(state, player)
    });

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
        energy_damage,
        status_damage,
        bench_online,
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

/// Prices the attacks of one player's Pokémon against the other player's Pokémon: the printed
/// damage, or with `real_damage` the engine's expected damage ([`expected_attack_damage`]).
///
/// The engine forecasts attacks of the current player's Active against the opponent's Active, so
/// any other pairing is priced on a scratch copy of the state (cloned once per evaluation, on
/// first use, and shared by both players' features) with the attacker as current player and both
/// Pokémon swapped into the Active Spots: a benched Pokémon is priced as if it were promoted,
/// which is when it would attack or be attacked.
struct AttackPricer<'a> {
    state: &'a State,
    real_damage: bool,
    scratch: Option<State>,
    reach: [[Option<f64>; 4]; 2],
}

impl<'a> AttackPricer<'a> {
    fn new(state: &'a State, real_damage: bool) -> Self {
        Self {
            state,
            real_damage,
            scratch: None,
            reach: [[None; 4]; 2],
        }
    }

    /// Damage of `attack` of `attacker`'s Pokémon at `attacker_idx` against the other player's
    /// Pokémon at `defender_idx`. Falls back to the printed damage when the engine can't price it.
    fn damage(
        &mut self,
        attacker: usize,
        attacker_idx: usize,
        attack: &Attack,
        defender_idx: usize,
    ) -> f64 {
        self.damage_with_extra(attacker, attacker_idx, attack, defender_idx, &[])
    }

    /// Como [`Self::damage`], pero con `extra` energia adherida de mas al atacante (para valuar
    /// la proxima energia: Mega Burst hace 50 mas por cada una).
    fn damage_with_extra(
        &mut self,
        attacker: usize,
        attacker_idx: usize,
        attack: &Attack,
        defender_idx: usize,
        extra: &[EnergyType],
    ) -> f64 {
        let printed = attack.fixed_damage as f64;
        if !self.real_damage {
            return printed;
        }
        let state = self.state;
        if extra.is_empty()
            && attacker_idx == 0
            && defender_idx == 0
            && state.current_player == attacker
        {
            return expected_attack_damage(state, attack).unwrap_or(printed);
        }
        let defender = (attacker + 1) % 2;
        let scratch = self.scratch.get_or_insert_with(|| state.clone());
        let attached_len = match scratch.in_play_pokemon[attacker][attacker_idx].as_mut() {
            Some(pokemon) => {
                let len = pokemon.attached_energy.len();
                pokemon.attached_energy.extend_from_slice(extra);
                len
            }
            None => return printed,
        };
        scratch.current_player = attacker;
        scratch.in_play_pokemon[attacker].swap(0, attacker_idx);
        scratch.in_play_pokemon[defender].swap(0, defender_idx);
        let damage = expected_attack_damage(scratch, attack).unwrap_or(printed);
        scratch.in_play_pokemon[defender].swap(0, defender_idx);
        scratch.in_play_pokemon[attacker].swap(0, attacker_idx);
        if let Some(pokemon) = scratch.in_play_pokemon[attacker][attacker_idx].as_mut() {
            pokemon.attached_energy.truncate(attached_len);
        }
        damage
    }

    /// [`reachable_damage`] de cada Pokemon en juego, calculado una vez por evaluacion: lo usan
    /// `energy_damage` de los dos jugadores y `status_damage`.
    fn reachable(&mut self, player: usize, idx: usize) -> f64 {
        if let Some(damage) = self.reach[player][idx] {
            return damage;
        }
        let damage = reachable_damage(self, player, idx);
        self.reach[player][idx] = Some(damage);
        damage
    }
}

/// Dano que el Pokemon de `player` en `idx` puede hacer en su proximo ataque contra el activo
/// rival (un Pokemon de la banca, como si estuviera de activo): el mejor ataque que puede pagar con
/// una energia mas (la que se adhiere el proximo turno), o la mitad del mejor con dos mas (tarda dos
/// turnos). La energia del tipo mas conveniente entre los del mazo. Asi la energia vale el dano que
/// habilita aunque pase del costo del ataque, y un descarte cuesta el dano que se pierde segun
/// cuantos turnos tarda en volver a cargar (descartar 1 de 3 casi no cuesta; todo, bastante).
fn reachable_damage(pricer: &mut AttackPricer, player: usize, idx: usize) -> f64 {
    let state = pricer.state;
    let Some(pokemon) = state.in_play_pokemon[player][idx].as_ref() else {
        return 0.0;
    };
    let mut types: Vec<EnergyType> = state.decks[player].energy_types().to_vec();
    types.sort();
    types.dedup();
    let one: Vec<Vec<EnergyType>> = types.iter().map(|t| vec![*t]).collect();
    let two: Vec<Vec<EnergyType>> = types
        .iter()
        .enumerate()
        .flat_map(|(i, a)| types[i..].iter().map(move |b| vec![*a, *b]))
        .collect();
    let attacks = pokemon.card.get_attacks();
    let missing: Vec<Vec<EnergyType>> = attacks
        .iter()
        .map(|atk| energy_missing(pokemon, &atk.energy_required, state, player))
        .collect();
    // Mejor dano con alguna de las combinaciones de energia extra
    let mut best = |combos: &[Vec<EnergyType>]| -> f64 {
        let mut best: f64 = 0.0;
        for (atk, missing) in attacks.iter().zip(&missing) {
            let payable = combos.iter().filter(|extra| covers(missing, extra));
            if atk.effect.is_none() {
                // Sin efecto el dano no depende de la energia: se calcula una vez
                if payable.count() > 0 {
                    best = best.max(pricer.damage(player, idx, atk, 0));
                }
            } else {
                for extra in payable {
                    best = best.max(pricer.damage_with_extra(player, idx, atk, 0, extra));
                }
            }
        }
        best
    };
    if types.is_empty() {
        return best(&[vec![]]);
    }
    best(&one).max(best(&two) / 2.0)
}

/// Si la energia `extra` alcanza para pagar `missing` (lo que devuelve `energy_missing`: primero
/// los tipos, despues los incoloros con lo que sobre).
fn covers(missing: &[EnergyType], extra: &[EnergyType]) -> bool {
    if missing.len() > extra.len() {
        return false;
    }
    let mut left = extra.to_vec();
    for energy in missing.iter().filter(|e| **e != EnergyType::Colorless) {
        match left.iter().position(|e| e == energy) {
            Some(i) => {
                left.remove(i);
            }
            None => return false,
        }
    }
    let colorless = missing
        .iter()
        .filter(|e| **e == EnergyType::Colorless)
        .count();
    colorless <= left.len()
}

/// Suma de [`reachable_damage`] de los Pokemon en juego del jugador (la banca pesa
/// `bench_factor`), cada uno con tope en el HP restante mas alto del lado rival.
fn calculate_energy_damage(
    state: &State,
    player: usize,
    bench_factor: f64,
    pricer: &mut AttackPricer,
) -> f64 {
    let opponent = (player + 1) % 2;
    let cap = state
        .enumerate_in_play_pokemon(opponent)
        .map(|(_, card)| card.get_remaining_hp() as f64)
        .fold(0.0, f64::max);
    let in_play: Vec<usize> = state
        .enumerate_in_play_pokemon(player)
        .map(|(idx, _)| idx)
        .collect();
    in_play
        .into_iter()
        .map(|idx| {
            let factor = if idx == 0 { 1.0 } else { bench_factor };
            factor * pricer.reachable(player, idx).min(cap)
        })
        .sum()
}

/// Dano esperado que el activo rival pierde en su proximo turno por sus estados: dormido 50% (la
/// moneda del chequeo lo despierta), paralizado 100%, confundido 50% (la moneda al atacar). Con
/// tope en el HP restante de mi activo.
fn calculate_status_damage(state: &State, player: usize, pricer: &mut AttackPricer) -> f64 {
    let opponent = (player + 1) % 2;
    let (Some(mine), Some(theirs)) = (
        state.maybe_get_active(player),
        state.maybe_get_active(opponent),
    ) else {
        return 0.0;
    };
    let lost = if theirs.is_paralyzed() {
        1.0
    } else {
        let mut attacks = 1.0;
        if theirs.is_asleep() {
            attacks *= 0.5;
        }
        if theirs.is_confused() {
            attacks *= 0.5;
        }
        1.0 - attacks
    };
    if lost == 0.0 {
        return 0.0;
    }
    // El dano esperado del motor ya viene a la mitad por la confusion: se compara contra el dano
    // sin ella
    let mut damage = pricer.reachable(opponent, 0);
    if theirs.is_confused() && pricer.real_damage {
        damage *= 2.0;
    }
    lost * damage.min(mine.get_remaining_hp() as f64)
}

/// Pokemon de la banca que ya pueden pagar algun ataque (1) o estan a una energia (0.5)
fn calculate_bench_online(state: &State, player: usize) -> f64 {
    state
        .enumerate_bench_pokemon(player)
        .map(|(_, card)| {
            let min_missing = card
                .card
                .get_attacks()
                .iter()
                .map(|atk| energy_missing(card, &atk.energy_required, state, player).len())
                .min();
            match min_missing {
                Some(0) => 1.0,
                Some(1) => 0.5,
                _ => 0.0,
            }
        })
        .sum()
}

/// 1.0 if the player's active has an attack it can already pay for whose damage reaches the
/// opponent's active remaining HP. Printed damage only, or with `real_damage` the engine's
/// expected damage (with weakness and damage modifiers).
fn can_ko_opponent_active(state: &State, player: usize, pricer: &mut AttackPricer) -> f64 {
    let opponent = (player + 1) % 2;
    let (Some(mine), Some(theirs)) = (
        state.maybe_get_active(player),
        state.maybe_get_active(opponent),
    ) else {
        return 0.0;
    };
    let target_hp = theirs.get_remaining_hp() as f64;
    let can_ko = mine.card.get_attacks().iter().any(|atk| {
        energy_missing(mine, &atk.energy_required, state, player).is_empty() && {
            let damage = pricer.damage(player, 0, atk, 0);
            damage > 0.0 && damage >= target_hp
        }
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
/// Uses opponent's active damage and simulates KOs until opponent reaches 3 points.
/// With `real_damage`, attacks are priced by the engine against each target (see
/// [`AttackPricer`]) instead of by their printed damage.
fn calculate_turns_until_opponent_wins(
    state: &State,
    player: usize,
    pricer: &mut AttackPricer,
) -> f64 {
    let opponent = (player + 1) % 2;
    let real_damage = pricer.real_damage;

    // Find the closest attack to dealing damage to my active: least missing energy, then most
    // damage (first one on ties). Attacks are priced by increasing missing energy (stable, so ties
    // keep board order) and only until the first damaging group, since pricing can be costly.
    let mut candidates: Vec<(usize, Attack, usize)> = state
        .enumerate_in_play_pokemon(opponent)
        .flat_map(|(idx, pokemon)| {
            let attacks = pokemon.card.get_attacks().into_iter();
            // Printed damage 0 is never a threat without real_damage
            let attacks = attacks.filter(move |atk| real_damage || atk.fixed_damage > 0);
            attacks.map(move |atk| {
                let missing = energy_missing(pokemon, &atk.energy_required, state, opponent);
                (idx, atk, missing.len())
            })
        })
        .collect();
    candidates.sort_by_key(|(_, _, missing)| *missing);
    let mut best_threat: Option<(usize, Attack, f64, usize)> = None;
    for (idx, atk, missing) in candidates {
        if best_threat
            .as_ref()
            .is_some_and(|(_, _, _, best_missing)| missing > *best_missing)
        {
            break;
        }
        let damage = pricer.damage(opponent, idx, &atk, 0);
        if damage <= 0.0 {
            continue; // Only consider attacks that deal damage
        }
        if best_threat
            .as_ref()
            .is_none_or(|(_, _, best_damage, _)| damage > *best_damage)
        {
            best_threat = Some((idx, atk, damage, missing));
        }
    }
    let Some((threat_idx, threat_attack, max_damage, missing_energy)) = best_threat else {
        return 30.0; // No pokemon can deal damage
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
    let mut bench_damage: [Option<f64>; 4] = [None; 4];
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

        let Some((bench_idx, safest_pokemon)) = safest_bench else {
            break; // No more bench pokemon
        };

        let damage = if real_damage {
            // The loop can pick the same Pokémon again, so price each one once
            *bench_damage[bench_idx].get_or_insert_with(|| {
                pricer.damage(opponent, threat_idx, &threat_attack, bench_idx)
            })
        } else {
            max_damage
        };
        if damage <= 0.0 {
            return 30.0; // The threat can't damage it (e.g. Safeguard against an ex)
        }
        let turns_to_ko = (safest_pokemon.get_remaining_hp() as f64 / damage).ceil();
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
fn calculate_pokemon_value(
    state: &State,
    player: usize,
    active_factor: f64,
    with_energy: bool,
) -> f64 {
    state
        .enumerate_in_play_pokemon(player)
        .map(|(pos, card)| {
            let relevant_energy = if with_energy {
                get_relevant_energy(state, player, card)
            } else {
                0.0
            };
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

    use crate::card_ids::CardId;
    use crate::database::get_card_by_enum;
    use crate::test_support::load_test_deck;
    use crate::players::RandomPlayer;
    use crate::Game;
    use strum::IntoEnumIterator;

    fn board(current_player: usize, mine: Vec<PlayedCard>, theirs: Vec<PlayedCard>) -> State {
        let mut state = State::default();
        state.current_player = current_player;
        for (idx, card) in mine.into_iter().enumerate() {
            state.in_play_pokemon[0][idx] = Some(card);
        }
        for (idx, card) in theirs.into_iter().enumerate() {
            state.in_play_pokemon[1][idx] = Some(card);
        }
        state
    }

    fn first_attack(id: CardId) -> Attack {
        get_card_by_enum(id).get_attacks()[0].clone()
    }

    fn can_ko(state: &State, player: usize, real_damage: bool) -> f64 {
        can_ko_opponent_active(state, player, &mut AttackPricer::new(state, real_damage))
    }

    fn turns(state: &State, player: usize, real_damage: bool) -> f64 {
        let mut pricer = AttackPricer::new(state, real_damage);
        calculate_turns_until_opponent_wins(state, player, &mut pricer)
    }

    #[test]
    fn real_damage_adds_weakness() {
        // Pikachu's Gnaw (20, Lightning) against Squirtle (weak to Lightning) and Pikachu (not)
        let gnaw = first_attack(CardId::A1094Pikachu);
        let state = board(
            0,
            vec![PlayedCard::from_id(CardId::A1094Pikachu)],
            vec![PlayedCard::from_id(CardId::A1053Squirtle)],
        );
        assert_eq!(expected_attack_damage(&state, &gnaw), Some(40.0));
        let state = board(
            0,
            vec![PlayedCard::from_id(CardId::A1094Pikachu)],
            vec![PlayedCard::from_id(CardId::A1094Pikachu)],
        );
        assert_eq!(expected_attack_damage(&state, &gnaw), Some(20.0));
    }

    #[test]
    fn real_damage_prices_the_non_current_player_benched_attacker() {
        // It's player 0's turn; player 1's benched Pikachu would hit my Squirtle for 40
        let gnaw = first_attack(CardId::A1094Pikachu);
        let state = board(
            0,
            vec![
                PlayedCard::from_id(CardId::A1053Squirtle),
                PlayedCard::from_id(CardId::A1094Pikachu),
            ],
            vec![
                PlayedCard::from_id(CardId::A1053Squirtle),
                PlayedCard::from_id(CardId::A1094Pikachu),
            ],
        );
        let mut pricer = AttackPricer::new(&state, true);
        assert_eq!(pricer.damage(1, 1, &gnaw, 0), 40.0);
        // The scratch copy is restored between calls: against my benched Pikachu, no weakness
        assert_eq!(pricer.damage(1, 1, &gnaw, 1), 20.0);
        assert_eq!(pricer.damage(1, 1, &gnaw, 0), 40.0);
        // The same scratch copy also prices the other side
        assert_eq!(pricer.damage(0, 1, &gnaw, 0), 40.0);
        assert_eq!(AttackPricer::new(&state, false).damage(1, 1, &gnaw, 0), 20.0);
    }

    #[test]
    fn real_damage_scales_with_full_bench() {
        // Pikachu ex's Circle Circuit: 30 for each benched Lightning Pokémon (printed 30)
        let circuit = first_attack(CardId::A1096PikachuEx);
        let attacker = || {
            PlayedCard::from_id(CardId::A1096PikachuEx)
                .with_energy(vec![EnergyType::Lightning, EnergyType::Lightning])
        };
        let pikachu = || PlayedCard::from_id(CardId::A1094Pikachu);
        let full_bench = board(
            0,
            vec![attacker(), pikachu(), pikachu(), pikachu()],
            vec![pikachu()],
        );
        let empty_bench = board(0, vec![attacker()], vec![pikachu()]);
        assert_eq!(expected_attack_damage(&full_bench, &circuit), Some(90.0));
        assert_eq!(expected_attack_damage(&empty_bench, &circuit), Some(0.0));

        // Pikachu has 60 HP: only the real damage with a full bench knocks it out
        assert_eq!(can_ko(&full_bench, 0, true), 1.0);
        assert_eq!(can_ko(&full_bench, 0, false), 0.0);
        assert_eq!(can_ko(&empty_bench, 0, true), 0.0);
    }

    #[test]
    fn real_damage_shortens_turns_until_opponent_wins_with_weakness() {
        // Opponent's Pikachu (Gnaw 20, paid) against my 60 HP Squirtle, weak to Lightning
        let state = board(
            0,
            vec![PlayedCard::from_id(CardId::A1053Squirtle)],
            vec![PlayedCard::from_id(CardId::A1094Pikachu)
                .with_energy(vec![EnergyType::Lightning])],
        );
        assert_eq!(turns(&state, 0, false), 3.0);
        assert_eq!(turns(&state, 0, true), 2.0);
    }

    #[test]
    fn expected_attack_damage_never_panics_for_any_card() {
        // Every implemented attack in the database, unpaid, against a plain defender
        for id in CardId::iter() {
            let card = get_card_by_enum(id);
            let Card::Pokemon(_) = card else { continue };
            let state = board(
                0,
                vec![PlayedCard::from_id(id)],
                vec![PlayedCard::from_id(CardId::A1094Pikachu)],
            );
            for attack in card.get_attacks() {
                if let Some(damage) = expected_attack_damage(&state, &attack) {
                    assert!(damage.is_finite() && damage >= 0.0, "{id:?} {attack:?}");
                }
            }
        }
    }

    // Step 2.2

    use crate::models::StatusCondition;

    fn with_deck_energy(mut state: State, mine: Vec<EnergyType>, theirs: Vec<EnergyType>) -> State {
        state.decks[0].energy_types = mine;
        state.decks[1].energy_types = theirs;
        state
    }

    fn rayquaza(energy: Vec<EnergyType>) -> PlayedCard {
        PlayedCard::from_id(CardId::B4120MegaRayquazaEx).with_energy(energy)
    }

    /// Mega Rayquaza ex contra un Lucario ex de 190 HP, con mazo de Fuego y Rayo
    fn rayquaza_board(energy: Vec<EnergyType>, defender: PlayedCard) -> State {
        use EnergyType::{Fire, Lightning};
        with_deck_energy(
            board(0, vec![rayquaza(energy)], vec![defender]),
            vec![Fire, Lightning],
            vec![EnergyType::Fighting],
        )
    }

    fn reach(state: &State, player: usize, idx: usize) -> f64 {
        reachable_damage(&mut AttackPricer::new(state, true), player, idx)
    }

    fn energy_damage(state: &State, player: usize) -> f64 {
        calculate_energy_damage(state, player, 0.5, &mut AttackPricer::new(state, true))
    }

    #[test]
    fn energy_is_valued_by_the_damage_it_enables_past_the_attack_cost() {
        use EnergyType::{Fire, Lightning};
        let lucario = || PlayedCard::from_id(CardId::B3081MegaLucarioEx);
        // Mega Burst cuesta [R][L] y hace 50 por cada [R]/[L]: la proxima energia suma 50
        let two = rayquaza_board(vec![Fire, Lightning], lucario());
        let three = rayquaza_board(vec![Fire, Lightning, Lightning], lucario());
        assert_eq!(reach(&two, 0, 0), 150.0);
        assert_eq!(reach(&three, 0, 0), 200.0);
        // Con tope en los 190 HP de Lucario: el dano de mas no vale
        assert_eq!(energy_damage(&three, 0), 190.0);
        // El valor viejo no distingue la 3a energia (solo cuenta hasta el costo del ataque)
        assert_eq!(
            calculate_pokemon_value(&two, 0, 1.0, true),
            calculate_pokemon_value(&three, 0, 1.0, true)
        );
        // Sin el factor de energia, pokemon_value es solo HP
        assert_eq!(calculate_pokemon_value(&three, 0, 1.0, false), 180.0);
    }

    #[test]
    fn unpaid_attacks_count_from_the_energy_that_completes_them() {
        use EnergyType::{Fire, Lightning};
        let lucario = || PlayedCard::from_id(CardId::B3081MegaLucarioEx);
        // Sin energia, Mega Burst tarda dos turnos: la mitad de 100
        assert_eq!(reach(&rayquaza_board(vec![], lucario()), 0, 0), 50.0);
        // Con una, la proxima lo completa
        assert_eq!(reach(&rayquaza_board(vec![Fire], lucario()), 0, 0), 100.0);
        // Con energia del tipo equivocado en el mazo no se completa
        let state = with_deck_energy(
            board(0, vec![rayquaza(vec![Fire])], vec![lucario()]),
            vec![Fire],
            vec![],
        );
        assert_eq!(reach(&state, 0, 0), 0.0);
        // Un Pokemon de la banca se valua como si fuera el activo, y pesa la mitad
        let mut state = rayquaza_board(vec![Fire, Lightning], lucario());
        state.in_play_pokemon[0][1] = Some(rayquaza(vec![Fire, Lightning]));
        assert_eq!(reach(&state, 0, 1), 150.0);
        assert_eq!(energy_damage(&state, 0), 150.0 + 0.5 * 150.0);
    }

    #[test]
    fn discarding_energy_costs_the_damage_lost_until_it_recharges() {
        use EnergyType::{Fire, Lightning};
        let lucario = || PlayedCard::from_id(CardId::B3081MegaLucarioEx);
        let full = rayquaza_board(vec![Fire, Fire, Lightning, Lightning], lucario());
        let discarded = rayquaza_board(vec![], lucario());
        // Despues de Mega Burst con 4 energias: de 190 (tope) a 50 (dos turnos para recargar)
        assert_eq!(energy_damage(&full, 0), 190.0);
        assert_eq!(energy_damage(&discarded, 0), 50.0);
        // Si el rival queda con un Pokemon a 30 HP, descargar no cuesta nada
        let weak = || lucario().with_remaining_hp(30);
        let full = rayquaza_board(vec![Fire, Fire, Lightning, Lightning], weak());
        let discarded = rayquaza_board(vec![], weak());
        assert_eq!(energy_damage(&full, 0), 30.0);
        assert_eq!(energy_damage(&discarded, 0), 30.0);
    }

    #[test]
    fn status_damage_is_the_damage_the_opponent_loses() {
        // El Pikachu rival (Gnaw 20, se paga con la proxima energia) contra mi Squirtle, debil a
        // Rayo: 40 por turno
        let with_status = |status: Option<StatusCondition>| {
            let mut pikachu = PlayedCard::from_id(CardId::A1094Pikachu);
            if let Some(status) = status {
                pikachu.set_status_raw(status);
            }
            with_deck_energy(
                board(0, vec![PlayedCard::from_id(CardId::A1053Squirtle)], vec![pikachu]),
                vec![EnergyType::Water],
                vec![EnergyType::Lightning],
            )
        };
        let status = |state: &State| {
            calculate_status_damage(state, 0, &mut AttackPricer::new(state, true))
        };
        assert_eq!(status(&with_status(None)), 0.0);
        assert_eq!(status(&with_status(Some(StatusCondition::Asleep))), 20.0);
        assert_eq!(status(&with_status(Some(StatusCondition::Paralyzed))), 40.0);
        assert_eq!(status(&with_status(Some(StatusCondition::Confused))), 20.0);
        // Veneno no cuenta aca (es dano, no un turno perdido)
        assert_eq!(status(&with_status(Some(StatusCondition::Poisoned))), 0.0);
        // Del lado del rival, mi Squirtle no tiene estados
        let state = with_status(Some(StatusCondition::Asleep));
        assert_eq!(
            calculate_status_damage(&state, 1, &mut AttackPricer::new(&state, true)),
            0.0
        );
    }

    #[test]
    fn bench_online_counts_benched_attackers_and_one_energy_away() {
        let state = board(
            0,
            vec![
                PlayedCard::from_id(CardId::A1094Pikachu).with_energy(vec![EnergyType::Lightning]),
                PlayedCard::from_id(CardId::A1094Pikachu).with_energy(vec![EnergyType::Lightning]),
                PlayedCard::from_id(CardId::A1094Pikachu),
                PlayedCard::from_id(CardId::A1096PikachuEx),
            ],
            vec![PlayedCard::from_id(CardId::A1094Pikachu)],
        );
        // El activo no cuenta (lo mira active_pokemon_online_score); Pikachu ex esta a dos
        assert_eq!(calculate_bench_online(&state, 0), 1.5);
        assert_eq!(calculate_bench_online(&state, 1), 0.0);
    }

    #[test]
    fn covers_matches_typed_energy_first() {
        use EnergyType::{Colorless, Fire, Lightning};
        assert!(covers(&[], &[]));
        assert!(covers(&[Fire], &[Fire]));
        assert!(!covers(&[Fire], &[Lightning]));
        assert!(covers(&[Colorless], &[Lightning]));
        assert!(covers(&[Fire, Colorless], &[Lightning, Fire]));
        assert!(!covers(&[Fire, Fire], &[Fire, Lightning]));
        assert!(!covers(&[Fire, Colorless], &[Fire]));
    }

    /// `calculate_turns_until_opponent_wins` before `real_damage` existed, verbatim
    fn legacy_turns_until_opponent_wins(state: &State, player: usize) -> f64 {
        let opponent = (player + 1) % 2;
        let best_threat = state
            .enumerate_in_play_pokemon(opponent)
            .filter_map(|(_, pokemon)| {
                pokemon
                    .card
                    .get_attacks()
                    .iter()
                    .filter(|atk| atk.fixed_damage > 0)
                    .map(|atk| {
                        let missing =
                            energy_missing(pokemon, &atk.energy_required, state, opponent);
                        (atk.fixed_damage, missing.len())
                    })
                    .min_by_key(|(damage, missing)| (*missing, u32::MAX - damage))
            })
            .min_by_key(|(damage, missing)| (*missing, u32::MAX - damage));
        let (max_damage, missing_energy) = match best_threat {
            Some((damage, missing)) => (damage as f64, missing),
            None => return 30.0,
        };
        let mut total_turns = 0.0;
        let mut opp_points = state.points[opponent];
        total_turns += missing_energy as f64;
        if let Some(my_active) = state.maybe_get_active(player) {
            total_turns += (my_active.get_remaining_hp() as f64 / max_damage).ceil();
            opp_points += my_active.card.get_knockout_points();
        }
        while opp_points < 3 {
            let safest_bench = state
                .enumerate_bench_pokemon(player)
                .max_by_key(|(_, card)| {
                    if opp_points == 2 {
                        card.get_remaining_hp()
                    } else {
                        let ko_points = card.card.get_knockout_points().max(1) as u32;
                        card.get_remaining_hp() * 1000 / ko_points
                    }
                });
            let Some((_, safest_pokemon)) = safest_bench else {
                break;
            };
            total_turns += (safest_pokemon.get_remaining_hp() as f64 / max_damage).ceil();
            opp_points += safest_pokemon.card.get_knockout_points();
        }
        total_turns
    }

    /// `can_ko_opponent_active` before `real_damage` existed, verbatim
    fn legacy_can_ko_opponent_active(state: &State, player: usize) -> f64 {
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

    #[test]
    fn real_damage_false_matches_the_previous_value_function() {
        // Every state of random games between several decks, from both players' perspective
        let decks = [
            "altaria.txt",
            "mewtwoex.txt",
            "benchmark-electric.txt",
            "giratina-darkrai.txt",
            "mega-absol-hydreigon.txt",
            "suicune-greninja.txt",
            "blastoiseex.txt",
            "venusaur-exeggutor.txt",
        ];
        let real = ValueFunctionParams {
            real_damage: true,
            can_ko_active: 125.0,
            ..ValueFunctionParams::baseline()
        };
        let step_2_2 = ValueFunctionParams {
            energy_damage: 2.0,
            pokemon_value_energy: false,
            status_damage: 1.0,
            bench_online: 100.0,
            ..real
        };
        // Los factores de las features nuevas no cambian nada con su coeficiente en 0
        let baseline_other_factor = ValueFunctionParams {
            energy_damage_bench: 0.9,
            ..ValueFunctionParams::baseline()
        };
        let mut states = 0;
        for (i, deck_a) in decks.iter().enumerate() {
            let deck_b = decks[(i + 1) % decks.len()];
            for seed in 0..3 {
                let players: Vec<Box<dyn crate::players::Player>> = vec![
                    Box::new(RandomPlayer {
                        deck: load_test_deck(deck_a),
                    }),
                    Box::new(RandomPlayer {
                        deck: load_test_deck(deck_b),
                    }),
                ];
                let mut game = Game::new(players, seed);
                while !game.is_game_over() {
                    game.play_tick();
                    let state = game.get_state_clone();
                    for player in 0..2 {
                        assert_eq!(
                            turns(&state, player, false),
                            legacy_turns_until_opponent_wins(&state, player)
                        );
                        assert_eq!(
                            can_ko(&state, player, false),
                            legacy_can_ko_opponent_active(&state, player)
                        );
                        // The real-damage path runs on every reachable state without panicking
                        assert!(parametric_value_function(&state, player, &real).is_finite());
                        assert!(parametric_value_function(&state, player, &step_2_2).is_finite());
                        assert_eq!(
                            baseline_value_function(&state, player),
                            parametric_value_function(&state, player, &baseline_other_factor)
                        );
                    }
                    states += 1;
                }
            }
        }
        assert!(states > 100, "only {states} states");
    }
}
