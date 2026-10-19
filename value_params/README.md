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
