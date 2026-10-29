import { createRoot } from "react-dom/client";
import { setupGsap } from "./anim/gsap";
import "./index.css";
import App from "./App.tsx";
import QuizApp from "./quiz/QuizApp.tsx";
import PlayApp from "./play/PlayApp.tsx";

// Registered once, before any timeline is built. See anim/gsap.ts.
setupGsap();

// No <StrictMode>: the board is an imperative Pixi/GSAP scene graph that outlives React renders
// (see board/Board.tsx and board/scene.ts). StrictMode's deliberate double-invoking of effects in
// development is designed for code that can safely re-run and clean up; guarding a persistent
// WebGL context + GSAP timeline cache against that would add real complexity for no production
// benefit, so it's left off here.
// `?quiz=<base>` abre el cuestionario de posiciones (quiz/<base>-preguntas.json) en vez del visor
// `?play=1` abre las partidas contra el bot (src/play/, server en play-server.ts)
const params = new URLSearchParams(window.location.search);
const quiz = params.get("quiz");
createRoot(document.getElementById("root")!).render(quiz ? <QuizApp name={quiz} /> : params.has("play") ? <PlayApp /> : <App />);
