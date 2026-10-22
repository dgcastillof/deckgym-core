import fs from 'node:fs'
import path from 'node:path'
import react from '@vitejs/plugin-react'
import { defineConfig, type Connect, type Plugin } from 'vite'

// Serves the repo-root `replays/` folder (one level up from `viewer/`) at `/replays/<file>.json` —
// what `cargo run -- play`'s printed link (`?url=/replays/<game_id>.json`) needs to actually
// resolve while `npm run dev`/`npm run preview` is running. See docs/replay-viewer-plan.md's
// "Round 4" notes and the root README's "Replay viewer" section for the end-to-end flow.
//
// Deliberately narrow: only `/replays/<one path segment>.json` is served (no subdirectories, no
// other extensions, no `..`), and only from that one fixed folder — this is a local dev
// convenience, not a general static file server.
function replaysStaticPlugin(): Plugin {
  const replaysDir = path.resolve(import.meta.dirname, '..', 'replays')

  const middleware: Connect.NextHandleFunction = (req, res, next) => {
    const url = req.url ?? ''
    if (!url.startsWith('/replays/')) {
      next()
      return
    }

    const rawPath = url.split('?')[0].split('#')[0]
    let rel: string
    try {
      rel = decodeURIComponent(rawPath.slice('/replays/'.length))
    } catch {
      res.statusCode = 400
      res.end('Invalid replay path')
      return
    }

    // Exactly one path segment, ending in .json — rejects traversal (`..`, more `/`) outright
    // rather than trying to sanitize it, and rejects anything but the one file type replays are.
    if (!/^[^/\\]+\.json$/.test(rel) || rel.includes('..')) {
      res.statusCode = 400
      res.end('Invalid replay path')
      return
    }

    const resolved = path.resolve(replaysDir, rel)
    if (resolved !== path.join(replaysDir, rel) || !resolved.startsWith(replaysDir + path.sep)) {
      res.statusCode = 400
      res.end('Invalid replay path')
      return
    }

    fs.readFile(resolved, (err, data) => {
      if (err) {
        res.statusCode = 404
        res.end('Replay not found')
        return
      }
      res.setHeader('Content-Type', 'application/json')
      res.end(data)
    })
  }

  return {
    name: 'deckgym-replays-static',
    configureServer(server) {
      server.middlewares.use(middleware)
    },
    configurePreviewServer(server) {
      server.middlewares.use(middleware)
    },
  }
}

// Cuestionario de posiciones (src/quiz/): sirve `/quiz/<archivo>.json` desde DECKGYM_QUIZ_DIR (o la
// carpeta `quiz/` del repo) y acepta PUT de las respuestas. Igual de angosto que el de replays: un
// solo segmento terminado en .json, y el PUT no puede pisar un archivo de preguntas.
const QUIZ_MAX_BYTES = 2 * 1024 * 1024

function quizPlugin(): Plugin {
  const quizDir = path.resolve(process.env.DECKGYM_QUIZ_DIR ?? path.resolve(import.meta.dirname, '..', 'quiz'))

  const middleware: Connect.NextHandleFunction = (req, res, next) => {
    const url = req.url ?? ''
    if (!url.startsWith('/quiz/')) {
      next()
      return
    }
    let rel: string
    try {
      rel = decodeURIComponent(url.split('?')[0].split('#')[0].slice('/quiz/'.length))
    } catch {
      res.statusCode = 400
      res.end('Invalid quiz path')
      return
    }
    const resolved = path.resolve(quizDir, rel)
    if (!/^[^/\\]+\.json$/.test(rel) || rel.includes('..') || !resolved.startsWith(quizDir + path.sep)) {
      res.statusCode = 400
      res.end('Invalid quiz path')
      return
    }

    if (req.method === 'GET' || req.method === 'HEAD') {
      fs.readFile(resolved, (err, data) => {
        if (err) {
          res.statusCode = 404
          res.end('Quiz file not found')
          return
        }
        res.setHeader('Content-Type', 'application/json')
        res.setHeader('Cache-Control', 'no-store')
        res.end(data)
      })
      return
    }

    if (req.method === 'PUT') {
      if (rel.endsWith('-preguntas.json')) {
        res.statusCode = 403
        res.end('Questions files are read-only')
        return
      }
      const chunks: Buffer[] = []
      let size = 0
      req.on('data', (chunk: Buffer) => {
        size += chunk.length
        if (size <= QUIZ_MAX_BYTES) chunks.push(chunk)
      })
      req.on('end', () => {
        const body = Buffer.concat(chunks).toString('utf8')
        let parsed: unknown
        try {
          parsed = JSON.parse(body)
        } catch {
          parsed = null
        }
        if (size > QUIZ_MAX_BYTES || !parsed || typeof parsed !== 'object' || !Array.isArray((parsed as { answers?: unknown }).answers)) {
          res.statusCode = 400
          res.end('Expected an answers JSON')
          return
        }
        fs.mkdir(quizDir, { recursive: true }, () => {
          fs.writeFile(resolved, JSON.stringify(parsed, null, 2) + '\n', (err) => {
            res.statusCode = err ? 500 : 204
            res.end()
          })
        })
      })
      return
    }

    res.statusCode = 405
    res.end()
  }

  return {
    name: 'deckgym-quiz',
    configureServer(server) {
      server.middlewares.use(middleware)
    },
    configurePreviewServer(server) {
      server.middlewares.use(middleware)
    },
  }
}

// https://vite.dev/config/
export default defineConfig({
  plugins: [react(), replaysStaticPlugin(), quizPlugin()],
})
