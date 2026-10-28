// Funcion de valor propia con features de preparacion (step 2.9, ver docs/search-design.md).
//
// La de upstream (`parametric_value_function` con `DECKGYM_VALUE_PARAMS`) no ve lo que un Pokemon
// va a poder hacer cuando evolucione: un Treecko con [G] y Rare Candy + Mega Sceptile ex en la mano
// vale lo que vale Pound (20). Y `turns_until_opponent_wins` toma como amenaza el ataque al que
// menos energia le falta aunque pegue poco: unirle la [G] al Treecko vuelve a Pound la amenaza y
// los turnos para ganar pasan, por ejemplo, de 2 a 6, asi que cargar a un basico de la linea
// parece peor que no hacer nada. Esta funcion le suma a la de upstream dos terminos, con
// coeficientes en un JSON aparte (`DECKGYM_PREP_PARAMS`) y sin tocar los de upstream:
//
// - `prep_damage`: por cada Pokemon en juego, el dano del mejor ataque que va a poder hacer con las
//   evoluciones que tiene en la mano (o en el mazo, con menos peso) y la energia que le falta,
//   descontado por los turnos que tarda en llegar.
// - `turns_to_win`: reemplaza a `turns_until_opponent_wins` (mismo calculo de KOs al activo y a la
//   banca hasta 3 puntos) eligiendo la amenaza que gana mas rapido, contando los turnos que le
//   faltan a cada forma: energia, evolucion en la mano o Rare Candy.
// - `lock` (step 3.6): lo que vale que el activo pueda dormir o paralizar al activo rival con un
//   ataque que paga pronto: el dano que el rival deja de hacer (la mitad si duerme, todo si queda
//   paralizado) mas el dano pasivo que dispara el sueno (habilidades de dano al final del turno a un
//   activo dormido, como Bad Dreams). Sin esto el bot pone adelante al Pokemon con mas PS aunque no
//   haga nada y guarda al que duerme gratis (cuestionario de Mega Altaria ex + Espeon, quiz/README.md
//   en tcgpocket). Se reconoce por la mecanica del motor (InflictStatusConditions,
//   BadDreamsEndOfTurn), no por nombre de carta.
// - `held` (step 2.12): lo que vale que el activo rival YA este dormido o paralizado al terminar mi
//   turno: el ataque que no va a poder hacer en el suyo (su amenaza) y, si duerme, el dano pasivo
//   de fin de turno que se come al final de su turno y (si la moneda no lo despierta) del mio. `lock`
//   valua poder dormirlo; sin `held` el bot no distingue dejarlo dormido de no hacerlo, y no juega
//   para Bad Dreams (diagnostico del 2.12: con y sin Darkrai el rival termina dormido igual).
// - `reach` (step 2.12): puntos de los Pokemon rivales que mi amenaza mas rapida noquea de un golpe
//   (los de la banca pesan `reach_bench`: hay que traerlos o esperar). Valua el dano repartido por lo
//   que acerca a un KO: tirarle al activo que ya muere con el proximo ataque no suma, dejar a uno de
//   la banca a tiro (para Cyrus o el siguiente) si. Sin esto el dano suelto en la banca solo cuenta
//   por los PS (Water Shuriken a la banca no ayudaba a ganar en el 2.12).
// - Efectos de un turno (step 2.12, cuestionario de Mega Altaria ex + Greninja): los efectos con
//   duracion sobre las cartas (Teary Attack, Growl: -30 al ataque del turno que viene; escudos)
//   valen para el primer golpe y no para los siguientes. Sin esto los turnos para ganar del rival
//   se calculaban como si el -30 durara toda la partida y el bot retiraba a Bonsly para tirarlo en
//   vez de cargar al atacante. Siempre activo en p<n> (no tiene coeficiente).
// - `generator` (step 3.7, partidas de Magnezone + Miraidon ex contra el bot): por cada Pokemon en
//   juego con una habilidad que se carga energia sola cada turno (Volt Charge, Dragon's Blessing
//   desde la banca), mio menos el del rival. Ningun termino veia la energia de los turnos que
//   vienen: el bot evolucionaba Magneton a Magnezone apenas podia (pierde Volt Charge) y el usuario
//   solo evoluciona al activo si lo salva o le sirve el ataque ya. Se reconoce por la mecanica del
//   motor (AbilityMechanic), no por nombre de carta.
//
// Las cartas en la mano solo se usan para el jugador desde cuyo punto de vista se evalua: la mano
// rival es informacion oculta (para el rival cuentan las evoluciones de su mano y su mazo juntos,
// con el peso del mazo, y en `turns_to_win` solo la forma que tiene en juego).

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::actions::abilities::AbilityMechanic;
use crate::actions::{
    expected_attack_damage, expected_attack_damage_with_choices, get_ability_mechanic, Mechanic,
    EFFECT_MECHANIC_MAP,
};
use crate::card_logic::can_rare_candy_evolve;
use crate::hooks::energy_missing;
use crate::models::{Attack, Card, EnergyType, PlayedCard, StatusCondition};
use crate::State;

use super::value_functions::{parametric_value_function, ValueFunctionParams};

/// Ruta a un JSON de [`PrepParams`] para los jugadores con la funcion de valor propia (`p<n>`)
pub const PREP_PARAMS_ENV: &str = "DECKGYM_PREP_PARAMS";

/// Turnos para ganar cuando ninguna amenaza hace dano (el mismo valor que usa upstream)
const NO_THREAT_TURNS: f64 = 30.0;

/// Coeficientes de las features de preparacion. Con `prep_damage` y `turns_to_win` en 0 la funcion
/// de valor es la de upstream tal cual.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PrepParams {
    /// Por punto de dano que cada Pokemon en juego va a poder hacer (ver [`analyze`]), mio menos
    /// el del rival.
    pub prep_damage: f64,
    /// Por turno: si no es 0, reemplaza al coeficiente `turns_until_opponent_wins` de upstream
    /// (que queda en 0) por este, con los turnos para ganar de la amenaza mas rapida (ver
    /// [`analyze`]): los turnos hasta que gana el rival menos los turnos hasta que gano yo.
    pub turns_to_win: f64,
    /// Factor por cada turno de mas que tarda en poder atacar (el turno que viene no se descuenta).
    pub turn_discount: f64,
    /// Peso de los Pokemon de la banca en `prep_damage` (el activo pesa 1).
    pub bench: f64,
    /// Peso de una evolucion que esta en el mazo y no en la mano (hay que robarla).
    pub deck_evolution: f64,
    /// Turnos propios que se miran hacia adelante como maximo.
    pub max_turns: usize,
    /// Por punto de dano que el activo evita o provoca dejando al activo rival dormido o paralizado
    /// (ver [`analyze`]), mio menos el del rival.
    pub lock: f64,
    /// Por punto de dano que el activo rival pierde o recibe por estar ya dormido o paralizado al
    /// evaluar (ver [`Outlook::held`]), mio menos el del rival.
    pub held: f64,
    /// Por punto de KO a tiro de un golpe (ver [`Outlook::reach`]), mio menos el del rival.
    pub reach: f64,
    /// Peso de un Pokemon de la banca rival en `reach` (el activo pesa 1).
    pub reach_bench: f64,
    /// Step 8.2: el dano de un ataque que lo elige en su efecto (Chase Order de Vespiquen ex:
    /// descartar un basico de la banca por 70 mas) es la mejor de las opciones que encola el motor.
    /// Sin esto vale 0 apenas hay una opcion y el bot no ve al atacante como amenaza.
    pub choices: bool,
    /// Step 3.7: por cada Pokemon en juego que genera energia cada turno con su habilidad (ver
    /// [`energy_generators`]), mio menos el del rival.
    pub generator: f64,
}

