# Coeficientes de la funcion de valor

`DECKGYM_VALUE_PARAMS=<ruta a un JSON>` hace que los jugadores `e<depth>` y `v` usen esos
coeficientes (`ValueFunctionParams`, src/players/value_functions.rs). Los campos que falten quedan
con el valor del baseline; un campo desconocido o un archivo invalido abortan. Sin la variable se
usa el baseline (mismo comportamiento que antes para `e<depth>`).

    DECKGYM_VALUE_PARAMS=value_params/best.json ./target/release/deckgym simulate a.txt b.txt --num 100 --players v,v --parallel

- `best.json`: mejor conjunto de la busqueda (coordinate descent, `v,v`, 50 partidas por par, top 10
  del meta B4a de Limitless, objetivo = Pearson entre winrate simulado y real por matchup).
  Scripts en el repo tcgpocket: `scripts/deckgym-calibrate.mjs` (evaluacion) y
  `scripts/deckgym-calibrate-search.mjs` (busqueda).
- `search-log.jsonl`: cada evaluacion de la busqueda (parametros, semilla, metricas).
- `search-console.log`: resumen de la busqueda, coeficiente por coeficiente.
- `validation-e2.txt`: baseline vs `best.json` con `e2,e2`, 200 partidas por par, semilla 31337.

## Resultados (2026-10-15)

Pearson / Spearman por matchup contra los winrates reales (wins/(wins+losses)):

| evaluacion                                   | baseline      | best.json     |
|----------------------------------------------|---------------|---------------|
| `v,v` top 10, 200 partidas, semilla fresca   | 0.25 / 0.20   | 0.44 / 0.43   |
| `v,v` top 20 fuera de la busqueda (105 pares)| 0.17 / 0.15   | 0.21 / 0.17   |
| `e2,e2` top 10, 200 partidas                 | 0.44 / 0.42   | 0.38 / 0.34   |

Lo ganado en top 10 con `v` es en buena parte sobreajuste (fuera de muestra mejora poco) y no
transfiere a `e2`. El cambio que si ayudo es estructural: `v` ahora resuelve sus propias
elecciones pendientes (Rare Candy, Poke Ball...) antes de evaluar; sin eso nunca jugaba Rare Candy.

## Dano real (`real_damage`, step 2.1, 2026-10-18)

`"real_damage": true` hace que `can_ko_active` y `turns_until_opponent_wins` valuen cada ataque con
el dano esperado que calcula el motor (`actions::expected_attack_damage`: la misma prediccion que un
ataque real, con debilidad, banca, estadio, herramientas y monedas por su probabilidad) en vez del
numero impreso. Un Pokemon de la banca se valua como si estuviera de activo. En el baseline es
`false`, asi que sin el campo todo queda igual que antes (lo chequea un test contra la version vieja).

- `real-damage.json`: baseline + `real_damage: true`.

    node scripts/deckgym-benchmark.mjs --players e2,e2 --top 20 --num 200 --params ../deckgym-core/value_params/real-damage.json

Con `e2`, top 20, 200 por par, semilla 20261017: Pearson por matchup 0.272 -> 0.288, MAE 18.2 ->
16.9pp (detalle en benchmarks/README.md de tcgpocket). `e2` queda ~15-20% mas lento por partida.
No cuenta el dano que un efecto hace dentro de su closure ni el de las habilidades (Greninja).

## Energia, descarte, estados y banca (step 2.2, 2026-10-20)

Diagnostico del 15/10: la energia valia solo hasta el costo del ataque mas caro (la 3a y 4a de Mega
Rayquaza ex valian 0), un ataque que descarta energia puntuaba ~-560 si no noqueaba (Rayquaza
cargaba y terminaba el turno sin usar Mega Burst), dormir/paralizar/confundir valian 0 y el bono de
"online" solo miraba al activo. Campos nuevos (todos en 0, o `pokemon_value_energy: true`, en el
baseline, asi que sin ellos nada cambia):

