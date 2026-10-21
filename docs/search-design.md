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

# Completar el turno propio (jugador `t<n>`)

Step 2.7 del roadmap de simulacion de tcgpocket.

## El problema

La profundidad de `e<n>` cuenta acciones, no turnos, y la busqueda corta cuando el turno pasa al
rival. Con `e2`, usar Water Shuriken (Greninja, A1 089) y elegir el objetivo ya gastan las dos
acciones: esa hoja se evalua a mitad de turno, con 20 de dano y sin el ataque que todavia falta.
"Atacar ya" se evalua con el dano completo del ataque. El bot ataca, el turno termina y la habilidad
se pierde (efecto horizonte). En el step 2.5, e2 uso Water Shuriken en el 56-67% de los turnos en
que estaba disponible.

El test `e2_attacks_and_loses_water_shuriken` (`cargo test --lib own_turn`) lo confirma: Bulbasaur
activo con [G][G] (Vine Whip, 40), Greninja en la banca y un Charmander rival con 60 PS. Shuriken mas
Vine Whip lo noquea; e2 ataca directo y no noquea, con la funcion de valor baseline y con la del
step 2.2. Si el shuriken solo ya noquea (un Rattata con 20 PS en la banca), e2 lo ve: el KO entra en
sus dos acciones (`e2_sees_a_water_shuriken_that_knocks_out_by_itself`). El horizonte tapa a la
habilidad solo cuando lo que rinde depende del ataque que viene despues.

## Idea

`t<n>` puntua cada accion legal al final de mi turno, no a mitad:

1. Se aplica la candidata (esperanza sobre las ramas de `forecast_action`, como `e<n>`).
2. Se completa el resto de mi turno: la accion `j` del resto se elige con una busqueda de
   profundidad `max(1, n - 1 - j)` (para `t2`, greedy de un ply con las elecciones encadenadas
   resueltas, como `v` y como el resto del turno en las muestras de `o<n>`). Una eleccion del rival
   dentro de mi turno (promover tras un KO) la hace su greedy.
3. Se evalua la funcion de valor cuando le toca al rival (o termina la partida): el mismo tipo de
   estado que evalua `e<n>` despues de un ataque, asi todas las candidatas se comparan en el mismo
   punto.

Las ramas de azar del resto del turno (monedas) se promedian mientras haya hasta 8 abiertas, contando
las de la candidata; pasado ese tope, cada accion con azar sigue por una sola rama sorteada con su
probabilidad. La informacion oculta no se rearma: igual que `e<n>`, los robos salen del mazo real.

En el setup (`turn_count == 0`) y en las decisiones mias durante el turno rival no hay turno propio
que completar: ahi decide la busqueda de `e<n>`.

Por que asi y no "jugar primero las acciones que no cierran opciones": decidir que acciones son
gratis pide reglas por carta (una habilidad de dano es gratis, una que descarta energia no), y la
regla 2 de `docs/FORK.md` dice que los bots no reimplementan reglas. Completar el turno con la misma
funcion de valor no necesita saber nada de las cartas.

El greedy del resto del turno tiene su propio horizonte (entre "atacar" y "shuriken" a un ply elige
atacar), pero no importa para la decision de la raiz: cada vez que el bot decide, el shuriken es una
candidata que se completa con el ataque. Lo que si se pierde son las lineas de dos o mas acciones
gratis antes del ataque cuando la primera no es la candidata (poco comun).

Costo por decision ~ `acciones legales * ramas * (acciones que quedan en el turno) * acciones legales`:
unas veces `e2`, sin el turno rival de `o<n>`.

## Parametros

Codigo `t<n>` (`t` solo = `t2`). La funcion de valor es la de `DECKGYM_VALUE_PARAMS`, igual que
`e<n>`, `o<n>` y `v`. No tiene variables de entorno propias.

## Mediciones (t<n>)

Tests (`cargo test --lib own_turn`): en la posicion del shuriken e2 ataca directo y no noquea, t2 tira
el shuriken, ataca y noquea (con la funcion de valor baseline y la del 2.2, 3 semillas); si el
shuriken solo ya noquea, los dos lo usan; t2 juega partidas enteras contra e2 con decisiones legales.