impl Default for PrepParams {
    fn default() -> Self {
        Self {
            prep_damage: 0.0,
            turns_to_win: 0.0,
            turn_discount: 0.5,
            bench: 0.5,
            deck_evolution: 0.25,
            max_turns: 3,
            lock: 0.0,
            held: 0.0,
            reach: 0.0,
            reach_bench: 0.5,
            choices: false,
            generator: 0.0,
        }
    }
}

impl PrepParams {
    pub fn from_json_file(path: &str) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
        serde_json::from_str(&text).map_err(|e| format!("{path}: {e}"))
    }

    /// Los de `DECKGYM_PREP_PARAMS`, o los default (sin preparacion) si no esta. Se lee una vez
    /// por proceso; un archivo invalido aborta, como `DECKGYM_VALUE_PARAMS`.
    pub fn loaded() -> &'static Self {
        static LOADED: OnceLock<PrepParams> = OnceLock::new();
        LOADED.get_or_init(|| match std::env::var(PREP_PARAMS_ENV) {
            Ok(path) if !path.is_empty() => Self::from_json_file(&path)
                .unwrap_or_else(|e| panic!("Invalid {PREP_PARAMS_ENV}: {e}")),
            _ => Self::default(),
        })
    }
}

/// La funcion de valor de upstream con los coeficientes cargados mas la preparacion de
/// `DECKGYM_PREP_PARAMS`
pub fn loaded_prep_value_function(state: &State, myself: usize) -> f64 {
    prep_value_function(
        state,
        myself,
        ValueFunctionParams::loaded(),
        PrepParams::loaded(),
    )
}

pub fn prep_value_function(
    state: &State,
    myself: usize,
    params: &ValueFunctionParams,
    prep: &PrepParams,
) -> f64 {
    if prep.prep_damage == 0.0
        && prep.turns_to_win == 0.0
        && prep.lock == 0.0
        && prep.held == 0.0
        && prep.reach == 0.0
        && prep.generator == 0.0
    {
        return parametric_value_function(state, myself, params);
    }
    let mut upstream = *params;
    if prep.turns_to_win != 0.0 {
        upstream.turns_until_opponent_wins = 0.0;
    }
    let base = parametric_value_function(state, myself, &upstream);
    let opponent = (myself + 1) % 2;
    let mut scratch = None;
    let mine = analyze(state, myself, myself, prep, &mut scratch);
    let theirs = analyze(state, opponent, myself, prep, &mut scratch);
    base + prep.prep_damage * (mine.damage - theirs.damage)
        + prep.turns_to_win * (theirs.turns_to_win - mine.turns_to_win)
        + prep.lock * (mine.lock - theirs.lock)
        + prep.held * (mine.held - theirs.held)
        + prep.reach * (mine.reach - theirs.reach)
        + prep.generator * (energy_generators(state, myself) - energy_generators(state, opponent))
}

/// Cada termino de [`prep_value_function`] ya multiplicado por su coeficiente (suman el puntaje):
/// los de upstream con su nombre y los de preparacion con `prep_` adelante. Para diagnosticar que
/// empuja una decision (step 3.7, `DECKGYM_VALUE_BREAKDOWN` en el grabador de replays).
pub fn prep_value_breakdown(
    state: &State,
    myself: usize,
    params: &ValueFunctionParams,
    prep: &PrepParams,
) -> Vec<(&'static str, f64)> {
    let mut upstream = *params;
    if prep.turns_to_win != 0.0 {
        upstream.turns_until_opponent_wins = 0.0;
    }
    let mut terms = super::value_functions::parametric_breakdown(state, myself, &upstream);
    let opponent = (myself + 1) % 2;
    let mut scratch = None;
    let mine = analyze(state, myself, myself, prep, &mut scratch);
    let theirs = analyze(state, opponent, myself, prep, &mut scratch);
    terms.extend([
        ("prep_damage", prep.prep_damage * (mine.damage - theirs.damage)),
        ("prep_turns_to_win", prep.turns_to_win * (theirs.turns_to_win - mine.turns_to_win)),
        ("prep_lock", prep.lock * (mine.lock - theirs.lock)),
        ("prep_held", prep.held * (mine.held - theirs.held)),
        ("prep_reach", prep.reach * (mine.reach - theirs.reach)),
        (
            "prep_generator",
            prep.generator * (energy_generators(state, myself) - energy_generators(state, opponent)),
        ),
    ]);
    terms
}

/// Pokemon en juego de `player` que se cargan energia solos cada turno con su habilidad (de la zona
/// o del descarte, a si mismos o al activo). No cuentan los que cortan el turno al usarla ni los que
/// solo cargan desde la banca si estan de activo.
pub(crate) fn energy_generators(state: &State, player: usize) -> f64 {
    state
        .enumerate_in_play_pokemon(player)
        .filter(|(idx, pokemon)| match get_ability_mechanic(&pokemon.card) {
            Some(
                AbilityMechanic::AttachEnergyFromZoneToActiveTypedPokemon { .. }
                | AbilityMechanic::AttachEnergyFromZoneToYourTypedPokemon { .. }
                | AbilityMechanic::AttachEnergyFromZoneToSelf { .. }
                | AbilityMechanic::AttachEnergyFromZoneToSelfAndDamage { .. }
                | AbilityMechanic::AttachEnergyFromDiscardToSelfAndDamage { .. },
            ) => true,
            Some(AbilityMechanic::AttachEnergyFromDiscardToActiveFromBench) => *idx != 0,
            _ => false,
        })
        .count() as f64
}

/// Como [`loaded_prep_value_function`] con los coeficientes de otros archivos: el p<n> sombra del
/// asiento humano (`DECKGYM_SHADOW_PREP_PARAMS` y `DECKGYM_SHADOW_VALUE_PARAMS`) puede puntuar con
/// parametros distintos a los del rival, para volver a puntuar partidas grabadas con un arreglo
/// nuevo. Sin `upstream`, los de `DECKGYM_VALUE_PARAMS`.
pub fn prep_value_function_from(
    params: PrepParams,
    upstream: Option<ValueFunctionParams>,
) -> impl Fn(&State, usize) -> f64 {
    move |state, myself| {
        let upstream = upstream.as_ref().unwrap_or_else(|| ValueFunctionParams::loaded());
        prep_value_function(state, myself, upstream, &params)
    }
}

