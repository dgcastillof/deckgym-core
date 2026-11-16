// Funcion de valor aprendida de self-play (paso 3.5 de tcgpocket).
//
// Las vueltas a mano sobre p2 se agotaron: lo que queda del sesgo de la simulacion contra Limitless
// no se arreglo con terminos nuevos ni con recalibrar los pesos a mano (rama calibrate-value). Este
// archivo arma lo necesario para aprender los pesos con una regresion logistica sobre posiciones
// de self-play (`simulate --value-dataset`, ver `src/value_dataset.rs`, y
// `scripts/deckgym-learn-value.mjs` en tcgpocket):
//
// - `features`: el vector de features de una posicion desde el punto de vista de un jugador. Son
//   los terminos de la funcion de valor de p2 ([`prep_value_breakdown`]) con cada coeficiente en
//   1, y cuatro features nuevas de armado ([`armado`]). Como la funcion de valor es una suma
//   ponderada, p2 con unos coeficientes es exactamente `sum(coeficiente * feature)`.
// - `l<n>`: t<n> con la funcion de valor de `DECKGYM_PREP_LEARNED_PARAMS` (un JSON con `value`,
//   `prep` y `armado`), para jugar el valor aprendido contra el titular p<n> en la misma partida.
//
// Las features de armado (el titular no las tiene: el armado es lo que p2 juega peor, docs/mejoras.md
// de tcgpocket):
// - `armado_ready_turns`: turnos hasta que el atacante principal (la carta con el ataque impreso mas
//   fuerte de las que tiene el jugador) puede atacar, los del rival menos los mios.
// - `armado_line_board`: Pokemon en juego de la linea del atacante principal, mios menos los del rival.
// - `armado_line_hand`: piezas de la linea en mi mano (y Rare Candy si el atacante es fase 2). Solo
//   las mias: la mano rival es informacion oculta.
// - `armado_attacker_energy`: la energia del Pokemon de la linea mas cargado, hasta el costo del
//   ataque principal, mia menos la del rival.
// Se reconocen por la linea evolutiva y el costo del ataque, no por nombre de carta.

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::card_logic::can_rare_candy_evolve;
use crate::hooks::{energy_missing, to_playable_card};
use crate::models::{Card, EnergyType};
use crate::State;

use super::own_value::{fill_from_zone, forms, prep_value_breakdown, prep_value_function, zone_of, PrepParams};
use super::value_functions::ValueFunctionParams;

/// Ruta al JSON de [`LearnedParams`] del jugador `l<n>`
pub const LEARNED_PARAMS_ENV: &str = "DECKGYM_PREP_LEARNED_PARAMS";

/// Turnos de `armado_ready_turns` cuando el atacante principal no llega (o tarda mas)
pub const READY_CAP: f64 = 6.0;

/// Peso de una forma que esta en el mazo (como `deck_evolution` del titular)
const DECK_WEIGHT: f64 = 0.25;

/// Coeficientes de las features de armado
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ArmadoParams {
    pub ready_turns: f64,
    pub line_board: f64,
    pub line_hand: f64,
    pub attacker_energy: f64,
}

/// Lo que lee `l<n>`: los coeficientes de upstream, los de preparacion y los de armado
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LearnedParams {
    pub value: ValueFunctionParams,
    pub prep: PrepParams,
    #[serde(default)]
    pub armado: ArmadoParams,
}

impl LearnedParams {
    pub fn from_json_file(path: &str) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
        serde_json::from_str(&text).map_err(|e| format!("{path}: {e}"))
    }

    /// Los de `DECKGYM_PREP_LEARNED_PARAMS`. Sin la variable no hay `l<n>`: aborta, para que un
    /// error de tipeo nunca juegue con otros coeficientes.
    pub fn loaded() -> &'static Self {
        static LOADED: OnceLock<LearnedParams> = OnceLock::new();
        LOADED.get_or_init(|| match std::env::var(LEARNED_PARAMS_ENV) {
            Ok(path) if !path.is_empty() => Self::from_json_file(&path)
                .unwrap_or_else(|e| panic!("Invalid {LEARNED_PARAMS_ENV}: {e}")),
            _ => panic!("l<n> needs {LEARNED_PARAMS_ENV}"),
        })
    }
}

