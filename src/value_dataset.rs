//! `simulate --value-dataset <archivo.jsonl>` (paso 3.5 de tcgpocket): por cada posicion en la que
//! un jugador decide (mas de una accion legal), el vector de features de la funcion de valor de p2
//! (`players::own_learned::features`) desde el punto de vista de quien decide, etiquetado al terminar
//! la partida con su resultado: 1 si gano, 0 si perdio, 0.5 empate o partida cortada.
//!
//! Formato JSON lines. La primera linea es el encabezado (`header: true`): nombres de las features y
//! el coeficiente de cada una (`seccion.campo` de `LearnedParams`), los coeficientes de
//! `DECKGYM_VALUE_PARAMS` y `DECKGYM_PREP_PARAMS` con que se calcularon (los no lineales, como
//! `turn_discount` o `real_damage`, definen las features), los mazos, jugadores, partidas y semilla.
//! Despues una linea por posicion: `g` (partida, en el orden de las semillas), `p` (quien decide,
//! 0 = mazo A), `t` (turno), `e` (1 si la accion elegida fue pasar el turno), `y` (etiqueta) y `f`
//! (features, en el orden del encabezado).
//!
//! El colector solo lee el estado: no toca el rng de la partida, asi que con la misma semilla las
//! partidas salen iguales con y sin volcado (lo prueba `dataset_does_not_change_games`).

use std::any;
use std::io::Write;

use serde::Serialize;
use serde_json::json;
use uuid::Uuid;

use crate::actions::{Action, SimpleAction};
use crate::players::own_learned::{features, unit_params, FEATURES};
use crate::players::own_value::PrepParams;
use crate::players::{PlayerCode, ValueFunctionParams};
use crate::optimize::ParallelConfig;
use crate::simulate::{print_stats, Simulation};
use crate::simulation_event_handler::{GameStartMetadata, SimulationEventHandler, StatsCollector};
use crate::state::GameOutcome;
use crate::{Deck, State};

/// Una posicion de decision
#[derive(Debug, Clone, Serialize)]
pub struct Position {
    pub p: usize,
    pub t: u8,
    pub e: u8,
    pub f: Vec<f64>,
}

/// Las posiciones de una partida y su resultado
#[derive(Debug, Clone)]
pub struct GamePositions {
    pub seed: u64,
    pub positions: Vec<Position>,
    pub outcome: Option<GameOutcome>,
}

pub struct ValueDatasetCollector {
    unit: (ValueFunctionParams, PrepParams),
    current: Option<GamePositions>,
    pub games: Vec<GamePositions>,
}

impl Default for ValueDatasetCollector {
    fn default() -> Self {
        Self {
            unit: unit_params(ValueFunctionParams::loaded(), PrepParams::loaded()),
            current: None,
            games: Vec::new(),
        }
    }
}

impl SimulationEventHandler for ValueDatasetCollector {
    fn on_game_start_with_metadata(&mut self, _game_id: Uuid, metadata: &GameStartMetadata) {
        self.current = Some(GamePositions {
            seed: metadata.seed,
            positions: Vec::new(),
            outcome: None,
        });
    }

    fn on_action(
        &mut self,
        _game_id: Uuid,
        state_before_action: &State,
        actor: usize,
        playable_actions: &[Action],
        action: &Action,
    ) {
        if playable_actions.len() < 2 {
            return;
        }
        let Some(game) = self.current.as_mut() else {
            return;
        };
        let f = features(state_before_action, actor, &self.unit.0, &self.unit.1);
        game.positions.push(Position {
            p: actor,
            t: state_before_action.turn_count,
            e: matches!(action.action, SimpleAction::EndTurn) as u8,
            f,
        });
    }

    fn on_game_end(&mut self, _game_id: Uuid, _state: State, outcome: Option<GameOutcome>) {
        if let Some(mut game) = self.current.take() {
            game.outcome = outcome;
            self.games.push(game);
        }
    }

    fn merge(&mut self, other: &dyn SimulationEventHandler) {
        if let Some(other) = (other as &dyn any::Any).downcast_ref::<ValueDatasetCollector>() {
            self.games.extend(other.games.iter().cloned());
        } else {
            panic!("Attempted to merge ValueDatasetCollector with incompatible type");
        }
    }
}

/// Etiqueta de una posicion de `player`
fn label(outcome: Option<GameOutcome>, player: usize) -> f64 {
    match outcome {
        Some(GameOutcome::Win(w)) if w == player => 1.0,
        Some(GameOutcome::Win(_)) => 0.0,
        _ => 0.5,
    }
}