/// Lo que `player` puede llegar a hacer contra el otro jugador
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Outlook {
    /// Suma, por cada Pokemon en juego (la banca pesa `bench`), del mejor
    /// `peso * dano * turn_discount^(turnos - 1)` entre sus formas y ataques
    pub damage: f64,
    /// Turnos hasta que `player` gana con su amenaza mas rapida
    pub turns_to_win: f64,
    /// Mejor `lost * min(amenaza rival, PS de la forma) + pasivo` del activo, descontado como
    /// `damage`: `lost` es la parte del proximo ataque rival que se pierde por el estado (dormido
    /// 0.5, paralizado 1), la amenaza es el mejor ataque del activo rival que paga con una energia
    /// mas (sin activo rival, en el armado, los PS de la forma), y el pasivo es el dano de fin de turno a un activo dormido de los Pokemon en juego de
    /// `player`, al final de su turno y (si sigue dormido) del siguiente.
    pub lock: f64,
    /// Si el activo rival ya esta dormido o paralizado: su amenaza (el ataque que no hace en su
    /// turno) mas, si duerme, 1.5 veces el dano pasivo a un dormido de `player` (al final de su turno
    /// seguro y del mio con la moneda del medio).
    pub held: f64,
    /// Suma de los puntos de KO de los Pokemon rivales con PS restantes <= el dano de la amenaza mas
    /// rapida de `player` contra cada uno (la banca pesa `reach_bench`).
    pub reach: f64,
}

/// Una forma a la que puede llegar un Pokemon en juego: la carta, en cuantos turnos propios puede
/// evolucionar a ella (0 = ya la tiene) y el peso (1 si es segura, `deck_evolution` si hay que
/// robarla)
struct Form {
    card: Card,
    evolve_turns: usize,
    weight: f64,
}

/// Una amenaza para `turns_to_win`: el Pokemon de `idx` con `form`, que ataca con `attack` (con la
/// energia `fill` que le falta) y hace `damage` al activo rival
struct Threat {
    idx: usize,
    form: Card,
    attack: Attack,
    fill: Vec<EnergyType>,
    damage: f64,
}

/// Recorre las formas y ataques de los Pokemon en juego de `player`. Turnos de una forma = los
/// turnos propios hasta poder atacar con ella (al menos 1, el que viene; uno por energia que falta
/// y uno por paso de evolucion, que pueden ir en paralelo; una energia de un tipo que la zona del
/// mazo no genera no llega nunca). Dano = el que calcula el motor contra el activo rival con esa
/// forma y esa energia. `viewer` es desde quien se evalua: solo de el se mira la mano.
pub(crate) fn analyze(
    state: &State,
    player: usize,
    viewer: usize,
    prep: &PrepParams,
    scratch: &mut Option<State>,
) -> Outlook {
    let defender = (player + 1) % 2;
    let cap = state
        .enumerate_in_play_pokemon(defender)
        .map(|(_, card)| card.get_remaining_hp() as f64)
        .fold(0.0, f64::max);
    let mut zone: Vec<EnergyType> = state.decks[player].energy_types().to_vec();
    zone.sort();
    zone.dedup();
    let in_play: Vec<(usize, PlayedCard)> = state
        .enumerate_in_play_pokemon(player)
        .map(|(idx, pokemon)| (idx, pokemon.clone()))
        .collect();
    let target_hp = state
        .maybe_get_active(defender)
        .map(|active| active.get_remaining_hp() as f64);

    let (threat, passive) = if prep.lock == 0.0 && prep.held == 0.0 {
        (0.0, 0.0)
    } else {
        (
            active_threat(state, defender, &zone_of(state, defender), scratch, prep.choices),
            asleep_passive_damage(state, player),
        )
    };
    let mut lock: f64 = 0.0;

    let mut damage = 0.0;
    // La amenaza que primero noquea al activo rival: (turnos hasta el KO, -dano) mas chico
    let mut best: Option<(f64, Threat)> = None;
    for (idx, pokemon) in in_play {
        let mut best_value: f64 = 0.0;
        for form in forms(state, player, viewer, &pokemon, prep.deck_evolution) {
            let mut as_form = pokemon.clone();
            as_form.card = form.card.clone();
            for attack in form.card.get_attacks() {
                let missing = energy_missing(&as_form, &attack.energy_required, state, player);
                let Some(fill) = fill_from_zone(&missing, &zone) else {
                    continue;
                };
                let turns = form.evolve_turns.max(fill.len()).max(1);
                if turns > prep.max_turns {
                    continue;
                }
                let hit = price(
                    state,
                    scratch,
                    prep.choices,
                    player,
                    idx,
                    &form.card,
                    &attack,
                    &fill,
                    0,
                );
                if idx == 0 && prep.lock != 0.0 {
                    let (lost, asleep) = status_lock(&attack);
                    if lost > 0.0 {
                        // PS de la forma: los que le quedan mas lo que suma la evolucion
                        let hp = pokemon.get_remaining_hp() as f64
                            + (card_hp(&form.card) - card_hp(&pokemon.card)).max(0.0);
                        // Sin activo rival (el armado, cuando elijo primero) se supone que pega
                        // lo que haga falta para noquearlo
                        let lost_damage = if target_hp.is_some() { threat.min(hp) } else { hp };
                        let value = lost * lost_damage + if asleep { 1.5 * passive } else { 0.0 };
                        lock = lock.max(
                            form.weight * value * prep.turn_discount.powi(turns as i32 - 1),
                        );
                    }
                }
                if cap > 0.0 {
                    let value = form.weight
                        * hit.min(cap)
                        * prep.turn_discount.powi(turns as i32 - 1);
                    best_value = best_value.max(value);
                }
                // En turns_to_win solo cuentan las formas seguras (en juego o en la mano propia)
                if let (Some(hp), true) = (target_hp, form.weight == 1.0 && hit > 0.0) {
                    // El primer golpe con los efectos de un turno, los demas sin
                    let steady = if turns == 1 {
                        price_steady(
                            state,
                            scratch,
                            prep.choices,
                            player,
                            idx,
                            &form.card,
                            &attack,
                            &fill,
                            0,
                        )
                    } else {
                        hit
                    };
                    let to_ko = if turns > 1 || hp <= hit || steady <= 0.0 {
                        (turns - 1) as f64 + (hp / hit).ceil()
                    } else {
                        1.0 + ((hp - hit) / steady).ceil()
                    };
                    let better = best.as_ref().is_none_or(|(t, b)| {
                        to_ko < *t || (to_ko == *t && hit > b.damage)
                    });
                    if better {
                        let threat = Threat {
                            idx,
                            form: form.card.clone(),
                            attack: attack.clone(),
                            fill: fill.clone(),
                            damage: hit,
                        };
                        best = Some((to_ko, threat));
                    }
                }
            }
        }
        damage += if idx == 0 {
            best_value
        } else {
            prep.bench * best_value
        };
    }
    let held = match state.maybe_get_active(defender) {
        Some(active) if prep.held != 0.0 && active.is_asleep() => threat + 1.5 * passive,
        Some(active) if prep.held != 0.0 && active.is_paralyzed() => threat,
        _ => 0.0,
    };
    let reach = if prep.reach == 0.0 {
        0.0
    } else {
        ko_reach(state, player, best.as_ref().map(|(_, t)| t), prep, scratch)
    };
    let turns_to_win = if prep.turns_to_win == 0.0 {
        0.0
    } else {
        turns_to_win(state, player, best, scratch, prep.choices)
    };
    Outlook {
        damage,
        turns_to_win,
        lock,
        held,
        reach,
    }
}

