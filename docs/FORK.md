# Fork propio de deckgym-core

Este repo es el fork de [bcollazo/deckgym-core](https://github.com/bcollazo/deckgym-core) que usa el
tracker personal tcgpocket para simular mazos de Pokemon
TCG Pocket (sugeridor de mazos para eventos, calibracion de la funcion de valor). Los scripts que
lo manejan viven en tcgpocket (`scripts/README-deckgym.md`); aca solo estan los cambios del motor.

## Remotes

| remote     | repo                                   | uso                                  |
|------------|----------------------------------------|--------------------------------------|
| `origin`   | dgcastillof/deckgym-core (el fork)     | donde se pushea                      |
| `upstream` | bcollazo/deckgym-core (el original)    | de donde se baja y se sincroniza     |

## Ramas

- `work`: rama de trabajo. Es `upstream/main` mas dos commits propios (abajo). Es la que se compila y
  usa tcgpocket (`../deckgym-core/target/release/deckgym.exe`).
- `backup/calibrate-value-2026-10`: la rama vieja `calibrate-value` tal cual estaba el 2026-10-18
  (basada en el commit 4e25889 de upstream, 1559 commits atras). Solo respaldo; no se toca.
- `main`: espejo de upstream, no tiene cambios propios.

## Que tiene `work` distinto de upstream

1. **Funcion de valor calibrable** (`src/players/value_functions.rs`, `value_function_player.rs`,
   `value_params/`): `DECKGYM_VALUE_PARAMS=<json>` carga los coeficientes de los jugadores `v` y
   `e<n>` (ver `value_params/README.md`); ocho features nuevas; el jugador `v` resuelve las
   elecciones pendientes (Rare Candy, Poke Ball...) antes de evaluar; `--seed` deriva una semilla
   distinta por partida en vez de repetir la misma N veces.
2. **`simulate --stats-json <archivo>`** (`src/event_stats.rs`, `src/state/tally.rs`, ganchos en
   `apply_action_helpers.rs`): un registro por partida con KOs al activo por tipo, dano maximo de un
   ataque y Pokemon puestos en juego por fase. El formato es el que lee `runStats` en
   `scripts/deckgym-lib.mjs` de tcgpocket: si se cambia aca, cambiarlo alla.

## Sincronizar con upstream

    git fetch upstream
    git checkout work
    git rebase upstream/main
    cargo build --release && cargo test
    git push --force-with-lease origin work

Son pocos commits propios, asi que el rebase es barato. Si algo choca, los diffs de cada cambio se ven
con `git show <commit>`. Despues de un rebase hay que recompilar (~8 min) y correr los dos comandos de
humo de tcgpocket: `node scripts/simulate-events.mjs --event garchomp --num 20` (restaurar luego
`public/events/sim.json` con `git checkout`) y
`node scripts/deckgym-calibrate.mjs --top 4 --players v,v --num 20 --params ../deckgym-core/value_params/best.json`.

## Que se podria mandar a upstream

- El arreglo de `--seed` (cada partida con su semilla derivada): independiente, chico y util a
  cualquiera que quiera simulaciones reproducibles.
- `v` resolviendo elecciones pendientes antes de evaluar: sin eso nunca juega Rare Candy.
- `--stats-json`: general si se reemplaza el vocabulario de tareas de eventos por algo configurable.
- Los coeficientes calibrables por JSON: util, pero `best.json` esta sobreajustado al top 10 del meta
  B4a de Limitless y no conviene mandarlo como default (ver `value_params/README.md`).
- Los bugs de motor anotados en la auditoria del 2026-10-15 (Teary Attack de Bonsly, Boosted
  Evolution de Eevee, Dragon's Blessing sin chequeo de tipo) conviene revisarlos contra upstream
  antes de reportarlos: pueden estar arreglados despues de tantos commits.

El motor es AGPL: cualquier cambio publicado en el fork tiene que seguir bajo esa licencia.