| campo | default | que mide |
| --- | --- | --- |
| `energy_damage` | 0 | por punto de dano: lo que cada Pokemon en juego puede hacer en su proximo ataque (el mejor ataque que paga con una energia mas, o la mitad del mejor con dos mas porque tarda dos turnos), con el tipo de energia mas conveniente del mazo y el dano real si `real_damage`. Tope: el HP restante mas alto del lado rival. Mio menos el del rival |
| `energy_damage_bench` | 0.5 | peso de la banca dentro de `energy_damage` (el activo pesa 1) |
| `pokemon_value_energy` | true | con false, `pokemon_value` es solo HP: la energia la valua `energy_damage` |
| `status_damage` | 0 | por punto de dano que el activo rival deja de hacer en su proximo turno: dormido 50%, paralizado 100%, confundido 50% (dormido y confundido se combinan), con tope en el HP de mi activo. Mio menos el del rival |
| `bench_online` | 0 | Pokemon de la banca que ya pagan un ataque (1) o estan a una energia (0.5) |

Con `energy_damage` la energia vale el dano que habilita: el valor marginal de una energia es cuanto
sube el mejor ataque del proximo turno (Mega Burst: +50 por cada [R]/[L], sin techo en el costo). Y
descartar cuesta el dano futuro que se pierde segun cuanto tarda en recargar: Rayquaza con 4
energias alcanza 250 (190 con tope contra un Lucario); despues de Mega Burst, 50 (dos turnos para
volver a pagar [R][L]). Descartar 1 de varias casi no cuesta (la proxima energia la repone). Si el
rival queda con poco HP en todo su lado, el tope baja y el costo del descarte tambien. El bono fijo
de 500 de `active_pokemon_online_score` era el otro acantilado: con `energy_damage` conviene
bajarlo a 0.

- `step-2-2.json`: `real_damage` + `energy_damage: 2`, `active_damage: 1` y `active_pokemon_online_score: 0`.

Elegido con pruebas cortas (`e2`, 100 partidas por asiento en 4 matchups de Rayquaza/Lucario y
`deckgym-calibrate.mjs --top 20 --num 40`): sacar el bono fijo de online es lo que mas pesa
(Rayquaza dejaba de terminar el turno sin atacar), `energy_damage` 2 le gana a 1 y 3, y
`status_damage` / `bench_online` no mejoraron nada (`bench_online: 150` bajo Rayquaza vs Lucario de
39% a 21%), asi que quedan en 0. Benchmark `e2` top 20, 200 por par, semilla 20261017: Pearson por
matchup 0.288 -> 0.335, MAE 16.9 -> 15.6pp, Rayquaza vs Lucario 43% -> 52% (real 65%); Lucario sigue
+16pp sobrestimado. `e2` queda bastante mas lento (cada evaluacion precia los ataques con 1 y 2
energias de mas). No se corrio `deckgym-calibrate-search.mjs` (best.json sin cambios).

## Preparacion (`prep.json`, step 2.9, 2026-10-21)

Funcion de valor propia del jugador `p<n>` (`src/players/own_value.rs`): la de upstream con
`DECKGYM_VALUE_PARAMS` mas dos terminos con coeficientes en otro JSON, `DECKGYM_PREP_PARAMS`
(`PrepParams`; los campos que falten quedan en el default, que no suma nada). `e<n>`, `v`, `o<n>` y
`t<n>` no la usan.

