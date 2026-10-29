// Partidas contra el bot en el visor (`/?play=1`, step 3.7 del roadmap de tcgpocket): el server de vite
// lanza `deckgym play` con el jugador A como bot externo (`x`, docs/bot-protocol.md) y le pasa a ese bot
// las jugadas que la persona elige en el navegador. El motor calcula todo (reglas, jugadas legales,
// el rival); el visor solo dibuja la mesa y manda el indice elegido.
//
// Se activa con DECKGYM_PLAY_CONFIG=<config.json>, que escribe `scripts/deckgym-play.mjs` en tcgpocket:
//   { exe, outDir, deck: {id, name, file}, rivals: [{id, name, rank, file}], players, env, labels, seed }
//
// Cadena: motor --(stdin/stdout del bot externo)--> scripts/play-bridge.mjs --(TCP local)--> este
// server --(HTTP)--> navegador. Con DECKGYM_REPLAY_LIVE el motor agrega cada jugada a
// <partida>/<game_id>.live.jsonl, de donde sale lo que hizo el rival entre una decision y otra; con
// DECKGYM_EXTERNAL_SHADOW=p2 el replay final guarda en `scores` lo que habria valuado p2 cada jugada
// de la persona (eso nunca llega al navegador).
//
// Rutas: GET /play/api/state?since=<n> (sesion, jugadas nuevas desde la n, decision pendiente),
// POST /play/api/choose {ply, i, note}, POST /play/api/new {abandon?}.

import { spawn, type ChildProcess } from 'node:child_process'
import fs from 'node:fs'
import net from 'node:net'
import path from 'node:path'
import { pathToFileURL } from 'node:url'
import type { Connect, Plugin } from 'vite'

interface DeckRef {
  id: string
  name: string
  rank: number
  file: string
}

interface PlayConfig {
  exe: string
  outDir: string
  deck: DeckRef
  rivals: DeckRef[]
  players: string
  env: Record<string, string>
  /** Modulo con `actionLabel(action, state, cards, actor)` (scripts/lib/action-labels.mjs) */
  labels: string
  seed: number
}

interface GameRecord {
  n: number
  rival: { id: string; name: string; rank: number }
  seed: number
  dir: string
  startedAt: string
  gameId?: string
  endedAt?: string
  outcome?: unknown
  points?: [number, number]
  abandoned?: boolean
  errors?: number
}

type Json = Record<string, unknown>
type ActionLabel = (action: unknown, state: unknown, cards: unknown, actor: number) => string

const HUMAN_TIMEOUT_MS = 7 * 24 * 3600 * 1000
const BODY_MAX = 64 * 1024