/// Puntos de KO de los Pokemon rivales que `threat` (la amenaza mas rapida de `player`, con su
/// forma y energia) noquea de un golpe: el activo pesa 1 y la banca `reach_bench`. Solo cuenta la
/// amenaza que ya puede atacar el turno que viene: una que tarda mas no pone a nadie a tiro.
fn ko_reach(
    state: &State,
    player: usize,
    threat: Option<&Threat>,
    prep: &PrepParams,
    scratch: &mut Option<State>,
) -> f64 {
    let Some(threat) = threat else {
        return 0.0;
    };
    let ready = threat.fill.is_empty()
        && state.in_play_pokemon[player][threat.idx]
            .as_ref()
            .is_some_and(|p| p.card == threat.form);
    if !ready {
        return 0.0;
    }
    let defender = (player + 1) % 2;
    let targets: Vec<(usize, f64, f64)> = state
        .enumerate_in_play_pokemon(defender)
        .map(|(idx, p)| (idx, p.get_remaining_hp() as f64, p.card.get_knockout_points() as f64))
        .collect();
    let mut total = 0.0;
    for (idx, hp, points) in targets {
        let hit = if idx == 0 {
            threat.damage
        } else {
            price(
                state,
                scratch,
                prep.choices,
                player,
                threat.idx,
                &threat.form,
                &threat.attack,
                &threat.fill,
                idx,
            )
        };
        if hit > 0.0 && hp <= hit {
            total += points * if idx == 0 { 1.0 } else { prep.reach_bench };
        }
    }
    total
}

/// PS impresos de una carta (0 si no es Pokemon)
fn card_hp(card: &Card) -> f64 {
    match card {
        Card::Pokemon(pokemon) => pokemon.hp as f64,
        Card::Trainer(_) => 0.0,
    }
}

/// Tipos de energia que genera la zona de `player`
fn zone_of(state: &State, player: usize) -> Vec<EnergyType> {
    let mut zone: Vec<EnergyType> = state.decks[player].energy_types().to_vec();
    zone.sort();
    zone.dedup();
    zone
}

/// Parte del proximo ataque rival que se pierde si `attack` deja dormido (0.5: la moneda del
/// chequeo lo despierta la mitad de las veces) o paralizado (1) al activo rival, y si lo duerme
fn status_lock(attack: &Attack) -> (f64, bool) {
    let Some(effect) = attack.effect.as_deref() else {
        return (0.0, false);
    };
    let conditions = match EFFECT_MECHANIC_MAP.get(effect) {
        Some(Mechanic::InflictStatusConditions {
            conditions,
            target_opponent: true,
        }) => conditions,
        _ => return (0.0, false),
    };
    let asleep = conditions.contains(&StatusCondition::Asleep);
    let lost = if conditions.contains(&StatusCondition::Paralyzed) {
        1.0
    } else if asleep {
        0.5
    } else {
        0.0
    };
    (lost, asleep)
}

/// Dano esperado del mejor ataque del activo de `player` contra el activo rival, entre los que
/// puede pagar con a lo sumo una energia mas de su zona
fn active_threat(
    state: &State,
    player: usize,
    zone: &[EnergyType],
    scratch: &mut Option<State>,
    choices: bool,
) -> f64 {
    let Some(active) = state.maybe_get_active(player) else {
        return 0.0;
    };
    let card = active.card.clone();
    let mut best: f64 = 0.0;
    for attack in card.get_attacks() {
        let missing = energy_missing(active, &attack.energy_required, state, player);
        let Some(fill) = fill_from_zone(&missing, zone) else {
            continue;
        };
        if fill.len() > 1 {
            continue;
        }
        best = best.max(price(state, scratch, choices, player, 0, &card, &attack, &fill, 0));
    }
    best
}

/// Dano de fin de turno que los Pokemon en juego de `player` le hacen a un activo rival dormido
fn asleep_passive_damage(state: &State, player: usize) -> f64 {
    state
        .enumerate_in_play_pokemon(player)
        .filter_map(|(_, pokemon)| match get_ability_mechanic(&pokemon.card) {
            Some(AbilityMechanic::BadDreamsEndOfTurn { amount }) => Some(*amount as f64),
            _ => None,
        })
        .sum()
}

/// Como `calculate_turns_until_opponent_wins` de upstream, desde el lado del atacante y con la
/// amenaza ya elegida: los turnos hasta noquear al activo rival (`to_ko`, que ya cuenta los que tarda
/// en poder atacar) y despues a los de la banca (el que mas aguanta por punto que da, o el de mas HP
/// si falta un punto) hasta llegar a 3 puntos.
fn turns_to_win(
    state: &State,
    player: usize,
    best: Option<(f64, Threat)>,
    scratch: &mut Option<State>,
    choices: bool,
) -> f64 {
    let defender = (player + 1) % 2;
    let Some((to_ko, threat)) = best else {
        return NO_THREAT_TURNS;
    };
    let mut total = to_ko;
    let mut points = state.points[player];
    if let Some(active) = state.maybe_get_active(defender) {
        points += active.card.get_knockout_points();
    }
    let mut bench_damage: [Option<f64>; 4] = [None; 4];
    while points < 3 {
        let safest = state.enumerate_bench_pokemon(defender).max_by_key(|(_, card)| {
            if points == 2 {
                card.get_remaining_hp()
            } else {
                let ko_points = card.card.get_knockout_points().max(1) as u32;
                card.get_remaining_hp() * 1000 / ko_points
            }
        });
        let Some((bench_idx, target)) = safest else {
            break;
        };
        // La banca cae en turnos que vienen: sin los efectos de un turno
        let hit = *bench_damage[bench_idx].get_or_insert_with(|| {
            price_steady(
                state,
                scratch,
                choices,
                player,
                threat.idx,
                &threat.form,
                &threat.attack,
                &threat.fill,
                bench_idx,
            )
        });
        if hit <= 0.0 {
            return NO_THREAT_TURNS;
        }
        total += (target.get_remaining_hp() as f64 / hit).ceil();
        points += target.card.get_knockout_points();
    }
    total
}

