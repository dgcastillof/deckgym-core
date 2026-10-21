# Busqueda con la respuesta del rival (jugador `o<n>`)

Step 2.3 del roadmap de simulacion de tcgpocket. Este documento es el diseno; los numeros medidos
estan al final.

## El problema

`ExpectiMiniMaxPlayer` (`e<n>`) corta la busqueda apenas el turno pasa al rival
(`state.current_player != myself` en `expectiminimax`). Con `e2` eso significa: hasta dos acciones
propias del mismo turno y despues la funcion de valor estatica, en el estado "le toca al rival, con
el robo pendiente". Nunca ve lo que el rival puede hacer con su turno: si me noquea el activo, si
le alcanza la energia, si conviene retirar. Mas profundidad no ayuda porque las acciones extra
siguen siendo mias (Rayquaza vs Lucario: e2 33%, e3 27%, e4 31%, real 65%).

## Idea

`o<n>` = la misma busqueda propia de `e<n>` para elegir candidatos, mas una simulacion del turno
del rival para puntuar los mejores:

1. **Fase 1 (candidatos).** Se corre exactamente la busqueda de `e<n>` sobre las acciones legales y
   se ordenan por ese puntaje. Si hay una sola accion legal, o estamos en el setup
   (`turn_count == 0`), se devuelve lo de `e<n>` y listo.
2. **Fase 2 (respuesta del rival).** Las `top` mejores (por defecto 3) se re-puntuan con `k`
   muestras (por defecto 2, ver Mediciones). Cada muestra:
   - **Determinizacion.** Se clona el estado y se rearma la informacion oculta de forma
     consistente con lo visible: la mano y el mazo del rival se juntan, se mezclan y se reparte
     una mano del mismo tamano (el resto es su mazo, en orden aleatorio); mi mazo se mezcla (yo
     tampoco se el orden). Lo visible (mesa, descartes, energias, puntos, tamano de mano) no se
     toca. Supuesto: el bot conoce la lista del mazo rival (mano + mazo = lista menos lo visto),
     lo razonable en simulaciones del meta donde los dos mazos son conocidos.
     Excepcion: si hay elecciones pendientes en la pila (`move_generation_stack`) no se rearma nada,
     porque esas acciones pueden nombrar cartas concretas de una mano revelada (Mega Absol ex:
     "descarta este Partidario de la mano rival") o del mazo; rearmarlas hacia que el motor entre en
     panico ("Player hand should contain card to remove"). Son decisiones cortas dentro de un efecto.
   - **Mi resto de turno.** Se aplica el candidato con `apply_action` (el azar se muestrea de la
     distribucion exacta) y se sigue jugando mi turno con la misma profundidad que le quedaba a
     `e<n>`: la accion `j` del resto del turno se elige con una busqueda de profundidad
     `max(1, n - 1 - j)` (para `o2` es siempre greedy de un ply, que es justo lo que hace el
     segundo ply de `e2`). Asi `o<n>` no juega su propio turno peor que `e<n>`.
   - **Turno del rival.** Se juega su turno entero con su mejor accion segun la misma funcion de
     valor desde su punto de vista: busqueda de profundidad `opp_depth` (por defecto 1 = greedy de
     un ply, como `v`, resolviendo las elecciones encadenadas tipo Pokeball/Rare Candy). Sus
     robos salen del mazo determinizado, sus monedas de `apply_action`. Si en su turno me toca
     decidir a mi (promover un activo tras un KO) decido con greedy de un ply.
   - **Corte.** Cuando empieza mi turno siguiente (`current_player == myself` y `turn_count` subio
     dos), o termina la partida, o se pasa el tope de 80 acciones, se evalua la funcion de valor
     desde mi punto de vista. Es el mismo tipo de estado que evalua `e<n>` (principio de turno con
     el robo pendiente), un turno mas tarde.
   - Las `k` muestras se promedian con peso igual (cada una ya es una muestra de la distribucion
     conjunta de mano oculta, robos y monedas).
3. **Eleccion.** Gana el candidato con mayor promedio de fase 2; un empate lo desempata el orden de
   fase 1.

**Numeros aleatorios comunes.** La muestra `i` usa la misma semilla para todos los candidatos
(misma mano del rival, mismo orden de mazo), asi la diferencia entre candidatos no es ruido de
determinizacion. Las semillas salen del rng de la partida, asi que una partida con la misma semilla
es reproducible.

## Por que esta poda

- La explosion combinatoria del turno rival (varias acciones, cada una con ramas de azar, sobre `k`
  manos posibles) hace imposible un minimax completo. Se reemplaza por una muestra de la linea
  principal: en cada nodo propio o rival se sigue solo la mejor accion (la que elegiria esa
  politica), no todas.
