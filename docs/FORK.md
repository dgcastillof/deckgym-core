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

- `work`: rama de trabajo. Es `upstream/main` mas los commits propios (abajo). Es la que se compila y
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
   `scripts/deckgym-lib.mjs` de tcgpocket: si se cambia aca, cambiarlo alla. Desde `c972f98` tambien
   cuenta por jugador las habilidades, ataques y entrenadores usados y los turnos en que cada
   habilidad y ataque estaba disponible (`abilities`, `attacks`, `trainers`, `ability_turns`,
   `attack_turns`; los lee `scripts/deckgym-card-audit.mjs --usage`). Desde el step 2.9, `attack_first_turn`
   (primer turno en que se ofrecio cada ataque) y, con `DECKGYM_STATS_TURNS=1`, `turn_ends` (tablero y mano al final
   de cada turno propio; lo lee `deckgym-card-audit.mjs --turns`).
3. **Bots propios** (`src/players/own_*.rs`, ver `docs/search-design.md`): `o<n>`, busqueda con la respuesta
   del rival (step 2.3); `t<n>`, puntua cada jugada al final del turno propio, completando el resto del turno
   con greedy, para no perder las habilidades gratis antes de atacar (step 2.7); `p<n>`, t<n> con la funcion de
   valor propia de preparacion (`own_value.rs`, `value_params/prep.json` por `DECKGYM_PREP_PARAMS`, step 2.9);
   `d<n>`, t<n> que re-puntua sus mejores jugadas jugando dos turnos propios (step 2.9, experimental: caro).
4. **Tests de auditoria de cartas** (`tests/pokemon/meta_audit_*_test.rs`, step 2.5): las 56 cartas de
   los mazos que el benchmark mas desvia; los `#[ignore = "BUG: ..."]` son bugs conocidos (desde el step
   2.4 queda solo Thieving Incisors, simplificacion a proposito). Resultado en `docs/cartas-sospechosas.md`
   de tcgpocket.
5. **Arreglos mandados upstream y todavia no integrados**, un commit cada uno, que se descartan en el
   rebase cuando upstream los trae:
   - Clear Veil / Crystal Body evitan el descarte de energia por ataques rivales (`fix/clear-veil`,
     PR bcollazo/deckgym-core#427).
   - Dragon's Blessing de Dragonair solo une al activo [N] (`fix/dragons-blessing`, step 2.4; PR
     pendiente de autorizacion).

## Estrategia y reglas del fork

Decision del 2026-10-18: las cartas, las reglas y las expansiones nuevas (por ejemplo el doble tipo de
la expansion C) se toman de upstream, que implementa los sets en ~1-5 semanas con ~90 commits por mes.
Nosotros no mantenemos el motor aparte: nuestros bots van por su cuenta y compiten contra los de
deckgym. Para que eso sea cierto se siguen cuatro reglas:

1. **Nuestros bots viven aparte.** Cada bot propio va en un archivo propio bajo `src/players/` (por
   ejemplo `src/players/own_*.rs`), con codigos propios (`o<n>` y los que sigan) y una funcion de valor
   propia. Nunca se modifican los jugadores de upstream (`r`, `w`, `aa`, `et`, `er`, `v`, `e<n>`, `m`)
   ni su funcion de valor: tienen que seguir siendo la referencia contra la que medimos. Lo unico que
   se toca de `src/players/` en un bot nuevo es `mod.rs`, para registrar el codigo.
2. **Los bots no reimplementan reglas.** Danio, debilidad, tipos (incluido el doble tipo que viene) y
   efectos se le preguntan al motor (aplicando la accion sobre una copia del estado, o con las
   funciones que ya expone). Asi lo nuevo de upstream entra solo con cada rebase.
3. **Los cambios genericos del motor se mandan upstream** como PR: bugs, cartas, arreglos de reglas.
   Lo que se queda solo en el fork es lo que no le sirve a nadie mas (nuestros bots).
4. **Si una carta clave del meta tarda en llegar**, se implementa en el fork en un commit aislado (un
   commit por carta, sin mezclar con otra cosa) y se manda upstream; cuando upstream la trae, el
   commit propio se descarta en el rebase.

`node scripts/deckgym-fork-check.mjs` (en tcgpocket) hace cumplir la regla 1: lista los archivos que
`work` toca respecto de `upstream/main` y falla si alguno es de los jugadores de upstream o de su
funcion de valor. Hay una lista blanca al principio del script con las excepciones ya aceptadas.

Excepcion heredada (no es el modelo a seguir): los cambios de `614ba4f` (funcion de valor calibrable
por JSON, `v` que resuelve elecciones pendientes) tocan `value_function_player.rs` y `value_functions.rs`
de upstream. Estan en la lista blanca porque ya estaban hechos antes de la regla; no se agregan mas
cambios ahi, y la salida limpia es mandarlos upstream (ver abajo) o moverlos a un jugador propio.
La escalera (`scripts/deckgym-ladder.mjs`) marca como referencia a los jugadores de upstream y compara
el mejor nuestro contra el mejor de deckgym.

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
- Los bugs de motor anotados en la auditoria del 2026-10-15 ya estan resueltos (step 2.4, 2026-10-21):
  Boosted Evolution de Eevee lo arreglo upstream (`66ec553`), Teary Attack de Bonsly tambien (#426,
  entro con el rebase del 2026-10-21) y Dragon's Blessing va como `fix/dragons-blessing`.

El motor es AGPL: cualquier cambio publicado en el fork tiene que seguir bajo esa licencia.