/// Las formas de `pokemon`: la actual; cada evolucion de la mano (y de Rare Candy si esta en la
/// mano), a un turno, o a dos si es fase 2 sin Rare Candy pero con la fase 1 en la mano; y las del
/// mazo con peso `deck_weight`. Para el rival (que no es `viewer`) su mano no se conoce: sus
/// evoluciones salen de la mano y el mazo juntos, con peso `deck_weight`. La legalidad se le
/// pregunta al motor (`can_evolve_into`, `can_rare_candy_evolve`); que el Pokemon haya entrado este
/// turno no importa, porque evoluciona el turno que viene.
fn forms(
    state: &State,
    player: usize,
    viewer: usize,
    pokemon: &PlayedCard,
    deck_weight: f64,
) -> Vec<Form> {
    let mut out = vec![Form {
        card: pokemon.card.clone(),
        evolve_turns: 0,
        weight: 1.0,
    }];
    let mut settled = pokemon.clone();
    settled.played_this_turn = false;
    let hand = &state.hands[player];
    let deck = &state.decks[player].cards;
    let candy = player == viewer
        && hand
            .iter()
            .any(|card| matches!(card, Card::Trainer(t) if t.name == "Rare Candy"));
    let pools: Vec<(Vec<&Card>, f64)> = if player == viewer {
        vec![
            (hand.iter().collect(), 1.0),
            (deck.iter().collect(), deck_weight),
        ]
    } else {
        vec![(hand.iter().chain(deck.iter()).collect(), deck_weight)]
    };
    for (cards, weight) in pools {
        if weight == 0.0 {
            continue;
        }
        for card in cards.iter().copied() {
            if !matches!(card, Card::Pokemon(_)) || out.iter().any(|f| f.card == *card) {
                continue;
            }
            let turns = if pokemon.card.can_evolve_into(card) {
                1
            } else if can_rare_candy_evolve(card, &settled) {
                // Fase 2 desde el basico: con Rare Candy en la mano en un turno, si no por la fase 1
                let via_stage1 = cards
                    .iter()
                    .any(|s1| pokemon.card.can_evolve_into(s1) && s1.can_evolve_into(card));
                if candy && weight == 1.0 {
                    1
                } else if via_stage1 || weight < 1.0 {
                    2
                } else {
                    continue;
                }
            } else {
                continue;
            };
            out.push(Form {
                card: card.clone(),
                evolve_turns: turns,
                weight,
            });
        }
    }
    out
}

/// La energia de la zona que paga `missing` (lo que devuelve `energy_missing`): los tipos tal cual
/// y los incoloros con el primer tipo de la zona. None si falta un tipo que la zona no genera.
fn fill_from_zone(missing: &[EnergyType], zone: &[EnergyType]) -> Option<Vec<EnergyType>> {
    let mut fill = Vec::with_capacity(missing.len());
    for energy in missing {
        if *energy == EnergyType::Colorless {
            fill.push(*zone.first()?);
        } else if zone.contains(energy) {
            fill.push(*energy);
        } else {
            return None;
        }
    }
    Some(fill)
}

/// Dano esperado de `attack` de `form` en el lugar `idx` de `player`, con `extra` energia de mas,
/// contra el Pokemon rival en `defender_idx`: se le pregunta al motor sobre una copia del estado
/// (una por evaluacion, compartida) con ese Pokemon cambiado y los dos puestos de activos. Si el
/// motor no lo puede calcular, el dano impreso.
#[allow(clippy::too_many_arguments)]
fn price(
    state: &State,
    scratch: &mut Option<State>,
    choices: bool,
    player: usize,
    idx: usize,
    form: &Card,
    attack: &Attack,
    extra: &[EnergyType],
    defender_idx: usize,
) -> f64 {
    price_with(state, scratch, choices, player, idx, form, attack, extra, defender_idx, false)
}

/// Como [`price`], sin los efectos de un turno del atacante y del defensor: el dano de los golpes
/// que vienen despues del primero
#[allow(clippy::too_many_arguments)]
fn price_steady(
    state: &State,
    scratch: &mut Option<State>,
    choices: bool,
    player: usize,
    idx: usize,
    form: &Card,
    attack: &Attack,
    extra: &[EnergyType],
    defender_idx: usize,
) -> f64 {
    price_with(state, scratch, choices, player, idx, form, attack, extra, defender_idx, true)
}

