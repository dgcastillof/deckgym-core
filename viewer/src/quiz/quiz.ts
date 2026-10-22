// Cuestionario de posiciones (step 3.6 del roadmap de tcgpocket): tipos del archivo de preguntas que
// escribe `scripts/deckgym-quiz.mjs` en tcgpocket y del de respuestas que guarda este visor, y los
// helpers puros de la pantalla (testeados en quiz.test.ts).

import type { Card, Replay, SimpleAction, ViewState } from "../types/replay";

export interface QuizOption {
  /** `Display` del SimpleAction, como en un replay */
  text: string;
  /** Texto corto en castellano con los nombres de las cartas de la mesa */
  label: string;
  action: SimpleAction;
}

export interface QuizHistoryEntry {
  turn: number;
  actor: number;
  label: string;
}

export interface QuizQuestion {
  id: string;
  gameId: string;
  rival: { id: string; rank: number; name: string };
  ply: number;
  turn: number;
  /** Turno propio del mazo (0 = armado inicial) */
  ownTurn: number;
  wentFirst: boolean;
  lost: boolean;
  kind: string;
  kinds: string[];
  flags: string[];
  /** Foto antes de decidir, con la mano del rival tapada */
  state: ViewState;
  history: QuizHistoryEntry[];
  options: QuizOption[];
  /** La jugada del bot: la pantalla no la muestra */
  bot: { chosen: number; scores: number[] };
}

export interface QuizFile {
  version: number;
  deck: { id: string; name: string; rank: number; list: { id: string; name: string; count: number }[] };
  cards: Record<string, Card>;
  questions: QuizQuestion[];
}

export interface QuizAnswer {
  id: string;
  /** Indice en `options` (el primero de las que tienen el mismo texto) */
  choice: number;
  note: string;
  at: string;
}

export interface QuizAnswers {
  version: 1;
  quiz: string;
  deck: string;
  updatedAt: string;
  answers: QuizAnswer[];
}

/** Las jugadas con el mismo texto son la misma para quien juega (bajar Darkrai a la banca 1 o a la
 * 2 con la banca vacia): se muestran una vez y la respuesta guarda el primer indice. */
export function uniqueOptions(options: QuizOption[]): { label: string; index: number }[] {
  const seen = new Set<string>();
  const out: { label: string; index: number }[] = [];
  options.forEach((option, index) => {
    const label = option.label || option.text;
    if (seen.has(label)) return;
    seen.add(label);
    out.push({ label, index });
  });
  return out;
}

/** Un replay de una sola foto (la posicion de la pregunta) para dibujarla con el tablero de siempre. */
export function questionReplay(quiz: QuizFile, question: QuizQuestion): Replay {
  return {
    version: 1,
    game_id: question.gameId,
    seed: 0,
    players: [
      { name: `Vos: ${quiz.deck.name}`, deck: quiz.deck.list.flatMap((c) => Array<string>(c.count).fill(c.id)) },
      { name: `Rival: #${question.rival.rank} ${question.rival.name}`, deck: [] },
    ],
    cards: quiz.cards,
    steps: [],
    final_state: question.state,
    outcome: null,
  };
}

/** Reemplaza (o agrega) la respuesta de una pregunta, en el orden de las preguntas. */
export function withAnswer(current: QuizAnswers, answer: QuizAnswer, order: string[]): QuizAnswers {
  const others = current.answers.filter((a) => a.id !== answer.id);
  const answers = [...others, answer].sort((a, b) => order.indexOf(a.id) - order.indexOf(b.id));
  return { ...current, updatedAt: answer.at, answers };
}

/** "Turno propio 2 (empezaste vos)" o "Armado inicial" */
export function turnLabel(question: QuizQuestion): string {
  if (question.turn === 0) return "Armado inicial";
  return `Turno propio ${question.ownTurn} (${question.wentFirst ? "empezaste vos" : "empezó el rival"})`;
}
