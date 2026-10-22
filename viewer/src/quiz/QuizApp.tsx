// Modo cuestionario del visor (`/?quiz=<base>`): muestra cada posicion de quiz/<base>-preguntas.json
// con el tablero de siempre y las jugadas posibles, sin la del bot. El usuario elige una, puede dejar
// una nota corta, y cada respuesta se guarda al toque en quiz/<base>.json (PUT al server de vite, ver
// vite.config.ts); si no hay server, queda en el navegador y se puede descargar.

import { useCallback, useEffect, useMemo, useState } from "react";
import { Board } from "../board/Board";
import { createCardImageStore, preloadCardImages, resolveCardImageConfig } from "../board/cardImages";
import type { BoardScene } from "../board/scene";
import { PlaybackController } from "../store/playbackController";
import { questionReplay, turnLabel, uniqueOptions, withAnswer, type QuizAnswers, type QuizFile } from "./quiz";

const NOTE_MAX = 280;

type SaveState = "idle" | "saving" | "saved" | "error";

function draftKey(name: string) {
  return `deckgym-quiz:${name}`;
}

function readDraft(name: string): QuizAnswers | null {
  try {
    const raw = window.localStorage.getItem(draftKey(name));
    return raw ? (JSON.parse(raw) as QuizAnswers) : null;
  } catch {
    return null;
  }
}

function writeDraft(name: string, answers: QuizAnswers) {
  try {
    window.localStorage.setItem(draftKey(name), JSON.stringify(answers));
  } catch {
    // Sin localStorage el server sigue guardando
  }
}

function download(name: string, answers: QuizAnswers) {
  const blob = new Blob([JSON.stringify(answers, null, 2)], { type: "application/json" });
  const a = document.createElement("a");
  a.href = URL.createObjectURL(blob);
  a.download = `${name}.json`;
  a.click();
  URL.revokeObjectURL(a.href);
}

