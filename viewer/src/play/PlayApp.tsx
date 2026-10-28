// Partidas contra el bot (`/?play=1`, ver play-server.ts): la persona juega el jugador de abajo, el
// motor juega el rival y calcula todo. Esta pantalla solo pregunta al server que paso desde la ultima
// vez, anima lo nuevo con el tablero de siempre y manda la jugada elegida. Si se recarga la pagina, la
// partida sigue donde estaba (el motor espera la decision pendiente).

import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { Board } from "../board/Board";
import { createCardImageStore, preloadCardImages, resolveCardImageConfig } from "../board/cardImages";
import type { BoardScene } from "../board/scene";
import { PlaybackController } from "../store/playbackController";
import type { Card, GameOutcome, Replay, ReplayPlayerInfo, ReplayStep, SimpleAction, ViewState } from "../types/replay";

const POLL_MS = 500;
const NOTE_MAX = 280;
/** Jugadas que se animan al abrir una partida ya empezada */
const CATCH_UP_STEPS = 4;

interface DeckRef {
  id: string;
  name: string;
  rank: number;
}

interface GameRecord {
  n: number;
  rival: DeckRef;
  outcome?: GameOutcome | null;
  points?: [number, number];
  abandoned?: boolean;
  endedAt?: string;
}

type PublicStep = ReplayStep & { label: string };

interface PendingOption {
  i: number;
  text: string;
  label: string;
  action: SimpleAction;
}

interface StateResponse {
  deck: DeckRef;
  rivals: DeckRef[];
  games: GameRecord[];
  next: DeckRef;
  game: {
    n: number;
    rival: DeckRef;
    total: number;
    start: { players: [ReplayPlayerInfo, ReplayPlayerInfo]; cards: Record<string, Card> } | null;
    steps: PublicStep[];
    pending: { ply: number; state: ViewState; options: PendingOption[] } | null;
    end: { final_state: ViewState; outcome: GameOutcome | null } | null;
    exited: boolean;
    error: string | null;
  } | null;
}

const NO_SUBSCRIBE = () => () => {};

function resultLabel(outcome: GameOutcome | null | undefined): string {
  if (!outcome) return "sin terminar";
  if (outcome === "Tie") return "empate";
  return outcome.Win === 0 ? "ganaste" : "perdiste";
}