export function playPlugin(): Plugin | null {
  const configPath = process.env.DECKGYM_PLAY_CONFIG
  if (!configPath) return null
  const config = JSON.parse(fs.readFileSync(configPath, 'utf8')) as PlayConfig
  const sessionFile = path.join(config.outDir, 'session.json')
  const bridge = path.resolve(import.meta.dirname, 'scripts', 'play-bridge.mjs')
  if (/\s/.test(bridge)) throw new Error(`La ruta del puente no puede tener espacios: ${bridge}`)

  let actionLabel: ActionLabel = (_a, _s, _c, _actor) => ''
  void import(pathToFileURL(config.labels).href).then((m: { actionLabel: ActionLabel }) => (actionLabel = m.actionLabel))

  const games: GameRecord[] = fs.existsSync(sessionFile) ? (JSON.parse(fs.readFileSync(sessionFile, 'utf8')).games as GameRecord[]) : []
  // Una partida que quedo a medias cuando se cerro el server no se puede retomar (el motor murio)
  for (const g of games) if (!g.outcome) g.abandoned = true
  const saveSession = () => {
    fs.mkdirSync(config.outDir, { recursive: true })
    fs.writeFileSync(sessionFile, JSON.stringify({ deck: config.deck, rivals: config.rivals, games }, null, 2) + '\n')
  }

  // La partida en curso (una a la vez)
  let current: {
    game: GameRecord
    child: ChildProcess
    socket: net.Socket | null
    pending: { ply: number; state: Json; actions: { i: number; text: string; action: unknown }[] } | null
    exited: boolean
    log: string
  } | null = null

  const server = net.createServer((socket) => {
    if (!current || current.socket) {
      socket.destroy()
      return
    }
    const game = current
    game.socket = socket
    let buffer = ''
    socket.setEncoding('utf8')
    socket.on('data', (chunk: string) => {
      buffer += chunk
      let nl: number
      while ((nl = buffer.indexOf('\n')) >= 0) {
        const line = buffer.slice(0, nl).trim()
        buffer = buffer.slice(nl + 1)
        if (!line) continue
        const msg = JSON.parse(line) as Json
        if (msg.type === 'hello') socket.write(JSON.stringify({ type: 'hello', name: 'Persona' }) + '\n')
        else if (msg.type === 'decide') game.pending = { ply: msg.ply as number, state: msg.state as Json, actions: msg.actions as never }
      }
    })
    socket.on('close', () => {
      if (game.socket === socket) game.socket = null
    })
    socket.on('error', () => {})
  })
  const ready = new Promise<number>((resolve) => server.listen(0, '127.0.0.1', () => resolve((server.address() as net.AddressInfo).port)))

  // Rota entre los rivales; una partida abandonada no cuenta, se repite ese rival
  const nextRival = () => config.rivals[games.filter((g) => !g.abandoned).length % config.rivals.length]

  async function startGame() {
    const port = await ready
    const n = games.length + 1
    const rival = nextRival()
    const dir = path.join(config.outDir, String(n).padStart(2, '0'))
    fs.mkdirSync(dir, { recursive: true })
    const game: GameRecord = { n, rival: { id: rival.id, name: rival.name, rank: rival.rank }, seed: config.seed + n, dir, startedAt: new Date().toISOString() }
    games.push(game)
    saveSession()
    const args = [
      'play', config.deck.file, rival.file,
      '--players', config.players,
      '--bot-a', `node ${bridge} ${port}`,
      '--bot-timeout-ms', String(HUMAN_TIMEOUT_MS),
      '--seed', String(game.seed),
      '--replay-dir', dir,
    ]
    const child = spawn(config.exe, args, {
      cwd: path.dirname(config.exe),
      env: { ...process.env, ...config.env, DECKGYM_REPLAY_LIVE: '1' },
      stdio: ['ignore', 'pipe', 'pipe'],
      windowsHide: true,
    })
    const state = { game, child, socket: null, pending: null, exited: false, log: '' }
    current = state
    const keep = (d: Buffer) => (state.log = (state.log + d.toString()).slice(-4000))
    child.stdout?.on('data', keep)
    child.stderr?.on('data', keep)
    child.on('close', () => {
      state.exited = true
      state.pending = null
      finishGame(game)
    })
  }

  // Al terminar: el resultado sale del replay final
  function finishGame(game: GameRecord) {
    game.endedAt = new Date().toISOString()
    const file = fs.readdirSync(game.dir).find((f) => f.endsWith('.json'))
    if (file) {
      const replay = JSON.parse(fs.readFileSync(path.join(game.dir, file), 'utf8'))
      game.gameId = replay.game_id
      game.outcome = replay.outcome
      game.points = replay.final_state?.points
      game.errors = replay.steps.filter((s: { note?: string }) => s.note?.startsWith('[bot error')).length
    }
    saveSession()
  }

  // Lo que el motor ya escribio de la partida, tapado para la persona: sin la mano del rival, sin
  // las opciones del rival (dicen que tiene en la mano) y sin los puntajes de la sombra
  function readLive(game: GameRecord) {
    const file = fs.readdirSync(game.dir).find((f) => f.endsWith('.live.jsonl'))
    if (!file) return null
    const lines = fs.readFileSync(path.join(game.dir, file), 'utf8').split('\n').filter(Boolean)
    let start: Json | null = null
    let end: Json | null = null
    const steps: Json[] = []
    for (const raw of lines) {
      let msg: Json
      try {
        msg = JSON.parse(raw)
      } catch {
        continue // la ultima linea a medio escribir
      }
      if (msg.type === 'start') start = msg
      else if (msg.type === 'step') steps.push(msg.step as Json)
      else if (msg.type === 'end') end = msg
    }
    return { start, steps, end }
  }

  const hideRivalHand = (state: Json): Json => {
    const players = (state.players as Json[]).map((p, i) => (i === 1 ? { ...p, hand: null } : p))
    return { ...state, players }
  }

  function publicStep(step: Json, cards: unknown) {
    const options = step.options as { text: string; action: unknown }[]
    const chosen = options[step.chosen as number]
    const label = chosen ? actionLabel(chosen.action, step.state, cards, step.actor as number) : '?'
    return {
      ply: step.ply,
      turn: step.turn,
      actor: step.actor,
      state: hideRivalHand(step.state as Json),
      options: chosen ? [chosen] : [],
      chosen: 0,
      note: null,
      label,
    }
  }

  function stateResponse(since: number) {
    const base = { deck: config.deck, rivals: config.rivals, games, next: nextRival() }
    if (!current) return { ...base, game: null }
    const { game, pending, exited } = current
    const live = readLive(game)
    const cards = live?.start?.cards ?? {}
    // El armado es simultaneo en el juego: hasta que termina, no se ve lo que puso el rival (el motor
    // lo arma primero)
    const all = live?.steps ?? []
    const setupOver = Boolean(live?.end) || all.some((s) => (s.turn as number) > 0)
    const firstRival = all.findIndex((s) => s.actor === 1)
    const visible = setupOver || firstRival < 0 ? all.length : firstRival
    const steps = all.slice(since, visible).map((s) => publicStep(s, cards))
    const hideRivalBoard = (state: Json): Json =>
      setupOver ? state : { ...state, players: (state.players as Json[]).map((p, i) => (i === 1 ? { ...p, in_play: [null, null, null, null] } : p)) }
    return {
      ...base,
      game: {
        n: game.n,
        rival: game.rival,
        total: visible,
        start: since === 0 && live?.start ? { players: live.start.players, cards } : null,
        steps,
        pending: pending
          ? {
              ply: pending.ply,
              state: hideRivalBoard(pending.state),
              options: pending.actions.map((a) => ({ i: a.i, text: a.text, action: a.action, label: actionLabel(a.action, pending.state, cards, 0) })),
            }
          : null,
        end: live?.end ? { final_state: hideRivalHand(live.end.final_state as Json), outcome: live.end.outcome } : null,
        exited,
        error: exited && !live?.end ? current.log.slice(-1500) : null,
      },
    }
  }

  const readBody = (req: Connect.IncomingMessage) =>
    new Promise<Json>((resolve, reject) => {
      let body = ''
      req.on('data', (c: Buffer) => {
        body += c
        if (body.length > BODY_MAX) reject(new Error('body too large'))
      })
      req.on('end', () => {
        try {
          resolve(body ? JSON.parse(body) : {})
        } catch (e) {
          reject(e)
        }
      })
    })

  const send = (res: import('node:http').ServerResponse, code: number, body: unknown) => {
    res.statusCode = code
    res.setHeader('Content-Type', 'application/json')
    res.setHeader('Cache-Control', 'no-store')
    res.end(JSON.stringify(body))
  }

  const middleware: Connect.NextHandleFunction = (req, res, next) => {
    const url = new URL(req.url ?? '', 'http://localhost')
    if (!url.pathname.startsWith('/play/api/')) return next()
    const route = url.pathname.slice('/play/api/'.length)
    ;(async () => {
      if (route === 'state' && req.method === 'GET') return send(res, 200, stateResponse(Number(url.searchParams.get('since') ?? 0)))
      if (route === 'choose' && req.method === 'POST') {
        const body = await readBody(req)
        const pending = current?.pending
        if (!current || !pending || !current.socket || body.ply !== pending.ply) return send(res, 409, { error: 'No hay una decision pendiente con ese ply' })
        const i = Number(body.i)
        if (!Number.isInteger(i) || i < 0 || i >= pending.actions.length) return send(res, 400, { error: 'Indice fuera de rango' })
        const note = typeof body.note === 'string' && body.note.trim() ? body.note.trim().slice(0, 280) : undefined
        current.pending = null
        current.socket.write(JSON.stringify({ i, note }) + '\n')
        return send(res, 200, { ok: true })
      }
      if (route === 'new' && req.method === 'POST') {
        const body = await readBody(req)
        if (current && !current.exited) {
          if (!body.abandon) return send(res, 409, { error: 'Hay una partida en curso' })
          current.game.abandoned = true
          current.child.kill()
          current.socket?.destroy()
        }
        await startGame()
        return send(res, 200, { ok: true })
      }
      send(res, 404, { error: 'Ruta desconocida' })
    })().catch((e: unknown) => send(res, 500, { error: String(e) }))
  }

  const stop = () => {
    if (current && !current.exited) {
      current.game.abandoned = true
      current.child.kill()
    }
    server.close()
  }
  process.once('exit', stop)

  return {
    name: 'deckgym-play',
    configureServer(s) {
      s.middlewares.use(middleware)
      s.httpServer?.once('close', stop)
    },
  }
}