| campo | default | que mide |
| --- | --- | --- |
| `prep_damage` | 0 | por punto de dano: por cada Pokemon en juego, el mejor ataque que va a poder hacer con las formas a las que llega (la actual, las evoluciones de la mano a un turno, Rare Candy + fase 2 de la mano a un turno, fase 1 + fase 2 de la mano a dos, las del mazo con peso `deck_evolution`), con el dano del motor contra el activo rival y tope en el HP mas alto del lado rival, por `turn_discount^(turnos - 1)`. Turnos = max(1, energia que falta, pasos de evolucion); un tipo que la zona no genera no llega. La banca pesa `bench`. Mio menos el del rival |
| `turns_to_win` | 0 | si no es 0, reemplaza a `turns_until_opponent_wins` de upstream (que queda en 0): mismo calculo de KOs al activo y a la banca hasta 3 puntos, pero la amenaza es la que noquea antes al activo contando los turnos que tarda en atacar cada forma (upstream toma la de menos energia faltante aunque pegue poco: con la [G] en un Treecko, Pound pasa a ser la amenaza y los turnos para ganar saltan de 2 a 6) |
| `turn_discount` | 0.5 | factor por turno de mas en `prep_damage` |
| `bench` | 0.5 | peso de la banca en `prep_damage` |
| `deck_evolution` | 0.25 | peso de una evolucion que hay que robar |
| `max_turns` | 3 | turnos que se miran hacia adelante |
| `lock` | 0 | por punto de dano (step 3.6): para el activo, la mejor forma con un ataque que duerme o paraliza al activo rival (mecanica `InflictStatusConditions` del motor) y que paga en `max_turns`: 0.5 (dormido) o 1 (paralizado) por min(amenaza rival, PS de la forma), mas 1.5 veces el dano de fin de turno a un activo dormido de los Pokemon en juego (`BadDreamsEndOfTurn`), por `turn_discount^(turnos - 1)`. La amenaza es el mejor ataque del activo rival que paga con una energia mas; sin activo rival (armado), los PS de la forma. Mio menos el del rival |

La mano solo se usa del jugador desde cuyo punto de vista se evalua: para el rival, sus evoluciones
salen de mano y mazo juntos con el peso del mazo, y en `turns_to_win` cuenta solo lo que tiene en juego.

- `prep.json`: `prep_damage: 2` (el mismo peso por punto que `energy_damage` en `step-2-2.json`) y
  `turns_to_win: 100` (el coeficiente de upstream). Se usa con `step-2-2.json`:

      DECKGYM_VALUE_PARAMS=value_params/step-2-2.json DECKGYM_PREP_PARAMS=value_params/prep.json ./target/release/deckgym simulate a.txt b.txt --players p2,t2
- `prep-lock.json` (step 3.6, 2026-10-22): `prep.json` mas `lock: 8`. Sale del cuestionario de posiciones de tcgpocket
  (quiz/README.md): el usuario pone adelante al que duerme gratis y deja a los Darkrai en la banca; el bot ponia al de mas PS.
  Con `lock: 8` p2 pone a Igglybuff adelante en el armado de Mega Altaria ex + Espeon en 41 de 81 partidas (antes 20) y nunca a
  Darkrai (antes 26). Medicion contra Limitless en `benchmarks/` de tcgpocket.
- `prep-8-2.json` (step 8.2, vuelta 1, 2026-10-27; titular desde ese dia): `prep-2-12-bench.json` mas `choices: true`. Los
  ataques que eligen su dano en el efecto (Chase Order de Vespiquen ex: 70, o 140 descartando un basico [G] de la banca;
  "este ataque hace X a 1 de los Pokemon rivales"; Wild Swing) no ponen el dano en el resultado sino que encolan la eleccion,
  y `expected_attack_damage` daba 0: p2 no veia a Vespiquen ex como amenaza y no la evolucionaba ni la cargaba. Con `choices`
  el precio corre el efecto sobre una copia y toma la mejor de las opciones de dano al activo que encola el motor
  (`AttackOutcomes::expected_damage_to_opponent_active_with_choices`); no cambia nada con el campo en `false`. La lista de
  ataques que cambian sale de `cargo test choices_audit -- --ignored --nocapture` (~55; en el top 20 del 19/10: Chase
  Order, Diving Icicles de Chien-Pao ex, Shadow Bullet de Hoopa ex, Jump Blues de Pheromosa). Medicion en tcgpocket,
  docs/mejoras.md (registro de vueltas): MAE ponderado 11,95 -> 10,53 y 10,40 pp con dos semillas, Pearson 0,41 -> 0,50 y 0,52.