/// Redondea a 6 cifras significativas para que el archivo no crezca de mas
fn compact(x: f64) -> f64 {
    if x == 0.0 || !x.is_finite() {
        return 0.0;
    }
    let digits = 6 - (x.abs().log10().floor() as i32) - 1;
    let scale = 10f64.powi(digits.clamp(-10, 12));
    (x * scale).round() / scale
}

/// Escribe el encabezado y las posiciones de `games` en `out`
pub fn write_dataset(
    out: &mut impl Write,
    games: &[GamePositions],
    header: serde_json::Value,
) -> std::io::Result<()> {
    writeln!(out, "{}", header)?;
    for (g, game) in games.iter().enumerate() {
        for pos in &game.positions {
            let f: Vec<f64> = pos.f.iter().map(|x| compact(*x)).collect();
            let line = json!({
                "g": g,
                "p": pos.p,
                "t": pos.t,
                "e": pos.e,
                "y": label(game.outcome, pos.p),
                "f": f,
            });
            writeln!(out, "{}", line)?;
        }
    }
    Ok(())
}

/// Corre la simulacion con el colector y escribe el dataset en `out_path`
pub fn simulate_value_dataset(
    deck_a_path: &str,
    deck_b_path: &str,
    player_codes: Vec<PlayerCode>,
    num_games: u32,
    seed: Option<u64>,
    parallel_config: ParallelConfig,
    out_path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
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
    .register::<ValueDatasetCollector>();
    simulation.run();
    if let Some(collector) = simulation.get_event_handler::<StatsCollector>() {
        print_stats(&collector.compute_stats());
    }
    let games = simulation
        .get_event_handler::<ValueDatasetCollector>()
        .map(|c| c.games.clone())
        .unwrap_or_default();
    let header = json!({
        "header": true,
        "features": FEATURES.iter().map(|(n, p)| json!({"name": n, "param": p})).collect::<Vec<_>>(),
        "value": ValueFunctionParams::loaded(),
        "prep": PrepParams::loaded(),
        "deckA": deck_a_path,
        "deckB": deck_b_path,
        "players": player_codes.iter().map(|p| format!("{p:?}")).collect::<Vec<_>>(),
        "numGames": num_games,
        "seed": seed,
        "games": games.len(),
        "positions": games.iter().map(|g| g.positions.len()).sum::<usize>(),
    });
    let file = std::fs::File::create(out_path)?;
    let mut out = std::io::BufWriter::new(file);
    write_dataset(&mut out, &games, header)?;
    out.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::players::PlayerCode;

    fn sim(with_dataset: bool) -> (Vec<Option<GameOutcome>>, Vec<GamePositions>) {
        let deck_a = Deck::from_file("example_decks/venusaur-exeggutor.txt").unwrap();
        let deck_b = Deck::from_file("example_decks/weezing-arbok.txt").unwrap();
        let mut simulation = Simulation::new_with_decks(
            deck_a,
            deck_b,
            vec![PlayerCode::P { max_depth: 2 }, PlayerCode::V],
            4,
            Some(20261116),
            false,
            None,
        )
        .unwrap()
        .register::<StatsCollector>();
        if with_dataset {
            simulation = simulation.register::<ValueDatasetCollector>();
        }
        let outcomes = simulation.run();
        let games = simulation
            .get_event_handler::<ValueDatasetCollector>()
            .map(|c| c.games.clone())
            .unwrap_or_default();
        (outcomes, games)
    }

    /// Misma semilla, mismo resultado con y sin volcado; y cada posicion tiene todas las features
    #[test]
    fn dataset_does_not_change_games() {
        let (plain, _) = sim(false);
        let (dumped, games) = sim(true);
        assert_eq!(plain, dumped);
        assert_eq!(games.len(), 4);
        assert!(games.iter().all(|g| !g.positions.is_empty()));
        for game in &games {
            assert_eq!(game.outcome, dumped[games.iter().position(|x| x.seed == game.seed).unwrap()]);
            assert!(game.positions.iter().all(|p| p.f.len() == FEATURES.len()));
        }
        let mut buf = Vec::new();
        write_dataset(&mut buf, &games, json!({"header": true})).unwrap();
        let text = String::from_utf8(buf).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 1 + games.iter().map(|g| g.positions.len()).sum::<usize>());
        let row: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        let y = row["y"].as_f64().unwrap();
        assert!(y == 0.0 || y == 0.5 || y == 1.0);
    }

    #[test]
    fn compact_keeps_six_digits() {
        assert_eq!(compact(123.456789), 123.457);
        assert_eq!(compact(-0.000123456789), -0.000123457);
        assert_eq!(compact(0.0), 0.0);
    }
}