/// La funcion de valor de `l<n>`: la de p2 con los coeficientes aprendidos mas el armado
pub fn learned_value_function(state: &State, myself: usize, params: &LearnedParams) -> f64 {
    let base = prep_value_function(state, myself, &params.value, &params.prep);
    let a = &params.armado;
    if a.ready_turns == 0.0 && a.line_board == 0.0 && a.line_hand == 0.0 && a.attacker_energy == 0.0
    {
        return base;
    }
    let t = armado_terms(state, myself);
    base + a.ready_turns * t[0] + a.line_board * t[1] + a.line_hand * t[2] + a.attacker_energy * t[3]
}

pub fn loaded_learned_value_function(state: &State, myself: usize) -> f64 {
    learned_value_function(state, myself, LearnedParams::loaded())
}

/// Nombre de cada feature y el coeficiente que le corresponde en [`LearnedParams`]
/// (`seccion.campo`), en el orden de [`features`]. `is_winner` no esta: en una posicion de
/// decision siempre vale 0 y el aprendido lo tiene que dejar fijo.
pub const FEATURES: &[(&str, &str)] = &[
    ("points", "value.points"),
    ("pokemon_value", "value.pokemon_value"),
    ("hand_size", "value.hand_size"),
    ("deck_size", "value.deck_size"),
    ("active_retreat_cost", "value.active_retreat_cost"),
    ("active_pokemon_online_score", "value.active_pokemon_online_score"),
    ("active_safety", "value.active_safety"),
    ("active_has_tool", "value.active_has_tool"),
    ("online_pokemon_count", "value.online_pokemon_count"),
    ("energy_distance_to_online", "value.energy_distance_to_online"),
    ("opponent_discard_size", "value.opponent_discard_size"),
    ("active_damage", "value.active_damage"),
    ("bench_hp", "value.bench_hp"),
    ("bench_count", "value.bench_count"),
    ("evolution_stage", "value.evolution_stage"),
    ("evolutions_in_hand", "value.evolutions_in_hand"),
    ("active_status", "value.active_status"),
    ("can_ko_active", "value.can_ko_active"),
    ("attached_energy", "value.attached_energy"),
    ("energy_damage", "value.energy_damage"),
    ("status_damage", "value.status_damage"),
    ("bench_online", "value.bench_online"),
    ("prep_damage", "prep.prep_damage"),
    ("prep_turns_to_win", "prep.turns_to_win"),
    ("prep_lock", "prep.lock"),
    ("prep_held", "prep.held"),
    ("prep_reach", "prep.reach"),
    ("prep_generator", "prep.generator"),
    ("prep_bench_generator", "prep.bench_generator"),
    ("armado_ready_turns", "armado.ready_turns"),
    ("armado_line_board", "armado.line_board"),
    ("armado_line_hand", "armado.line_hand"),
    ("armado_attacker_energy", "armado.attacker_energy"),
];

/// Los coeficientes lineales de `value` y `prep` en 1 (y `is_winner` y `turns_until_opponent_wins`
/// en 0), con el resto (`real_damage`, `turn_discount`, `bench`, `choices`...) como vienen: con
/// estos, cada termino de [`prep_value_breakdown`] es la feature cruda.
pub fn unit_params(
    value: &ValueFunctionParams,
    prep: &PrepParams,
) -> (ValueFunctionParams, PrepParams) {
    let mut v = *value;
    let mut p = *prep;
    for (_, param) in FEATURES {
        set_coefficient(&mut v, &mut p, &mut ArmadoParams::default(), param, 1.0);
    }
    v.is_winner = 0.0;
    v.turns_until_opponent_wins = 0.0;
    (v, p)
}

