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