export default function QuizApp({ name }: { name: string }) {
  const [quiz, setQuiz] = useState<QuizFile | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [answers, setAnswers] = useState<QuizAnswers>({ version: 1, quiz: name, deck: "", updatedAt: "", answers: [] });
  const [index, setIndex] = useState(0);
  const [choice, setChoice] = useState<number | null>(null);
  const [note, setNote] = useState("");
  const [saveState, setSaveState] = useState<SaveState>("idle");
  const [scene, setScene] = useState<BoardScene | null>(null);
  const images = useMemo(() => createCardImageStore(resolveCardImageConfig()), []);

  // Preguntas y, si ya hay, respuestas guardadas (el server gana sobre el borrador del navegador
  // salvo que el borrador sea mas nuevo)
  useEffect(() => {
    (async () => {
      try {
        const res = await fetch(`/quiz/${encodeURIComponent(name)}-preguntas.json`);
        if (!res.ok) throw new Error(`No se pudo leer /quiz/${name}-preguntas.json (HTTP ${res.status})`);
        const q = (await res.json()) as QuizFile;
        let saved: QuizAnswers | null = null;
        const ans = await fetch(`/quiz/${encodeURIComponent(name)}.json`).catch(() => null);
        if (ans?.ok) saved = (await ans.json()) as QuizAnswers;
        const draft = readDraft(name);
        const draftIsNewer = Boolean(draft && (!saved || draft.updatedAt > saved.updatedAt));
        const best = draftIsNewer ? draft : saved;
        // Respuestas que no llegaron al server (estaba apagado): se suben al abrir
        if (draftIsNewer && draft) {
          fetch(`/quiz/${encodeURIComponent(name)}.json`, {
            method: "PUT",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({ ...draft, deck: q.deck.id }, null, 2),
          })
            .then((res) => setSaveState(res.ok ? "saved" : "error"))
            .catch(() => setSaveState("error"));
        }
        const start = best ?? { version: 1 as const, quiz: name, deck: q.deck.id, updatedAt: "", answers: [] };
        setQuiz(q);
        setAnswers({ ...start, deck: q.deck.id });
        const answered = new Set(start.answers.map((a) => a.id));
        const firstOpen = q.questions.findIndex((x) => !answered.has(x.id));
        setIndex(firstOpen >= 0 ? firstOpen : 0);
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
      }
    })();
  }, [name]);

  const question = quiz?.questions[index] ?? null;

  useEffect(() => {
    if (quiz) preloadCardImages(images, Object.keys(quiz.cards));
  }, [quiz, images]);

  // Una foto por pregunta, con el tablero de siempre
  useEffect(() => {
    if (!quiz || !question || !scene) return;
    const c = new PlaybackController(scene, questionReplay(quiz, question));
    return () => c.destroy();
  }, [quiz, question, scene]);

  // Al cambiar de pregunta se carga lo ya respondido
  useEffect(() => {
    if (!question) return;
    const prev = answers.answers.find((a) => a.id === question.id);
    setChoice(prev?.choice ?? null);
    setNote(prev?.note ?? "");
    // answers no va en las dependencias: solo al cambiar de pregunta
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [question]);

  const save = useCallback(
    async (next: QuizAnswers) => {
      setAnswers(next);
      writeDraft(name, next);
      setSaveState("saving");
      try {
        const res = await fetch(`/quiz/${encodeURIComponent(name)}.json`, {
          method: "PUT",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify(next, null, 2),
        });
        setSaveState(res.ok ? "saved" : "error");
      } catch {
        setSaveState("error");
      }
    },
    [name],
  );

  if (error) return <div className="quiz-message">{error}</div>;
  if (!quiz || !question) return <div className="quiz-message">Cargando el cuestionario…</div>;

  const order = quiz.questions.map((q) => q.id);
  const answered = new Set(answers.answers.map((a) => a.id));
  const options = uniqueOptions(question.options);
  const last = index === quiz.questions.length - 1;

  const submit = () => {
    if (choice == null) return;
    void save(withAnswer(answers, { id: question.id, choice, note: note.trim().slice(0, NOTE_MAX), at: new Date().toISOString() }, order));
    if (!last) setIndex(index + 1);
  };

  return (
    <div className="app">
      <header className="app-header">
        <div>
          <h1>Cuestionario: {quiz.deck.name}</h1>
          <span className="subtitle">
            {answered.size} de {quiz.questions.length} respondidas
            {saveState === "saving" && " · guardando…"}
            {saveState === "saved" && " · guardado"}
            {saveState === "error" && " · no se pudo guardar en el server (queda en el navegador)"}
          </span>
        </div>
        <div className="header-actions">
          <button type="button" onClick={() => download(name, answers)}>
            Descargar respuestas
          </button>
        </div>
      </header>
      <div className="app-body quiz-body">
        <div className="main-column quiz-board">
          <div className="app-shell">
            <Board onReady={setScene} images={images} />
          </div>
          <div className="quiz-dots" aria-label="Preguntas">
            {quiz.questions.map((q, i) => (
              <button
                key={q.id}
                type="button"
                className={`quiz-dot${i === index ? " current" : ""}${answered.has(q.id) ? " done" : ""}`}
                title={`${q.id}: ${q.kind}`}
                onClick={() => setIndex(i)}
              >
                {i + 1}
              </button>
            ))}
          </div>
        </div>
        <aside className="sidebar quiz-sidebar">
          <div className="quiz-head">
            <div className="quiz-title">
              Pregunta {index + 1} de {quiz.questions.length}
            </div>
            <div>{turnLabel(question)}</div>
            <div>
              Rival: #{question.rival.rank} {question.rival.name} · puntos {question.state.points[0]}-{question.state.points[1]} · mano rival{" "}
              {question.state.players[1].hand_count}
            </div>
          </div>
          {question.history.length > 0 && (
            <div className="quiz-history">
              <h2>Lo que pasó antes</h2>
              {question.history.map((h, i) => (
                <div key={i} className={`log-row player-${h.actor}`}>
                  <span className="who">{h.actor === 0 ? "Vos" : "Rival"}</span>
                  <span className="log-text">{h.label}</span>
                </div>
              ))}
            </div>
          )}
          <h2 className="quiz-section">¿Qué hacés ahora?</h2>
          <div className="quiz-hint">El orden dentro del turno no cuenta: si vas a hacer varias cosas, elegí la primera que harías.</div>
          <div className="options-list quiz-options">
            {options.map((o) => (
              <label key={o.index} className={`option-row quiz-option${choice === o.index ? " chosen" : ""}`}>
                <input type="radio" name="quiz-choice" checked={choice === o.index} onChange={() => setChoice(o.index)} />
                <span className="option-text">{o.label}</span>
              </label>
            ))}
          </div>
          <textarea
            className="quiz-note"
            placeholder="Nota (opcional): por qué"
            maxLength={NOTE_MAX}
            value={note}
            onChange={(e) => setNote(e.target.value)}
          />
          <div className="quiz-actions">
            <button type="button" disabled={index === 0} onClick={() => setIndex(index - 1)}>
              Anterior
            </button>
            <button type="button" disabled={last} onClick={() => setIndex(index + 1)}>
              Saltar
            </button>
            <button type="button" className="primary" disabled={choice == null} onClick={submit}>
              {last ? "Guardar" : "Guardar y seguir"}
            </button>
          </div>
          {answered.size === quiz.questions.length && (
            <div className="bot-note quiz-done">
              Listo. El reporte sale con <code>node scripts/deckgym-quiz.mjs --report quiz/{name}.json</code> en tcgpocket.
            </div>
          )}
        </aside>
      </div>
    </div>
  );
}
