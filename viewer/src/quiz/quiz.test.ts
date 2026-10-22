import { describe, expect, it } from "vitest";
import type { ViewState } from "../types/replay";
import { questionReplay, turnLabel, uniqueOptions, withAnswer, type QuizFile, type QuizQuestion } from "./quiz";

const emptyState: ViewState = {
  turn: 3,
  current_player: 0,
  points: [0, 1],
  winner: null,
  players: [0, 1].map(() => ({
    hand: [],
    hand_count: 0,
    deck_count: 10,
    discard: [],
    discard_energies: [],
    energy_zone: { current: null, next: null },
    in_play: [null, null, null, null],
  })) as unknown as ViewState["players"],
  stadium: null,
};

const question: QuizQuestion = {
  id: "q01",
  gameId: "g",
  rival: { id: "r", rank: 2, name: "Mega Lucario ex" },
  ply: 10,
  turn: 3,
  ownTurn: 2,
  wentFirst: true,
  lost: false,
  kind: "evolucionar",
  kinds: ["evolucionar"],
  flags: [],
  state: emptyState,
  history: [],
  options: [
    { text: "Place(Darkrai, 1)", label: "Bajar Darkrai a la banca", action: "EndTurn" },
    { text: "Place(Darkrai, 2)", label: "Bajar Darkrai a la banca", action: "EndTurn" },
    { text: "EndTurn", label: "Pasar el turno", action: "EndTurn" },
  ],
  bot: { chosen: 2, scores: [1, 1, 2] },
};

const quiz: QuizFile = {
  version: 1,
  deck: { id: "d", name: "Mega Altaria ex Espeon", rank: 1, list: [{ id: "B1 102", name: "Mega Altaria ex", count: 2 }] },
  cards: {},
  questions: [question],
};

describe("uniqueOptions", () => {
  it("shows options with the same text once, keeping the first index", () => {
    expect(uniqueOptions(question.options)).toEqual([
      { label: "Bajar Darkrai a la banca", index: 0 },
      { label: "Pasar el turno", index: 2 },
    ]);
  });
});

describe("questionReplay", () => {
  it("is a one-snapshot replay of the question's state with the deck list as player 0's deck", () => {
    const replay = questionReplay(quiz, question);
    expect(replay.steps).toEqual([]);
    expect(replay.final_state).toBe(emptyState);
    expect(replay.players[0].deck).toEqual(["B1 102", "B1 102"]);
    expect(replay.players[1].name).toContain("Mega Lucario ex");
  });
});

describe("withAnswer", () => {
  it("replaces an earlier answer to the same question and keeps question order", () => {
    const base = { version: 1 as const, quiz: "x", deck: "d", updatedAt: "", answers: [] };
    const order = ["q01", "q02"];
    const a = withAnswer(base, { id: "q02", choice: 1, note: "", at: "t1" }, order);
    const b = withAnswer(a, { id: "q01", choice: 0, note: "n", at: "t2" }, order);
    const c = withAnswer(b, { id: "q02", choice: 2, note: "", at: "t3" }, order);
    expect(c.answers.map((x) => [x.id, x.choice])).toEqual([
      ["q01", 0],
      ["q02", 2],
    ]);
    expect(c.updatedAt).toBe("t3");
  });
});

describe("turnLabel", () => {
  it("names the setup and own turns", () => {
    expect(turnLabel(question)).toBe("Turno propio 2 (empezaste vos)");
    expect(turnLabel({ ...question, turn: 0 })).toBe("Armado inicial");
  });
});