Uso (`scripts/deckgym-card-audit.mjs benchmarks/2026-10-18-e2-e2-574afdb.json --usage --players t2,e2`
en tcgpocket, 50 partidas contra el #1, baseline): Water Shuriken por turno disponible 0.99, 1.13 y
1.00 en los tres mazos de Greninja (e2: 0.56, 0.58, 0.67; o2: 0.82, 0.92, 0.86). La disponibilidad de
Terminating Tail, Mega Burning y Dede-Circuit no sube: eso es armar la linea de fase 2 en varios
turnos, no este horizonte.

Escalera (`deckgym-ladder.mjs --players e2,o2,t2 --num 200`, 6 mazos espejo, 2400 partidas por par,
semilla 20261017, `step-2-2.json` en los tres, 12 nucleos):

| Cara a cara | Winrate del primero | IC 95% | ms por partida (los dos jugadores) |
| --- | --- | --- | --- |
| t2 vs e2 | 65.2% | 63.3-67.1% | 136 |
| t2 vs o2 | 58.7% | 56.8-60.7% | 375 |
| o2 vs e2 | 60.2% | 58.3-62.2% | 354 |

e2 contra e2 anda en ~62 ms por partida, asi que t2 cuesta ~3 veces e2 y menos de la mitad que o2.

Benchmark de tcgpocket (`deckgym-benchmark.mjs --players t2,t2 --top 20 --num 200`, meta del 19/10,
`step-2-2.json`, las mismas condiciones que las filas de e2 y o2 de arriba): Pearson 0.321, MAE
15.0 pp, Spearman top 10 -0.33, 83 min. Igual que e2 (0.325; IC 95% bootstrap por pares de la
diferencia -0.10 a +0.11). Los mazos de Greninja no suben (Sceptile -25.3 pp, Blaziken -19.2,
Altaria -14.7): en el espejo los dos lados juegan mejor. Rayquaza vs Lucario baja a 36.5% (real
65.6%). Los matchups de t2 se parecen mas a los de o2 (correlacion 0.92) que a los de e2 (0.79): lo
que o2 cambiaba respecto de e2 venia sobre todo de completar el turno, no de ver al rival.

Conclusion: t2 es el bot mas fuerte y el mas barato de los nuestros, pero como o2 no acerca la
simulacion a los humanos. El desvio de los mazos de Greninja no venia del shuriken perdido.

# Preparar a varios turnos (jugadores `p<n>` y `d<n>`)

Step 2.9 del roadmap de simulacion de tcgpocket.

## Diagnostico

`simulate --stats-json` registra ahora el primer turno en que se ofrecio cada ataque
(`attack_first_turn`) y, con `DECKGYM_STATS_TURNS=1`, el tablero y la mano al final de cada turno
propio (`turn_ends`). `scripts/deckgym-card-audit.mjs --usage --turns` (en tcgpocket) lo resume por
atacante principal. Con t2 y `step-2-2.json`, 100 partidas de cada uno de los 8 mazos del step 2.5
contra el #1 (energia de Limitless, step 2.8):

- Todos los atacantes principales atacan por primera vez 1.3 a 2.8 turnos propios despues de lo mas
  temprano posible (2 si empieza segundo, 3 si empieza primero, para un ataque de dos energias),
  tambien los mazos lineales (Mega Lucario ex 1.4, Mega Manectric ex 1.9).
- Antes de ese turno, lo que mas falta es la evolucion (50-65% de los turnos en los mazos con Mega
  de fase 1 o 2), casi siempre porque la carta no esta o el basico entro ese turno: el bot evoluciona
  cuando puede (pudo y no lo hizo en 1-6% de esos turnos; Mega Sceptile ex 23%, el paso a Grovyle).
- "Energia desviada" (turnos en que, con la energia del turno en la linea, el atacante atacaba el
  siguiente): 0.1-0.2 por partida, salvo Mega Rayquaza ex (0.6, zona de dos tipos). Lo que se puede
  arreglar planificando es chico.

El sesgo concreto que aparecio: `turns_until_opponent_wins` (funcion de valor de upstream) toma como
amenaza el ataque al que menos energia le falta, aunque pegue poco. Unirle la [G] a un Treecko vuelve
a Pound (20) la amenaza y los turnos para ganar saltan, por ejemplo, de 2 a 6: con el coeficiente de
100, cargar al basico de la linea cuesta 400 puntos.

## `p<n>`: funcion de valor de preparacion

t<n> con `own_value::loaded_prep_value_function`: la de upstream mas `prep_damage` (dano que cada
Pokemon va a poder hacer con las evoluciones de la mano, Rare Candy y la energia que le falta,
descontado por turnos) y `turns_to_win` (reemplaza a `turns_until_opponent_wins` eligiendo la amenaza
que noquea antes, contando los turnos que tarda cada forma). Coeficientes y detalle en
`value_params/README.md`. Cuesta ~1.2 veces t2 por partida.

## `d<n>`: dos turnos propios

t<n> puntua todas las jugadas; las `top` mejores se re-puntuan con `samples` determinizaciones como
o<n>: el resto de mi turno con la busqueda que le quedaba a t<n>, el turno del rival con su greedy,
mi turno siguiente con una busqueda de profundidad `next_depth` (con greedy no encuentra "retirar y
atacar con el que se preparo") y se evalua cuando le vuelve a tocar al rival. Variables:
`DECKGYM_TWO_SAMPLES` (2), `DECKGYM_TWO_TOP` (3), `DECKGYM_TWO_DEPTH` (2). La funcion de valor es la de
`DECKGYM_VALUE_PARAMS`.

## Mediciones (p<n> y d<n>)

Tests (`cargo test --lib own_`): en la posicion del Treecko que entro este turno con Rare Candy y Mega
Sceptile ex en la mano, Furfrou (Sharp Fang, [C][C]) activo con [G] y un Chansey de 120 PS, t2 une la
[G] a Furfrou y ataca (50 ahora y 50 despues no noquean) y p2 se la une al Treecko (y retira a Furfrou
para que el Rare Candy del turno que viene caiga sobre el activo: 20 + 130 noquean); si el activo ya
ataca, p2 carga al Treecko; los turnos para ganar cuentan a Mega Sceptile ex del turno que viene; la
mano rival no se usa. d2 en la posicion de Sharp Fang ataca, como t2: el plan de cinco acciones del
turno siguiente no sale en suficientes muestras.

Uso (`deckgym-card-audit.mjs --usage --turns --num 100`, t2 contra p2 con `step-2-2.json`): la energia
desviada baja (Mega Blaziken ex 0.18 -> 0.03, Mega Altaria ex 0.19 -> 0.07, Mega Rayquaza ex 0.60 ->
0.14 turnos por partida) y el primer ataque de Mega Blaziken ex llega medio turno antes; los turnos por
partida en que pueden atacar casi no cambian.

Escalera (meta del 19/10, 6 mazos espejo, `step-2-2.json`):

| Cara a cara | Winrate del primero | IC 95% | ms por partida (los dos jugadores) | Partidas |
| --- | --- | --- | --- | --- |
| p2 vs t2 | 54.2% | 52.3-56.2% | 348 | 2400 |
| d2 vs t2 | 50.7% | 45.0-56.3% | 1649 | 300 |

Costo en un espejo de Mega Sceptile ex + Greninja (24 partidas, tiempo de pared): t2 290 ms, p2 357 ms,
d2 3.4 s con los defaults; 1.7 s con greedy en el turno siguiente y 0.7 s con greedy, 1 muestra y 2
candidatas.

Benchmark top 20 (`--players p2,p2`, meta del 19/10, `step-2-2.json` + `prep.json`): Pearson 0.405,
MAE 12.0 pp, contra 0.454 y 12.3 pp de t2 (diferencia -0.049, IC 95% bootstrap -0.142 a +0.024).

Conclusion: p2 es el bot mas fuerte de los nuestros y corrige el sesgo de `turns_until_opponent_wins`,
pero no acerca la simulacion a los humanos. Con la energia del step 2.8, lo que queda del desvio no
viene de no planificar la linea. d2 no rinde lo que cuesta; queda registrado para relevarlo.