- Solo los `top` mejores candidatos de fase 1 pagan la simulacion: el orden por valor estatico ya
  separa las jugadas malas (pasar el turno con energia en la mano, etc.) y la respuesta del rival
  sirve para desempatar entre las buenas (retirar o no, a quien cargarle energia, atacar o
  preparar).
- Costo por decision ~ `top * k * (acciones que quedan en el turno + acciones del turno rival) *
  acciones legales * ramas`, con greedy de un ply: decenas de evaluaciones por muestra, no miles.

## Parametros

Codigo `o<n>` (`o` solo = `o2`). Variables de entorno (se leen al crear el jugador):

| Variable | Default | Que es |
| --- | --- | --- |
| `DECKGYM_OPP_SAMPLES` | 2 | `k`, muestras (determinizaciones) por candidato |
| `DECKGYM_OPP_TOP` | 3 | candidatos de fase 1 que se re-puntuan |
| `DECKGYM_OPP_DEPTH` | 1 | profundidad de la busqueda del rival en cada decision de su turno |

La funcion de valor es la de `DECKGYM_VALUE_PARAMS`, igual que `e<n>` y `v`.

Si el costo se va: bajar `k` antes que la profundidad (regla del roadmap); despues `top`.

## Mediciones

Escalera de tcgpocket (`scripts/deckgym-ladder.mjs --players e2,o2 --num 200`), 6 mazos espejo del meta,
200 partidas por mazo y asiento (2400 en total), semilla 20261017, `value_params/step-2-2.json` en los
dos jugadores, 12 nucleos. ms por partida es tiempo de pared e incluye a e2 (~60 ms).

| k | top | rival | o2 vs e2 | IC 95% | ms por partida |
| --- | --- | --- | --- | --- | --- |
| 3 | 3 | greedy | 61.7% | 59.7-63.6% | 528 |
| 2 | 3 | greedy | 60.1% | 58.2-62.1% | 391 |

Trade-off: con k=3 gana 1.6 pp mas pero se pasa del presupuesto de 500 ms; con k=2 entra en los dos
criterios (>= 60%, <= 500 ms), asi que el default queda en 2. El mazo mas caro (#5 del meta) anda en
~0.9 s por partida con k=2 y ~1 s con k=3, con ~9% de empates. La profundidad del rival (`DECKGYM_OPP_DEPTH=2`) no se probo en la escalera: multiplica el
costo del turno rival por la cantidad de acciones legales y no entra en el presupuesto.

La escalera se corrio antes del arreglo de la pila (ninguno de los 6 mazos tiene Mega Absol ex,
que es el que lo disparo en el benchmark); el arreglo solo cambia las decisiones con elecciones
pendientes.

Benchmark de tcgpocket (`scripts/deckgym-benchmark.mjs --top 20 --num 200`, 151 matchups del meta del
19/10 contra los winrates reales de Limitless, semilla 20261017, `step-2-2.json`):

| Jugador | Pearson | MAE | Spearman ranking top 10 | Lucario | Rayquaza vs Lucario (real 65.6%) | Tiempo |
| --- | --- | --- | --- | --- | --- | --- |
| e2 | 0.325 | 15.5 pp | -0.43 | +16.1 pp | 46.5% | 39 min |
| o2 | 0.375 | 15.2 pp | -0.52 | +17.2 pp | 38.5% | 5.4 h |

o2 sube el Pearson 0.05, pero con un IC 95% bootstrap por pares de -0.03 a +0.12 (no significativo), y
queda lejos del 0.60 del roadmap. Cambia mucho los matchups (hasta 15-20 pp por par), pero la mitad hacia
lo real y la mitad para el otro lado (55 pares mas cerca, 57 mas lejos). Conclusion: ver la respuesta del
rival hace un bot mas fuerte (60% contra e2), pero no hace que la simulacion se parezca mas a los
humanos. El error sistematico (Lucario +17, Rayquaza abajo, Greninja abajo) no viene del horizonte de
busqueda: hay que buscarlo en como el motor resuelve cartas concretas (steps 2.4 y 2.5) o en lo que mira
la funcion de valor.

Tests (`cargo test --lib own_reply`): en una posicion donde el Charmander rival noquea al Bulbasaur
herido el turno siguiente, o2 lo retira al Bulbasaur sano de la banca y e2 no; sin muestras o2 elige
igual que e2 en todas las decisiones de una partida (la busqueda copiada es fiel); la determinizacion
conserva lo visible; con una eleccion pendiente sobre la mano rival revelada no se determiniza.