/// Pone `w` en el coeficiente `param` (`seccion.campo`, como en [`FEATURES`])
pub fn set_coefficient(
    value: &mut ValueFunctionParams,
    prep: &mut PrepParams,
    armado: &mut ArmadoParams,
    param: &str,
    w: f64,
) {
    let slot: &mut f64 = match param {
        "value.points" => &mut value.points,
        "value.pokemon_value" => &mut value.pokemon_value,
        "value.hand_size" => &mut value.hand_size,
        "value.deck_size" => &mut value.deck_size,
        "value.active_retreat_cost" => &mut value.active_retreat_cost,
        "value.active_pokemon_online_score" => &mut value.active_pokemon_online_score,
        "value.active_safety" => &mut value.active_safety,
        "value.active_has_tool" => &mut value.active_has_tool,
        "value.online_pokemon_count" => &mut value.online_pokemon_count,
        "value.energy_distance_to_online" => &mut value.energy_distance_to_online,
        "value.opponent_discard_size" => &mut value.opponent_discard_size,
        "value.active_damage" => &mut value.active_damage,
        "value.bench_hp" => &mut value.bench_hp,
        "value.bench_count" => &mut value.bench_count,
        "value.evolution_stage" => &mut value.evolution_stage,
        "value.evolutions_in_hand" => &mut value.evolutions_in_hand,
        "value.active_status" => &mut value.active_status,
        "value.can_ko_active" => &mut value.can_ko_active,
        "value.attached_energy" => &mut value.attached_energy,
        "value.energy_damage" => &mut value.energy_damage,
        "value.status_damage" => &mut value.status_damage,
        "value.bench_online" => &mut value.bench_online,
        "prep.prep_damage" => &mut prep.prep_damage,
        "prep.turns_to_win" => &mut prep.turns_to_win,
        "prep.lock" => &mut prep.lock,
        "prep.held" => &mut prep.held,
        "prep.reach" => &mut prep.reach,
        "prep.generator" => &mut prep.generator,
        "prep.bench_generator" => &mut prep.bench_generator,
        "armado.ready_turns" => &mut armado.ready_turns,
        "armado.line_board" => &mut armado.line_board,
        "armado.line_hand" => &mut armado.line_hand,
        "armado.attacker_energy" => &mut armado.attacker_energy,
        other => panic!("Unknown coefficient {other}"),
    };
    *slot = w;
}

/// Las features de [`FEATURES`] para `myself`, con los parametros de [`unit_params`]
pub fn features(
    state: &State,
    myself: usize,
    unit_value: &ValueFunctionParams,
    unit_prep: &PrepParams,
) -> Vec<f64> {
    let terms = prep_value_breakdown(state, myself, unit_value, unit_prep);
    let armado = armado_terms(state, myself);
    FEATURES
        .iter()
        .map(|(name, _)| match *name {
            "armado_ready_turns" => armado[0],
            "armado_line_board" => armado[1],
            "armado_line_hand" => armado[2],
            "armado_attacker_energy" => armado[3],
            _ => terms
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, v)| *v)
                .unwrap_or_else(|| panic!("Missing term {name}")),
        })
        .collect()
}

/// Las cuatro features de armado desde `myself`, ya como diferencia (ver el encabezado)
pub fn armado_terms(state: &State, myself: usize) -> [f64; 4] {
    let opponent = (myself + 1) % 2;
    let mine = armado(state, myself, myself);
    let theirs = armado(state, opponent, myself);
    [
        theirs.ready_turns - mine.ready_turns,
        mine.line_board - theirs.line_board,
        mine.line_hand,
        mine.attacker_energy - theirs.attacker_energy,
    ]
}

/// Armado de `player` visto desde `viewer` (solo de el se mira la mano)
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Armado {
    pub ready_turns: f64,
    pub line_board: f64,
    pub line_hand: f64,
    pub attacker_energy: f64,
}