#[allow(clippy::too_many_arguments)]
fn price_with(
    state: &State,
    scratch: &mut Option<State>,
    choices: bool,
    player: usize,
    idx: usize,
    form: &Card,
    attack: &Attack,
    extra: &[EnergyType],
    defender_idx: usize,
    steady: bool,
) -> f64 {
    let printed = attack.fixed_damage as f64;
    let defender = (player + 1) % 2;
    let scratch = scratch.get_or_insert_with(|| state.clone());
    let Some(pokemon) = scratch.in_play_pokemon[player][idx].as_mut() else {
        return printed;
    };
    let original_card = std::mem::replace(&mut pokemon.card, form.clone());
    let attached = pokemon.attached_energy.len();
    pokemon.attached_energy.extend_from_slice(extra);
    let mine = if steady { Some(pokemon.take_effects()) } else { None };
    let theirs = if steady {
        scratch.in_play_pokemon[defender][defender_idx]
            .as_mut()
            .map(|p| p.take_effects())
    } else {
        None
    };
    let current = scratch.current_player;
    scratch.current_player = player;
    scratch.in_play_pokemon[player].swap(0, idx);
    scratch.in_play_pokemon[defender].swap(0, defender_idx);
    let damage = if choices {
        expected_attack_damage_with_choices(scratch, attack)
    } else {
        expected_attack_damage(scratch, attack)
    }
    .unwrap_or(printed);
    scratch.in_play_pokemon[defender].swap(0, defender_idx);
    scratch.in_play_pokemon[player].swap(0, idx);
    scratch.current_player = current;
    if let (Some(effects), Some(p)) = (theirs, scratch.in_play_pokemon[defender][defender_idx].as_mut()) {
        p.restore_effects(effects);
    }
    if let Some(pokemon) = scratch.in_play_pokemon[player][idx].as_mut() {
        pokemon.card = original_card;
        pokemon.attached_energy.truncate(attached);
        if let Some(effects) = mine {
            pokemon.restore_effects(effects);
        }
    }
    damage
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::actions::{apply_action, SimpleAction};
    use crate::card_ids::CardId;
    use crate::database::get_card_by_enum;
    use crate::players::{Player, TurnCompletionPlayer, TwoTurnPlayer};
    use crate::test_support::get_test_game_with_board;
    use crate::Deck;
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    /// Los coeficientes de value_params/step-2-2.json
    pub(crate) fn step_2_2_params() -> ValueFunctionParams {
        ValueFunctionParams {
            real_damage: true,
            energy_damage: 2.0,
            active_pokemon_online_score: 0.0,
            active_damage: 1.0,
            ..ValueFunctionParams::baseline()
        }
    }

    /// Los de value_params/prep.json
    pub(crate) fn prep_params() -> PrepParams {
        PrepParams::from_json_file("value_params/prep.json").unwrap()
    }

    fn step_2_2(state: &State, myself: usize) -> f64 {
        parametric_value_function(state, myself, &step_2_2_params())
    }

    fn step_2_2_prep(state: &State, myself: usize) -> f64 {
        prep_value_function(state, myself, &step_2_2_params(), &prep_params())
    }

    pub(crate) fn t2() -> TurnCompletionPlayer {
        TurnCompletionPlayer::with_value_function(Deck::default(), 2, Box::new(step_2_2))
    }

    pub(crate) fn p2() -> TurnCompletionPlayer {
        TurnCompletionPlayer::with_value_function(Deck::default(), 2, Box::new(step_2_2_prep))
    }

    pub(crate) fn d2() -> TwoTurnPlayer {
        let mut player = TwoTurnPlayer::new(Deck::default(), 2, Box::new(step_2_2));
        player.samples = 2;
        player.top = 3;
        player.next_depth = 2;
        player
    }

    /// Mi turno 3 (el segundo mio) con zona de energia Grass y la [G] de este turno por unir,
    /// `hand` en la mano y la mano rival vacia
    pub(crate) fn prep_state(
        mine: Vec<PlayedCard>,
        hand: &[CardId],
        theirs: Vec<PlayedCard>,
    ) -> State {
        let game = get_test_game_with_board(mine, theirs);
        let mut state = game.get_state_clone();
        state.hands[0] = hand.iter().map(|id| get_card_by_enum(*id)).collect();
        state.hands[1].clear();
        state.move_generation_stack.clear();
        state.energy_zone[0].current = Some(EnergyType::Grass);
        state.decks[0].energy_types = vec![EnergyType::Grass];
        state
    }

    /// Juega el turno de player 0 con `player`: el estado al pasar el turno y si ataco
    pub(crate) fn play_turn(player: &mut dyn Player, mut state: State, seed: u64) -> (State, bool) {
        let mut rng = StdRng::seed_from_u64(seed);
        let turn = state.turn_count;
        let mut attacked = false;
        while !state.is_game_over() && state.current_player == 0 && state.turn_count == turn {
            let (actor, actions) = state.generate_possible_actions();
            let action = if actions.len() == 1 || actor != 0 {
                actions[0].clone()
            } else {
                player.decision_fn(&mut rng, &state, &actions)
            };
            attacked |= matches!(action.action, SimpleAction::Attack(_));
            apply_action(&mut rng, &mut state, &action);
        }
        (state, attacked)
    }

    pub(crate) fn board(state: &State) -> Vec<String> {
        state
            .enumerate_in_play_pokemon(0)
            .map(|(_, p)| format!("{}{:?}", p.get_name(), p.attached_energy))
            .collect()
    }

    pub(crate) fn energy_on(state: &State, name: &str) -> usize {
        state
            .enumerate_in_play_pokemon(0)
            .filter(|(_, p)| p.get_name() == name)
            .map(|(_, p)| p.attached_energy.len())
            .sum()
    }

    fn has(state: &State, name: &str) -> bool {
        state
            .enumerate_in_play_pokemon(0)
            .any(|(_, p)| p.get_name() == name)
    }

    /// `active` con [G] y un Treecko que entro a la banca este turno (Rare Candy recien el que
    /// viene); en la mano Rare Candy y Mega Sceptile ex; el rival tiene un Chansey de 120 PS sin
    /// energia ni banca.
    pub(crate) fn candy_state(active: CardId) -> State {
        let mut treecko = PlayedCard::from_id(CardId::B3005Treecko);
        treecko.played_this_turn = true;
        prep_state(
            vec![
                PlayedCard::from_id(active).with_energy(vec![EnergyType::Grass]),
                treecko,
            ],
            &[CardId::A3144RareCandy, CardId::B3008MegaSceptileEx],
            vec![PlayedCard::from_id(CardId::A1202Chansey)],
        )
    }

    // Furfrou (B1 206) activo con [G] y Sharp Fang ([C][C], 50): unir la [G] al activo le deja
    // atacar ya. Atacar ahora y el turno que viene suma 100 y no noquea al Chansey; cargar al
    // Treecko deja a Mega Sceptile ex (130) atacando el turno que viene (Rare Candy, [G] y retirar
    // a Furfrou con su [G]) y noquea. Para upstream, la amenaza de cada lado es el ataque al que
    // menos energia le falta: con la [G] en el Treecko pasa a ser Pound (20), y los turnos para
    // ganar suben de 2 a 6.

    // Step 3.6: Igglybuff (Sleepy Lullaby: sin costo, duerme) adelante con dos Darkrai (Bad Dreams,
    // 20 al final de cada turno a un activo dormido) en la banca, o Darkrai adelante con Igglybuff
    // y el otro Darkrai en la banca. El rival tiene un Lucario con [F][F] (40).
    fn sleep_board(iggly_in_front: bool) -> State {
        let iggly = PlayedCard::from_id(CardId::A4a059Igglybuff);
        let darkrai = || PlayedCard::from_id(CardId::B2b040Darkrai);
        let mine = if iggly_in_front {
            vec![iggly, darkrai(), darkrai()]
        } else {
            vec![darkrai(), iggly, darkrai()]
        };
        let lucario = PlayedCard::from_id(CardId::A2092Lucario)
            .with_energy(vec![EnergyType::Fighting, EnergyType::Fighting]);
        prep_state(mine, &[], vec![lucario])
    }

    fn with_lock(lock: f64) -> PrepParams {
        PrepParams {
            lock,
            ..prep_params()
        }
    }

    #[test]
    fn lock_counts_the_lost_attack_and_the_bad_dreams_damage() {
        let prep = with_lock(1.0);
        let mut scratch = None;
        // Igglybuff ya puede dormir: la mitad de lo que pierde (min(40, 30 PS)) mas 40 de Bad Dreams
        // al final de mi turno y la mitad de las veces otros 40 al final del rival
        let front = analyze(&sleep_board(true), 0, 0, &prep, &mut scratch);
        assert_eq!(front.lock, 0.5 * 30.0 + 1.5 * 40.0);
        // Darkrai duerme con Dark Slumber, pero le faltan las tres energias: dos turnos de descuento.
        // Lucario le pega 60 (40 mas la debilidad), que entran en sus 100 PS
        let mut scratch = None;
        let back = analyze(&sleep_board(false), 0, 0, &prep, &mut scratch);
        assert_eq!(back.lock, (0.5 * 60.0 + 1.5 * 40.0) * 0.25);
        // Sin el coeficiente no se calcula
        let mut scratch = None;
        assert_eq!(analyze(&sleep_board(true), 0, 0, &prep_params(), &mut scratch).lock, 0.0);
    }

    #[test]
    fn lock_puts_the_free_sleeper_in_front_of_bad_dreams() {
        let value = |iggly_in_front, prep: &PrepParams| {
            prep_value_function(&sleep_board(iggly_in_front), 0, &step_2_2_params(), prep)
        };
        // Sin lock gana el de mas PS adelante: lo que el usuario contesto distinto en el cuestionario
        assert!(value(false, &prep_params()) > value(true, &prep_params()));
        // Con el coeficiente de value_params/prep-lock.json gana Igglybuff adelante
        assert!(value(true, &with_lock(8.0)) > value(false, &with_lock(8.0)));
    }

    #[test]
    fn held_counts_the_lost_attack_and_bad_dreams_only_while_asleep() {
        let prep = PrepParams {
            held: 1.0,
            ..prep_params()
        };
        let held = |status: Option<StatusCondition>| {
            let mut state = sleep_board(true);
            if let Some(s) = status {
                state.in_play_pokemon[1][0].as_mut().unwrap().set_status_raw(s);
            }
            let mut scratch = None;
            analyze(&state, 0, 0, &prep, &mut scratch).held
        };
        assert_eq!(held(None), 0.0);
        let paralyzed = held(Some(StatusCondition::Paralyzed));
        assert!(paralyzed > 0.0);
        // Dormido: lo mismo mas 1.5 veces los 40 de los dos Darkrai
        assert_eq!(held(Some(StatusCondition::Asleep)) - paralyzed, 1.5 * 40.0);
    }

    // Step 2.12: Furfrou con [G][G] (Sharp Fang, 50) contra un Chansey activo y un Lucario en la
    // banca: el Lucario a tiro (PS <= 50) suma `reach_bench`, el Chansey a tiro suma 1
    fn reach_board(chansey_hp: u32, lucario_hp: u32) -> State {
        let furfrou = PlayedCard::from_id(CardId::B1206Furfrou)
            .with_energy(vec![EnergyType::Grass, EnergyType::Grass]);
        let chansey = PlayedCard::from_id(CardId::A1202Chansey).with_remaining_hp(chansey_hp);
        let lucario = PlayedCard::from_id(CardId::A2092Lucario).with_remaining_hp(lucario_hp);
        prep_state(vec![furfrou], &[], vec![chansey, lucario])
    }

    #[test]
    fn reach_counts_the_points_one_hit_away() {
        let prep = PrepParams {
            reach: 1.0,
            ..prep_params()
        };
        let reach = |chansey, lucario| {
            let mut scratch = None;
            analyze(&reach_board(chansey, lucario), 0, 0, &prep, &mut scratch).reach
        };
        assert_eq!(reach(120, 60), 0.0);
        assert_eq!(reach(120, 40), prep.reach_bench);
        assert_eq!(reach(50, 40), 1.0 + prep.reach_bench);
        // Pegarle 20 al activo que ya muere no suma; dejar al de la banca a tiro si
        assert_eq!(reach(30, 60), reach(50, 60));
        assert!(reach(50, 40) > reach(30, 60));
    }

    #[test]
    fn a_one_turn_attack_reduction_only_slows_the_first_hit() {
        let prep = PrepParams {
            turns_to_win: 1.0,
            ..prep_params()
        };
        let turns = |teary: bool| {
            let mut state = reach_board(120, 60);
            if teary {
                state.in_play_pokemon[0][0]
                    .as_mut()
                    .unwrap()
                    .add_effect(crate::effects::CardEffect::ReducedAttackDamage { amount: 30 }, 1);
            }
            let mut scratch = None;
            analyze(&state, 0, 0, &prep, &mut scratch).turns_to_win
        };
        // Sharp Fang (50) noquea al Chansey de 120 en 3; con -30 el primer golpe es 20 y despues
        // 50: 1 + ceil(100 / 50) = 3, no ceil(120 / 20) = 6
        assert!(turns(true) <= turns(false) + 1.0, "{} vs {}", turns(true), turns(false));
    }

    #[test]
    fn t2_attacks_now_instead_of_charging_the_treecko() {
        for seed in 0..3 {
            let (end, attacked) = play_turn(&mut t2(), candy_state(CardId::B1206Furfrou), seed);
            assert!(attacked, "seed {seed}: {:?}", board(&end));
            assert_eq!(energy_on(&end, "Treecko"), 0, "seed {seed}: {:?}", board(&end));
        }
    }

    #[test]
    fn p2_charges_the_treecko_instead_of_attacking_now() {
        // p2 une la [G] al Treecko, retira a Furfrou (paga con su [G]) y ataca con Pound: el turno
        // que viene Rare Candy sobre el activo deja a Mega Sceptile ex atacando sin retirar
        // (20 + 130 noquea al Chansey). Lo que se chequea es que no carga a Furfrou para Sharp Fang
        for seed in 0..3 {
            let (end, _) = play_turn(&mut p2(), candy_state(CardId::B1206Furfrou), seed);
            assert_eq!(energy_on(&end, "Treecko"), 1, "seed {seed}: {:?}", board(&end));
            assert!(energy_on(&end, "Furfrou") < 2, "seed {seed}: {:?}", board(&end));
        }
    }

    #[test]
    fn p2_charges_the_benched_treecko_when_the_active_already_attacks() {
        // Furfrou (B3b 061, Double Headbutt [C]) ya ataca con su [G]: la del turno va al Treecko
        for seed in 0..3 {
            let (end, _) = play_turn(&mut p2(), candy_state(CardId::B3b061Furfrou), seed);
            assert_eq!(energy_on(&end, "Treecko"), 1, "seed {seed}: {:?}", board(&end));
        }
    }

    #[test]
    fn turns_to_win_counts_the_mega_sceptile_of_the_next_turn() {
        let prep = PrepParams {
            turns_to_win: 1.0,
            ..PrepParams::default()
        };
        // Con la [G] en el Treecko: Rare Candy y [G] el turno que viene, 130 al Chansey de 120
        let mut charged = candy_state(CardId::B1206Furfrou);
        charged.in_play_pokemon[0][1]
            .as_mut()
            .unwrap()
            .attached_energy
            .push(EnergyType::Grass);
        assert_eq!(analyze(&charged, 0, 0, &prep, &mut None).turns_to_win, 1.0);
        // Con la [G] en Furfrou y 50 al Chansey: Furfrou necesita 2 ataques mas y Mega Sceptile ex
        // (le faltan dos [G]) ataca recien en dos turnos
        let mut attacked = candy_state(CardId::B1206Furfrou);
        attacked.in_play_pokemon[0][0]
            .as_mut()
            .unwrap()
            .attached_energy
            .push(EnergyType::Grass);
        attacked.in_play_pokemon[1][0].as_mut().unwrap().apply_damage(50);
        assert_eq!(analyze(&attacked, 0, 0, &prep, &mut None).turns_to_win, 2.0);
    }

    #[test]
    fn the_opponent_hand_is_not_used() {
        // Desde el punto de vista del rival, mi Rare Candy y mi Mega Sceptile ex no se ven: mi
        // amenaza es la que tengo en juego
        let prep = PrepParams {
            turns_to_win: 1.0,
            deck_evolution: 0.0,
            ..PrepParams::default()
        };
        let mut state = candy_state(CardId::B1206Furfrou);
        state.in_play_pokemon[0][1]
            .as_mut()
            .unwrap()
            .attached_energy
            .push(EnergyType::Grass);
        let mine = analyze(&state, 0, 0, &prep, &mut None);
        let seen_by_rival = analyze(&state, 0, 1, &prep, &mut None);
        assert_eq!(mine.turns_to_win, 1.0);
        // Furfrou con [G]: Sharp Fang el turno que viene, 3 ataques al Chansey
        assert_eq!(seen_by_rival.turns_to_win, 3.0);
    }

    /// Furfrou activo con [G] (Double Headbutt [C], ya ataca) y un Treecko en la banca desde el
    /// turno anterior; en la mano Grovyle y Mega Sceptile ex, sin Rare Candy
    fn grovyle_state() -> State {
        prep_state(
            vec![
                PlayedCard::from_id(CardId::B3b061Furfrou).with_energy(vec![EnergyType::Grass]),
                PlayedCard::from_id(CardId::B3005Treecko),
            ],
            &[CardId::B3006Grovyle, CardId::B3008MegaSceptileEx],
            vec![PlayedCard::from_id(CardId::A1202Chansey)],
        )
    }

    #[test]
    fn p2_evolves_into_grovyle_on_the_way_to_mega_sceptile() {
        // t2 tambien lo hace aca: es para que la preparacion no lo rompa
        for seed in 0..3 {
            let (end, _) = play_turn(&mut p2(), grovyle_state(), seed);
            assert!(has(&end, "Grovyle"), "seed {seed}: {:?}", board(&end));
            assert_eq!(energy_on(&end, "Grovyle"), 1, "seed {seed}: {:?}", board(&end));
        }
    }

    #[test]
    fn without_coefficients_it_is_the_upstream_value_function() {
        let state = grovyle_state();
        let off = PrepParams::default();
        assert_eq!(
            prep_value_function(&state, 0, &step_2_2_params(), &off),
            parametric_value_function(&state, 0, &step_2_2_params())
        );
    }

    #[test]
    fn prep_damage_values_the_next_evolution_step() {
        // Treecko sin energia con Grovyle y Mega Sceptile ex en la mano: Mega Sceptile ex esta a
        // dos turnos (Grovyle y despues Mega, [G][G]). Grovyle con [G] esta a uno. Con tope en
        // el HP del Chansey (120) y la banca a la mitad: 0.5 * 120 * 0.5 antes, 0.5 * 120 despues
        let prep = PrepParams {
            prep_damage: 1.0,
            deck_evolution: 0.0,
            ..PrepParams::default()
        };
        let state = grovyle_state();
        let treecko = analyze(&state, 0, 0, &prep, &mut None).damage;
        let mut evolved = state.clone();
        evolved.in_play_pokemon[0][1] =
            Some(PlayedCard::from_id(CardId::B3006Grovyle).with_energy(vec![EnergyType::Grass]));
        evolved.hands[0].retain(|card| card.get_name() != "Grovyle");
        let grovyle = analyze(&evolved, 0, 0, &prep, &mut None).damage;
        // Lo del Furfrou activo es igual en los dos estados
        let furfrou = treecko - 0.5 * 120.0 * 0.5;
        assert!(
            (grovyle - furfrou - 0.5 * 120.0).abs() < 1e-6,
            "{treecko} -> {grovyle}"
        );
    }

    #[test]
    fn choices_price_chase_order_with_the_discard() {
        // Step 8.2: Vespiquen ex con [G][G] contra un Chansey. Con un Combee en la banca el motor
        // encola la eleccion (70, o 140 descartando al Combee) y el dano del resultado es 0
        let board = |bench: bool| {
            let mut mine = vec![PlayedCard::from_id(CardId::B4011VespiquenEx)
                .with_energy(vec![EnergyType::Grass, EnergyType::Grass])];
            if bench {
                mine.push(PlayedCard::from_id(CardId::B4010Combee));
            }
            prep_state(mine, &[], vec![PlayedCard::from_id(CardId::A1202Chansey)])
        };
        let hit = |bench: bool, choices: bool| {
            let state = board(bench);
            let vespiquen = state.in_play_pokemon[0][0].as_ref().unwrap().card.clone();
            let attack = vespiquen.get_attacks()[0].clone();
            price(&state, &mut None, choices, 0, 0, &vespiquen, &attack, &[], 0)
        };
        assert_eq!(hit(true, false), 0.0);
        assert_eq!(hit(true, true), 140.0);
        assert_eq!(hit(false, false), 70.0);
        assert_eq!(hit(false, true), 70.0);
        // Y pasa a ser la amenaza: Chansey (120) cae de un golpe
        let prep = PrepParams {
            turns_to_win: 1.0,
            choices: true,
            ..PrepParams::default()
        };
        assert_eq!(analyze(&board(true), 0, 0, &prep, &mut None).turns_to_win, 1.0);
    }

    #[test]
    fn a_type_the_zone_does_not_generate_is_never_reached() {
        // Mega Sceptile ex pide [G][G]: con zona de solo Fire no cuenta
        let prep = PrepParams {
            prep_damage: 1.0,
            deck_evolution: 0.0,
            ..PrepParams::default()
        };
        let mut state = grovyle_state();
        state.decks[0].energy_types = vec![EnergyType::Fire];
        let fire = analyze(&state, 0, 0, &prep, &mut None).damage;
        state.decks[0].energy_types = vec![EnergyType::Grass];
        let grass = analyze(&state, 0, 0, &prep, &mut None).damage;
        assert!(grass > fire, "{fire} vs {grass}");
    }
}