/** Las jugadas con el mismo texto son la misma para quien juega: se muestran una vez. */
function uniqueOptions(options: PendingOption[]): PendingOption[] {
  const seen = new Set<string>();
  return options.filter((o) => {
    const key = o.label || o.text;
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
}

export default function PlayApp() {
  const [info, setInfo] = useState<StateResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [scene, setScene] = useState<BoardScene | null>(null);
  const [controller, setController] = useState<PlaybackController | null>(null);
  const [log, setLog] = useState<PublicStep[]>([]);
  const [note, setNote] = useState("");
  const [sending, setSending] = useState(false);
  const images = useMemo(() => createCardImageStore(resolveCardImageConfig()), []);
  // La partida que se esta dibujando: el replay crece a medida que llegan jugadas
  const live = useRef<{ n: number; replay: Replay } | null>(null);
  const controllerRef = useRef<PlaybackController | null>(null);
  const logRef = useRef<HTMLDivElement>(null);

  const playback = useSyncExternalStore(
    useCallback((fn) => (controller ? controller.subscribe(fn) : NO_SUBSCRIBE()), [controller]),
    () => (controller ? controller.getSnapshot() : null),
  );

  const poll = useCallback(async () => {
    const since = live.current?.replay.steps.length ?? 0;
    const res = await fetch(`/play/api/state?since=${since}`);
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    const data = (await res.json()) as StateResponse;
    setInfo(data);
    const g = data.game;
    if (!g || !scene) return;

    // Partida nueva (o primera lectura): se arma el replay desde cero
    if (!live.current || live.current.n !== g.n) {
      if (!g.start) {
        // Todavia no arranco el motor, o cambio la partida: se vuelve a pedir desde el principio
        live.current = null;
        return;
      }
      const replay: Replay = {
        version: 1,
        game_id: `partida-${g.n}`,
        seed: 0,
        players: [
          { name: `Vos: ${data.deck.name}`, deck: g.start.players[0].deck },
          { name: `Bot: #${g.rival.rank} ${g.rival.name}`, deck: [] },
        ],
        cards: g.start.cards,
        steps: [],
        final_state: null,
        outcome: null,
      };
      preloadCardImages(images, Object.keys(g.start.cards));
      live.current = { n: g.n, replay };
      setLog([]);
      if (g.steps.length === 0 && !g.pending) return;
    }

    const replay = live.current.replay;
    if (g.steps.length > 0) {
      replay.steps.push(...g.steps);
      setLog((prev) => [...prev, ...g.steps]);
    }
    if (g.end) {
      replay.final_state = g.end.final_state;
      replay.outcome = g.end.outcome;
    } else if (g.pending) {
      replay.final_state = g.pending.state;
    } else if (g.steps.length > 0) {
      replay.final_state = null;
    }
    if (replay.steps.length === 0 && !replay.final_state) return;

    let c = controllerRef.current;
    if (!c || c.replay !== replay) {
      c?.destroy();
      c = new PlaybackController(scene, replay);
      c.setSpeed(1.6);
      // Al recargar a mitad de partida no se vuelve a animar todo: solo lo ultimo
      c.jumpTo(Math.max(0, c.maxIndex - CATCH_UP_STEPS));
      controllerRef.current = c;
      setController(c);
    }
    c.play();
  }, [scene, images]);

  // Un pedido a la vez
  const polling = useRef(false);
  useEffect(() => {
    const id = window.setInterval(() => {
      if (polling.current) return;
      polling.current = true;
      poll()
        .then(() => setError(null))
        .catch((e: unknown) => setError(`Sin conexión con el server del visor (${String(e)}). Si se cerró, volvé a abrirlo y recargá: la partida en curso se pierde, las terminadas quedan.`))
        .finally(() => (polling.current = false));
    }, POLL_MS);
    return () => window.clearInterval(id);
  }, [poll]);

  useEffect(() => {
    if (logRef.current) logRef.current.scrollTop = logRef.current.scrollHeight;
  }, [log.length]);

  const game = info?.game ?? null;
  const replay = live.current?.replay ?? null;
  const caughtUp = Boolean(playback && controller && !playback.animating && playback.index === controller.maxIndex);
  const pending = game?.pending && caughtUp ? game.pending : null;
  // El resultado se muestra cuando el tablero termino de animar lo ultimo
  const finished = Boolean((game?.end && caughtUp) || (game?.exited && !game.end));

  const choose = async (option: PendingOption) => {
    if (!pending || sending) return;
    setSending(true);
    try {
      const res = await fetch("/play/api/choose", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ ply: pending.ply, i: option.i, note: note.trim().slice(0, NOTE_MAX) }),
      });
      if (!res.ok) throw new Error((await res.json()).error ?? `HTTP ${res.status}`);
      setNote("");
      // Hasta la proxima lectura no hay decision pendiente
      setInfo((prev) => (prev?.game ? { ...prev, game: { ...prev.game, pending: null } } : prev));
    } catch (e) {
      setError(String(e));
    } finally {
      setSending(false);
    }
  };

  const newGame = async (abandon = false) => {
    if (abandon && !window.confirm("¿Abandonar esta partida y empezar otra?")) return;
    const res = await fetch("/play/api/new", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ abandon }) });
    if (!res.ok) setError((await res.json()).error ?? `HTTP ${res.status}`);
  };

  const played = info?.games.filter((g) => g.endedAt && !g.abandoned) ?? [];
  const wins = played.filter((g) => g.outcome && g.outcome !== "Tie" && g.outcome.Win === 0).length;
  const nextRival = info?.next ?? null;
  const state = replay && playback && controller ? (playback.index < replay.steps.length ? replay.steps[playback.index].state : replay.final_state) : null;

  return (
    <div className="app">
      <header className="app-header">
        <div>
          <h1>{info ? `Partidas: ${info.deck.name} contra el bot` : "Partidas contra el bot"}</h1>
          <span className="subtitle">
            Terminadas: {played.length}{played.length > 0 && ` (${wins} ganadas)`}
            {game && ` · partida ${game.n} contra #${game.rival.rank} ${game.rival.name}`}
          </span>
        </div>
        <div className="header-actions">
          {game && !finished && (
            <button type="button" onClick={() => void newGame(true)}>
              Abandonar
            </button>
          )}
        </div>
      </header>
      <div className="app-body quiz-body">
        <div className="main-column quiz-board">
          <div className="app-shell">
            <Board onReady={setScene} images={images} />
          </div>
        </div>
        <aside className="sidebar quiz-sidebar">
          {error && <div className="bot-note quiz-done">{error}</div>}
          {!game && info && (
            <div className="quiz-head">
              <div className="quiz-title">Listo para jugar</div>
              <div>Jugás {info.deck.name} (abajo). El rival lo juega el bot; los rivales van rotando.</div>
              <div className="quiz-actions">
                <button type="button" className="primary" onClick={() => void newGame()}>
                  Empezar contra #{nextRival?.rank} {nextRival?.name}
                </button>
              </div>
            </div>
          )}
          {game && (
            <div className="quiz-head">
              <div className="quiz-title">
                Partida {game.n}: #{game.rival.rank} {game.rival.name}
              </div>
              {state && (
                <div>
                  Turno {state.turn} · puntos {state.points[0]}-{state.points[1]} · mano rival {state.players[1].hand_count} · mazo {state.players[0].deck_count}
                </div>
              )}
            </div>
          )}
          {game && (
            <div className="quiz-history play-log" ref={logRef}>
              <h2>Lo que pasó</h2>
              {log.map((s) => (
                <div key={s.ply} className={`log-row player-${s.actor}`}>
                  <span className="ply">T{s.turn}</span>
                  <span className="who">{s.actor === 0 ? "Vos" : "Bot"}</span>
                  <span className="log-text">{s.label}</span>
                </div>
              ))}
            </div>
          )}
          {game && !finished && !game.end && (
            <>
              <h2 className="quiz-section">{pending ? "Tu jugada" : "Juega el bot…"}</h2>
              {pending && (
                <>
                  <div className="options-list quiz-options">
                    {uniqueOptions(pending.options).map((o) => (
                      <button key={o.i} type="button" className="option-row quiz-option play-option" disabled={sending} onClick={() => void choose(o)}>
                        <span className="option-text">{o.label || o.text}</span>
                      </button>
                    ))}
                  </div>
                  <textarea
                    className="quiz-note"
                    placeholder="Nota (opcional): el plan del turno o por qué. Se guarda con la próxima jugada."
                    maxLength={NOTE_MAX}
                    value={note}
                    onChange={(e) => setNote(e.target.value)}
                  />
                </>
              )}
            </>
          )}
          {game && finished && (
            <div className="quiz-head">
              <div className="quiz-title">
                {game.end ? `Terminó: ${resultLabel(game.end.outcome)} ${game.end.final_state.points[0]}-${game.end.final_state.points[1]}` : "La partida se cortó"}
              </div>
              {game.error && <pre className="play-error">{game.error}</pre>}
              <div className="quiz-actions">
                <button type="button" className="primary" onClick={() => void newGame()}>
                  Siguiente partida: #{nextRival?.rank} {nextRival?.name}
                </button>
              </div>
            </div>
          )}
          {played.length > 0 && (
            <div className="quiz-head">
              {played.map((g) => (
                <div key={g.n}>
                  {g.n}. #{g.rival.rank} {g.rival.name}: {resultLabel(g.outcome)}
                  {g.points && ` ${g.points[0]}-${g.points[1]}`}
                </div>
              ))}
            </div>
          )}
        </aside>
      </div>
    </div>
  );
}
