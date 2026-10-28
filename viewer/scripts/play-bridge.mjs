// Puente del asiento humano (ver play-server.ts): el motor lo corre como bot externo y este proceso
// solo reenvia las lineas de su stdin/stdout al server de vite por TCP local.
//   node play-bridge.mjs <puerto>
import net from 'node:net'

const socket = net.connect(Number(process.argv[2]), '127.0.0.1')
process.stdin.pipe(socket)
socket.pipe(process.stdout)
socket.on('close', () => process.exit(0))
socket.on('error', () => process.exit(1))
process.stdin.on('end', () => socket.end())