#[cfg(test)]
/// Step 8.2 (palanca 3): los ataques que `choices` valua distinto, con el atacante cargado, un
/// Combee en la banca y dos Chansey rivales. `cargo test choices_audit -- --ignored --nocapture`
mod choices_audit {
    use super::tests::prep_state;
    use super::*;
    use crate::card_ids::CardId;
    use crate::database::get_card_by_enum;
    use strum::IntoEnumIterator;

    #[test]
    #[ignore]
    fn list_attacks_priced_differently_with_choices() {
        let mut seen = std::collections::HashSet::new();
        for id in CardId::iter() {
            let card = get_card_by_enum(id);
            let Card::Pokemon(p) = &card else { continue };
            for attack in card.get_attacks() {
                if !seen.insert((p.name.clone(), attack.title.clone())) {
                    continue;
                }
                let energy: Vec<EnergyType> = attack
                    .energy_required
                    .iter()
                    .map(|e| if *e == EnergyType::Colorless { p.energy_type } else { *e })
                    .collect();
                let bench = PlayedCard::from_id(CardId::B4010Combee);
                let state = prep_state(
                    vec![
                        PlayedCard::from_id(id).with_energy(energy),
                        bench,
                        PlayedCard::from_id(CardId::A1202Chansey),
                    ],
                    &[],
                    vec![
                        PlayedCard::from_id(CardId::A1202Chansey),
                        PlayedCard::from_id(CardId::A1202Chansey).with_remaining_hp(60),
                    ],
                );
                let off = price(&state, &mut None, false, 0, 0, &card, &attack, &[], 0);
                let on = price(&state, &mut None, true, 0, 0, &card, &attack, &[], 0);
                if (off - on).abs() > 1e-9 {
                    let printed = attack.fixed_damage;
                    println!("CHOICES|{}|{}|{printed}|{off}|{on}", p.name, attack.title);
                }
            }
        }
    }
}
