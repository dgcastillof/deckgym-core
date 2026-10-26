use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};

use crate::models::EnergyType;

const NUM_TYPES: usize = 10;

fn type_index(energy_type: EnergyType) -> usize {
    match energy_type {
        EnergyType::Grass => 0,
        EnergyType::Fire => 1,
        EnergyType::Water => 2,
        EnergyType::Lightning => 3,
        EnergyType::Psychic => 4,
        EnergyType::Fighting => 5,
        EnergyType::Darkness => 6,
        EnergyType::Metal => 7,
        EnergyType::Dragon => 8,
        EnergyType::Colorless => 9,
    }
}

/// Whole-game bookkeeping for reporting only (event-task statistics, see
/// `crate::event_stats`): never read by game logic or players.
///
/// It compares equal and hashes to nothing, so two states that differ only in their tally are
/// the same state for the search players that memoize states.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GameTally {
    /// Opponent's Active Pokemon Knocked Out by an attack, per attacking player and per type of
    /// the attacking Pokemon (a dual-type attacker counts under each of its types).
    active_kos_by_type: [[u8; NUM_TYPES]; 2],
    /// Largest total damage dealt to the opponent's Pokemon by one attack, per player.
    max_attack_damage: [u16; 2],
    /// Damage accumulated by the attack of `attack_turn` (an attack can apply damage in several
    /// steps, e.g. after choosing bench targets).
    attack_damage: u16,
    attack_turn: u8,
    /// Ends of turn (both players' turns) in which the opponent's Active Pokemon was Asleep, per
    /// player: the turns a passive "if Asleep" ability such as Bad Dreams could have hit.
    asleep_turn_ends: [u16; 2],
    /// Damage dealt by passive end-of-turn abilities to the opponent's Asleep Active, per player.
    passive_asleep_damage: [u16; 2],
    /// Damage dealt to the opponent's Pokemon outside of attacks (Abilities, end-of-turn effects),
    /// per player: [to the Active, to the Bench].
    non_attack_damage: [[u16; 2]; 2],
    /// Opponent's Pokemon Knocked Out by any source, per player: [Active, Bench], and the points.
    kos: [[u8; 2]; 2],
}

impl GameTally {
    pub fn record_active_ko(&mut self, attacker: usize, attacker_types: &[EnergyType]) {
        for t in attacker_types {
            let slot = &mut self.active_kos_by_type[attacker][type_index(*t)];
            *slot = slot.saturating_add(1);
        }
    }

    /// Damage (after modifiers, before capping to the target's remaining HP) dealt to the
    /// opponent by the active attack of `turn`.
    pub fn record_attack_damage(&mut self, attacker: usize, turn: u8, damage: u32) {
        if self.attack_turn != turn {
            self.attack_turn = turn;
            self.attack_damage = 0;
        }
        self.attack_damage = self.attack_damage.saturating_add(damage.min(u16::MAX as u32) as u16);
        self.max_attack_damage[attacker] = self.max_attack_damage[attacker].max(self.attack_damage);
    }

    pub fn record_asleep_turn_end(&mut self, player: usize) {
        self.asleep_turn_ends[player] = self.asleep_turn_ends[player].saturating_add(1);
    }

    pub fn record_passive_asleep_damage(&mut self, player: usize, damage: u32) {
        let d = damage.min(u16::MAX as u32) as u16;
        self.passive_asleep_damage[player] = self.passive_asleep_damage[player].saturating_add(d);
    }

    pub fn record_non_attack_damage(&mut self, player: usize, to_bench: bool, damage: u32) {
        let slot = &mut self.non_attack_damage[player][to_bench as usize];
        *slot = slot.saturating_add(damage.min(u16::MAX as u32) as u16);
    }

    pub fn record_ko(&mut self, player: usize, bench: bool) {
        let slot = &mut self.kos[player][bench as usize];
        *slot = slot.saturating_add(1);
    }

    pub fn asleep_turn_ends(&self, player: usize) -> u16 {
        self.asleep_turn_ends[player]
    }

    pub fn passive_asleep_damage(&self, player: usize) -> u16 {
        self.passive_asleep_damage[player]
    }

    pub fn non_attack_damage(&self, player: usize) -> [u16; 2] {
        self.non_attack_damage[player]
    }

    pub fn kos(&self, player: usize) -> [u8; 2] {
        self.kos[player]
    }

    pub fn active_kos(&self, player: usize, energy_type: EnergyType) -> u8 {
        self.active_kos_by_type[player][type_index(energy_type)]
    }

    pub fn max_attack_damage(&self, player: usize) -> u16 {
        self.max_attack_damage[player]
    }
}

impl PartialEq for GameTally {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

impl Eq for GameTally {}

impl Hash for GameTally {
    fn hash<H: Hasher>(&self, _state: &mut H) {}
}
