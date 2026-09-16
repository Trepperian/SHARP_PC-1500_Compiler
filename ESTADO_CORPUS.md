# Estado del corpus `test/basic/`

Clasificación de los 39 programas BASIC de `test/basic/` según el resultado de
compilarlos con `dull --native-code`, tras aplicar los arreglos D1a, D1b y D10
(saltos a líneas inexistentes, `FOR` sin `NEXT`, e `IF` con consecuente vacío).

Presupuesto de RAM de usuario real: **18176 bytes** (`0x0100`–`0x47FF`, Sharp
PC-1500 con expansión CE-161).

## Grupo 1 — Compilan bien y caben en memoria (19)

| Programa | Bytes | Margen libre |
|---|---:|---:|
| test_simple | 382 | 17794 |
| test_decimals | 989 | 17187 |
| pacman | 3620 | 14556 |
| mole | 4606 | 13570 |
| bathyscaph | 5723 | 12453 |
| rasemottes | 6242 | 11934 |
| jackpot | 6414 | 11762 |
| Pilesjr | 6850 | 11326 |
| bombing | 7788 | 10388 |
| bowling | 7858 | 10318 |
| labyrinthe | 11245 | 6931 |
| donkey-kong | 11581 | 6595 |
| micromur | 12663 | 5513 |
| morpion | 13033 | 5143 |
| aterrissage | 13134 | 5042 |
| trio | 13522 | 4654 |
| slalom | 16996 | 1180 |
| invader-v2 | 17189 | 987 |
| bataille-dans-l-espace | 17694 | 482 |

## Grupo 2 — Compilan pero exceden memoria (18)

| Programa | Bytes | Exceso |
|---|---:|---:|
| meteorites | 18549 | +373 |
| course | 22604 | +4428 |
| jeu-des-blocs | 23637 | +5461 |
| tank | 26592 | +8416 |
| dames | 28091 | +9915 |
| simulateur-de-vol | 29423 | +11247 |
| tempter | 31841 | +13665 |
| othello | 33005 | +14829 |
| minenboot | 37172 | +18996 |
| loup-des-mers | 39088 | +20912 |
| DungeonQuest | 39527 | +21351 |
| monstres&merveilles | 40490 | +22314 |
| formula1 | 41314 | +23138 |
| decathlon | 48815 | +30639 |
| ghosthouse | 51878 | +33702 |
| blackjack | 57851 | +39675 |
| scrabble | 91606 | +73430 (⚠️ excede el límite de 16 bits del `.lh5`, no se escribe) |
| gloupman | 124047 | +105871 (⚠️ excede el límite de 16 bits del `.lh5`, no se escribe) |

## Grupo 3 — Fallan al compilar (2)

| Programa | Causa |
|---|---|
| battlecars | Identificador `VMA` de 3 caracteres, no soportado (máximo 2 + `$` opcional) |
| force | Elemento vacío en lista `ON...GOTO` (`,,` en la lista de destinos) |