/// Todas las cartas de `player` (en juego, mano, mazo y descarte): de aca sale el atacante principal.
/// La lista de un mazo del meta es publica, asi que la mano rival entra para elegirlo, no para contar piezas.
fn all_cards(state: &State, player: usize) -> Vec<&Card> {
    let mut out: Vec<&Card> = state
        .enumerate_in_play_pokemon(player)
        .map(|(_, p)| &p.card)
        .collect();
    out.extend(state.hands[player].iter());
    out.extend(state.decks[player].cards.iter());
    out.extend(state.discard_piles[player].iter());
    out
}

/// El dano impreso mas alto de los ataques de una carta
fn max_printed(card: &Card) -> u32 {
    card.get_attacks().iter().map(|a| a.fixed_damage).max().unwrap_or(0)
}

fn stage(card: &Card) -> u8 {
    match card {
        Card::Pokemon(p) => p.stage,
        Card::Trainer(_) => 0,
    }
}

fn is_rare_candy(card: &Card) -> bool {
    matches!(card, Card::Trainer(t) if t.name == "Rare Candy")
}

/// El atacante principal de `player`: el de mayor dano impreso (desempata la fase y los PS)
fn main_attacker(cards: &[&Card]) -> Option<Card> {
    cards
        .iter()
        .filter(|c| matches!(c, Card::Pokemon(_)))
        .max_by_key(|c| {
            let hp = match c {
                Card::Pokemon(p) => p.hp,
                Card::Trainer(_) => 0,
            };
            (max_printed(c), stage(c), hp)
        })
        .map(|c| (*c).clone())
}

/// Nombres de la linea de `main` entre `cards`: el, sus fases 1 y sus basicos (directo, por la fase
/// 1 o, si es fase 2, por Rare Candy)
fn line_names(main: &Card, cards: &[&Card]) -> Vec<String> {
    let mut names = vec![main.get_name()];
    let stage1: Vec<&Card> = cards
        .iter()
        .copied()
        .filter(|c| c.can_evolve_into(main))
        .collect();
    for c in &stage1 {
        let name = c.get_name();
        if !names.contains(&name) {
            names.push(name);
        }
    }
    if stage(main) == 2 {
        for c in cards.iter().copied().filter(|c| c.is_basic()) {
            let linked = stage1.iter().any(|s1| c.can_evolve_into(s1))
                || rare_candy_basic(c, main);
            let name = c.get_name();
            if linked && !names.contains(&name) {
                names.push(name);
            }
        }
    }
    names
}

/// Si Rare Candy evoluciona `basic` en `stage2` (lo pregunta al motor con un Pokemon en juego de mentira)
fn rare_candy_basic(basic: &Card, stage2: &Card) -> bool {
    can_rare_candy_evolve(stage2, &to_playable_card(basic, false))
}

