//! Per-game statistics for in-game event tasks ("Knock Out the opponent's Active Pokemon with a
//! Dragon-type attack", "Put 3 Basic Pokemon into play", "Do 100 damage with one attack", "Win by
//! turn 14", "Win without the opponent getting any points"): one compact record per game, written
//! as JSON by `simulate --stats-json <file>` so a caller can compute the share of games that meet
//! any threshold.

use serde::Serialize;
use std::any;
use std::collections::BTreeMap;
use uuid::Uuid;

use crate::{
    actions::{Action, SimpleAction},
    models::{Card, EnergyType},
    optimize::ParallelConfig,
    players::PlayerCode,
    simulate::print_stats,
    simulation_event_handler::{SimulationEventHandler, StatsCollector},
    state::GameOutcome,
    Deck, Simulation, State,
};

const ALL_TYPES: [EnergyType; 10] = [
    EnergyType::Grass,
    EnergyType::Fire,
    EnergyType::Water,
    EnergyType::Lightning,
    EnergyType::Psychic,
    EnergyType::Fighting,
    EnergyType::Darkness,
    EnergyType::Metal,
    EnergyType::Dragon,
    EnergyType::Colorless,
];

/// One player's side of a finished game.
#[derive(Debug, Clone, Default, Serialize)]
pub struct PlayerGameStats {
    pub points: u8,
    /// Opponent's Active Pokemon Knocked Out by this player's attacks, by attacker type (only
    /// types with at least one KO).
    pub active_kos: BTreeMap<String, u8>,
    /// Largest total damage dealt to the opponent's Pokemon by one attack.
    pub max_attack_damage: u16,
    /// Pokemon put into play: [Basic, Stage 1, Stage 2] (placed from hand or evolved).
    pub put_into_play: [u16; 3],
}

#[derive(Debug, Clone, Serialize)]
pub struct GameStats {
    /// 0 or 1; None for a tie or an unfinished game.
    pub winner: Option<usize>,
    /// Global turn count when the game ended (as in the game app: both players' turns count).
    pub turns: u8,
    pub players: [PlayerGameStats; 2],
}

/// Collects one `GameStats` per game.
#[derive(Default)]
pub struct EventStatsCollector {
    put_into_play: [[u16; 3]; 2],
    pub games: Vec<GameStats>,
}

impl SimulationEventHandler for EventStatsCollector {
    fn on_game_start(&mut self, _game_id: Uuid) {
        self.put_into_play = [[0; 3]; 2];
    }

    fn on_action(
        &mut self,
        _game_id: Uuid,
        _state_before_action: &State,
        actor: usize,
        _playable_actions: &[Action],
        action: &Action,
    ) {
        let card = match &action.action {
            SimpleAction::Place(card, _) => card,
            SimpleAction::Evolve { evolution, .. } => evolution,
            _ => return,
        };
        if let Card::Pokemon(pokemon) = card {
            let stage = (pokemon.stage as usize).min(2);
            self.put_into_play[actor][stage] += 1;
        }
    }

    fn on_game_end(&mut self, _game_id: Uuid, state: State, outcome: Option<GameOutcome>) {
        let player = |p: usize| PlayerGameStats {
            points: state.points[p],
            active_kos: ALL_TYPES
                .iter()
                .filter_map(|t| {
                    let n = state.tally.active_kos(p, *t);
                    (n > 0).then(|| (format!("{t:?}"), n))
                })
                .collect(),
            max_attack_damage: state.tally.max_attack_damage(p),
            put_into_play: self.put_into_play[p],
        };
        self.games.push(GameStats {
            winner: match outcome {
                Some(GameOutcome::Win(w)) => Some(w),
                _ => None,
            },
            turns: state.turn_count,
            players: [player(0), player(1)],
        });
    }

    fn merge(&mut self, other: &dyn SimulationEventHandler) {
        if let Some(other) = (other as &dyn any::Any).downcast_ref::<EventStatsCollector>() {
            self.games.extend(other.games.iter().cloned());
        } else {
            panic!("Attempted to merge EventStatsCollector with incompatible type");
        }
    }
}

#[derive(Debug, Serialize)]
pub struct EventStatsReport {
    pub deck_a: String,
    pub deck_b: String,
    pub players: Vec<String>,
    pub num_games: u32,
    pub seed: Option<u64>,
    pub wins: [u32; 2],
    pub ties: u32,
    pub games: Vec<GameStats>,
}

/// Runs `num_games` between the two decks and returns one record per game (in the order the
/// games were started, so a fixed seed gives the same file).
pub fn simulate_event_stats(
    deck_a_path: &str,
    deck_b_path: &str,
    player_codes: Vec<PlayerCode>,
    num_games: u32,
    seed: Option<u64>,
    parallel_config: ParallelConfig,
) -> Result<EventStatsReport, Box<dyn std::error::Error>> {
    let deck_a = Deck::from_file(deck_a_path)?;
    let deck_b = Deck::from_file(deck_b_path)?;
    let mut simulation = Simulation::new_with_decks(
        deck_a,
        deck_b,
        player_codes.clone(),
        num_games,
        seed,
        parallel_config.enabled,
        parallel_config.num_threads,
    )?
    .register::<StatsCollector>()
    .register::<EventStatsCollector>();
    simulation.run();

    if let Some(collector) = simulation.get_event_handler::<StatsCollector>() {
        print_stats(&collector.compute_stats());
    }
    let games = simulation
        .get_event_handler::<EventStatsCollector>()
        .map(|c| c.games.clone())
        .unwrap_or_default();
    let mut wins = [0, 0];
    let mut ties = 0;
    for game in &games {
        match game.winner {
            Some(w) => wins[w] += 1,
            None => ties += 1,
        }
    }
    Ok(EventStatsReport {
        deck_a: deck_a_path.to_string(),
        deck_b: deck_b_path.to_string(),
        players: player_codes.iter().map(|p| format!("{p:?}")).collect(),
        num_games,
        seed,
        wins,
        ties,
        games,
    })
}