pub fn armado(state: &State, player: usize, viewer: usize) -> Armado {
    let cards = all_cards(state, player);
    let Some(main) = main_attacker(&cards) else {
        return Armado {
            ready_turns: READY_CAP,
            line_board: 0.0,
            line_hand: 0.0,
            attacker_energy: 0.0,
        };
    };
    let line = line_names(&main, &cards);
    let main_name = main.get_name();
    let best = max_printed(&main);
    let attacks: Vec<_> = main
        .get_attacks()
        .iter()
        .filter(|a| a.fixed_damage == best)
        .cloned()
        .collect();
    let cost = attacks
        .iter()
        .map(|a| a.energy_required.len())
        .min()
        .unwrap_or(0) as f64;
    let zone: Vec<EnergyType> = zone_of(state, player);

    let mut ready = READY_CAP;
    let mut line_board = 0.0;
    let mut attacker_energy: f64 = 0.0;
    for (_, pokemon) in state.enumerate_in_play_pokemon(player) {
        if !line.contains(&pokemon.get_name()) {
            continue;
        }
        line_board += 1.0;
        attacker_energy = attacker_energy.max((pokemon.attached_energy.len() as f64).min(cost));
        for form in forms(state, player, viewer, pokemon, DECK_WEIGHT) {
            if form.card.get_name() != main_name {
                continue;
            }
            let mut as_form = pokemon.clone();
            as_form.card = form.card.clone();
            for attack in &attacks {
                let missing = energy_missing(&as_form, &attack.energy_required, state, player);
                let Some(fill) = fill_from_zone(&missing, &zone) else {
                    continue;
                };
                let draw = if form.weight < 1.0 { 1 } else { 0 };
                let turns = (form.evolve_turns.max(fill.len()) + draw) as f64;
                ready = ready.min(turns);
            }
        }
    }
    // Sin la linea en juego: un basico de la linea en la mano (o en el mazo, un turno mas) que se
    // baja ahora y evoluciona en los turnos que siguen
    if ready >= READY_CAP {
        let basic_in = |pool: &[Card]| {
            pool.iter()
                .any(|c| c.is_basic() && line.contains(&c.get_name()))
        };
        let steps = (stage(&main) as f64).max(cost).max(1.0);
        if player == viewer && basic_in(&state.hands[player]) {
            ready = ready.min(steps);
        } else if basic_in(&state.decks[player].cards)
            || (player != viewer && basic_in(&state.hands[player]))
        {
            ready = ready.min(steps + 1.0);
        }
    }
    let line_hand = if player == viewer {
        state.hands[player]
            .iter()
            .filter(|c| {
                (matches!(c, Card::Pokemon(_)) && line.contains(&c.get_name()))
                    || (stage(&main) == 2 && is_rare_candy(c))
            })
            .count() as f64
    } else {
        0.0
    };
    Armado {
        ready_turns: ready.min(READY_CAP),
        line_board,
        line_hand,
        attacker_energy,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card_ids::CardId;
    use crate::models::PlayedCard;
    use crate::players::own_value::tests::{candy_state, prep_state, step_2_2_params};

    fn titular_prep() -> PrepParams {
        PrepParams::from_json_file("value_params/prep-8-2.json").unwrap()
    }

    /// Con los coeficientes del titular en `l<n>`, el valor es la suma de coeficiente por feature
    /// (mas `is_winner`, que no es feature) y coincide con el de p2
    #[test]
    fn learned_value_is_the_weighted_sum_of_features() {
        let value = step_2_2_params();
        let prep = titular_prep();
        let (uv, up) = unit_params(&value, &prep);
        let learned = LearnedParams {
            value,
            prep,
            armado: ArmadoParams {
                ready_turns: 30.0,
                line_board: 20.0,
                line_hand: 10.0,
                attacker_energy: 5.0,
            },
        };
        let states = [
            candy_state(CardId::B3005Treecko),
            candy_state(CardId::A1202Chansey),
            prep_state(
                vec![PlayedCard::from_id(CardId::B3005Treecko)],
                &[CardId::B3008MegaSceptileEx, CardId::A3144RareCandy],
                vec![PlayedCard::from_id(CardId::A1202Chansey).with_energy(vec![EnergyType::Grass])],
            ),
        ];
        for state in &states {
            for myself in 0..2 {
                let f = features(state, myself, &uv, &up);
                assert_eq!(f.len(), FEATURES.len());
                let sum: f64 = FEATURES
                    .iter()
                    .enumerate()
                    .map(|(i, (_, param))| coefficient(&learned, param) * f[i])
                    .sum();
                // Posiciones sin ganador: is_winner vale 0
                assert!(state.winner.is_none());
                let expected = learned_value_function(state, myself, &learned);
                assert!(
                    (expected - sum).abs() < 1e-6 * expected.abs().max(1.0),
                    "{expected} != {sum}"
                );
                // set_coefficient escribe donde coefficient lee
                let (mut v, mut p, mut a) = (value, prep, ArmadoParams::default());
                for (_, param) in FEATURES {
                    set_coefficient(&mut v, &mut p, &mut a, param, coefficient(&learned, param));
                }
                assert_eq!((v, p, a), (value, prep, learned.armado));
                // Sin armado, el aprendido con los coeficientes del titular es p2
                let p2 = prep_value_function(state, myself, &value, &prep);
                let no_armado = LearnedParams {
                    armado: ArmadoParams::default(),
                    ..learned
                };
                assert_eq!(learned_value_function(state, myself, &no_armado), p2);
            }
        }
    }

    fn coefficient(params: &LearnedParams, param: &str) -> f64 {
        let v = &params.value;
        let p = &params.prep;
        let a = &params.armado;
        match param {
            "value.points" => v.points,
            "value.pokemon_value" => v.pokemon_value,
            "value.hand_size" => v.hand_size,
            "value.deck_size" => v.deck_size,
            "value.active_retreat_cost" => v.active_retreat_cost,
            "value.active_pokemon_online_score" => v.active_pokemon_online_score,
            "value.active_safety" => v.active_safety,
            "value.active_has_tool" => v.active_has_tool,
            "value.online_pokemon_count" => v.online_pokemon_count,
            "value.energy_distance_to_online" => v.energy_distance_to_online,
            "value.opponent_discard_size" => v.opponent_discard_size,
            "value.active_damage" => v.active_damage,
            "value.bench_hp" => v.bench_hp,
            "value.bench_count" => v.bench_count,
            "value.evolution_stage" => v.evolution_stage,
            "value.evolutions_in_hand" => v.evolutions_in_hand,
            "value.active_status" => v.active_status,
            "value.can_ko_active" => v.can_ko_active,
            "value.attached_energy" => v.attached_energy,
            "value.energy_damage" => v.energy_damage,
            "value.status_damage" => v.status_damage,
            "value.bench_online" => v.bench_online,
            // turns_to_win reemplaza a turns_until_opponent_wins de upstream
            "prep.prep_damage" => p.prep_damage,
            "prep.turns_to_win" => p.turns_to_win,
            "prep.lock" => p.lock,
            "prep.held" => p.held,
            "prep.reach" => p.reach,
            "prep.generator" => p.generator,
            "prep.bench_generator" => p.bench_generator,
            "armado.ready_turns" => a.ready_turns,
            "armado.line_board" => a.line_board,
            "armado.line_hand" => a.line_hand,
            "armado.attacker_energy" => a.attacker_energy,
            other => panic!("{other}"),
        }
    }

    /// Treecko en juego con Rare Candy y Mega Sceptile ex en la mano: la linea tiene dos piezas en
    /// la mano y una en juego; con la Rare Candy, Mega Sceptile ex llega en un turno si la energia alcanza
    #[test]
    fn armado_sees_the_line_in_hand_and_board() {
        let state = prep_state(
            vec![PlayedCard::from_id(CardId::B3005Treecko).with_energy(vec![EnergyType::Grass])],
            &[CardId::A3144RareCandy, CardId::B3008MegaSceptileEx],
            vec![PlayedCard::from_id(CardId::A1202Chansey)],
        );
        let mine = armado(&state, 0, 0);
        assert_eq!(mine.line_board, 1.0);
        assert_eq!(mine.line_hand, 2.0);
        assert!(mine.ready_turns < READY_CAP, "{mine:?}");
        // Sin las piezas en la mano tarda mas
        let mut without = state.clone();
        without.hands[0].clear();
        let bare = armado(&without, 0, 0);
        assert_eq!(bare.line_hand, 0.0);
        assert!(bare.ready_turns > mine.ready_turns, "{bare:?} vs {mine:?}");
        // La mano del rival no se cuenta
        let theirs = armado(&state, 0, 1);
        assert_eq!(theirs.line_hand, 0.0);
    }
}

#[cfg(test)]
mod code_tests {
    use crate::players::{parse_player_code, PlayerCode};

    #[test]
    fn l_codes_parse() {
        assert_eq!(parse_player_code("l2"), Ok(PlayerCode::L { max_depth: 2 }));
        assert_eq!(parse_player_code("L"), Ok(PlayerCode::L { max_depth: 2 }));
        assert!(parse_player_code("lx").is_err());
    }
}
