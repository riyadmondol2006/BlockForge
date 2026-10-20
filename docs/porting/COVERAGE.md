# Identifier coverage (Bedrock -> Java)

Generated from `cargo test --lib convert::mapping::validation -- --nocapture` (`src/convert/mapping/validation*.rs`), which pushes
every vanilla block, item and id of Chunker's `cli/data` (51 Java versions 1.13..26.3, 58 Bedrock versions 1.12.0..1.21.130) through
our tables. Scope is the Bedrock -> Java direction only: Bedrock ids and states to the pivot, and the pivot to Java ids and states.
Set `CHUNKER_DATA` to the data folder; the tests skip with a note when it is missing. Total runtime about 11 s (16 cores).

Cells are `checked / allowed / failed`. `allowed` are the expected non-passes: Chunker's own allow-lists
(`UNSUPPORTED_BLOCKS`, `LOSSY_BLOCKS`, `removeLossyStates`, `UNSUPPORTED_ITEMS`, `LOSSY_ITEMS`, ported verbatim in `validation_lossy.rs`) and,
for the cross-edition and table checks, content that the target version does not have (`newer:`) or that no Java version has (`gap:`).

## Result

**0 failures in every check on every version.** No table bug was found by these tests: each Bedrock block state of each Bedrock
version maps to a pivot block with valid states, each pivot block state combination of each Java version (full product, 46,550
combinations over 1,292 types) maps to a Java block state that exists in that version's `blocks.json`, and the Bedrock -> Java
end-to-end check (Bedrock version x closest Java release and latest Java) has no invalid output. This matches Chunker's own
test suite, whose tables these are generated from.

## Checks

| Area | Check | What it asserts |
|---|---|---|
| Bedrock blocks | `to_pivot` | every `block_states.json` state (x `waterlogged` true/false) maps, except Chunker's `UNSUPPORTED_BLOCKS` (education, elements, technical) |
| Bedrock blocks | `pivot_valid` | the pivot block has only states of its type with valid values |
| Bedrock blocks | `roundtrip` | Bedrock -> pivot -> Bedrock gives the same block after `removeLossyStates`; `LOSSY_BLOCKS` allowed |
| Java blocks (output) | `from_pivot_maps` | every state combination of every pivot type the version supports maps to Java |
| Java blocks (output) | `from_pivot_valid` | the result is a block state present in the version's `blocks.json` |
| Bedrock items | `to_pivot` / `roundtrip` | every real item of `item_names.json` maps to the pivot and back to the same id; `UNSUPPORTED_ITEMS`, `LOSSY_ITEMS` allowed |
| Java items (output) | `from_pivot_maps` / `from_pivot_valid` | every item pivot type maps to an id in `items.json` (the latest version must support all, like Chunker) |
| Cross | `to_java` | Bedrock state -> pivot -> Java for the closest Java release and the latest one: result must exist in the Java data |
| Tables | per pair | Bedrock biome/entity/enchantment/effect/potion/trim/banner/painting values map to the pivot, are unique, and the Java table at the closest/latest release writes the pivot name |

Chunker's data folder holds no vanilla list of biomes, entities, enchantments, effects or potions, so those are checked
internally and cross-edition only (no external ground truth).


## Per version

### Bedrock blocks (Bedrock -> pivot)

| Version | ids | `to_pivot` | `pivot_valid` | `roundtrip` |
|---|---|---|---|---|
| 1.12.0.28 | 460 | 6160 / 412 / 0 | 5748 / 0 / 0 | 5748 / 72 / 0 |
| 1.13.3.0 | 465 | 6090 / 414 / 0 | 5676 / 0 / 0 | 5676 / 72 / 0 |
| 1.14.1.4 | 469 | 6258 / 414 / 0 | 5844 / 0 / 0 | 5844 / 72 / 0 |
| 1.14.20.1 | 469 | 6258 / 414 / 0 | 5844 / 0 / 0 | 5844 / 72 / 0 |
| 1.14.32.1 | 469 | 6258 / 414 / 0 | 5844 / 0 / 0 | 5844 / 72 / 0 |
| 1.14.60.5 | 469 | 6258 / 414 / 0 | 5844 / 0 / 0 | 5844 / 72 / 0 |
| 1.16.1.02 | 553 | 13264 / 742 / 0 | 12522 / 0 / 0 | 12522 / 108 / 0 |
| 1.16.20.03 | 553 | 13216 / 742 / 0 | 12474 / 0 / 0 | 12474 / 108 / 0 |
| 1.16.40.02 | 553 | 13216 / 742 / 0 | 12474 / 0 / 0 | 12474 / 108 / 0 |
| 1.16.101.01 | 554 | 13222 / 744 / 0 | 12478 / 0 / 0 | 12478 / 108 / 0 |
| 1.16.201.02 | 554 | 13222 / 744 / 0 | 12478 / 0 / 0 | 12478 / 108 / 0 |
| 1.16.210.06 | 554 | 13150 / 744 / 0 | 12406 / 0 / 0 | 12406 / 108 / 0 |
| 1.16.221.01 | 554 | 13150 / 744 / 0 | 12406 / 0 / 0 | 12406 / 108 / 0 |
| 1.17.1.01 | 663 | 15364 / 744 / 0 | 14620 / 0 / 0 | 14620 / 146 / 0 |
| 1.17.11.01 | 697 | 15704 / 744 / 0 | 14960 / 0 / 0 | 14960 / 146 / 0 |
| 1.17.34.02 | 701 | 15888 / 744 / 0 | 15144 / 0 / 0 | 15144 / 194 / 0 |
| 1.17.41.01 | 704 | 15896 / 750 / 0 | 15146 / 0 / 0 | 15146 / 194 / 0 |
| 1.18.1.02 | 704 | 15896 / 750 / 0 | 15146 / 0 / 0 | 15146 / 194 / 0 |
| 1.18.12.01 | 708 | 15892 / 750 / 0 | 15142 / 0 / 0 | 15142 / 194 / 0 |
| 1.18.31.04 | 717 | 16456 / 746 / 0 | 15710 / 0 / 0 | 15710 / 194 / 0 |
| 1.19.2.02 | 734 | 16578 / 746 / 0 | 15832 / 0 / 0 | 15832 / 208 / 0 |
| 1.19.11.01 | 734 | 16578 / 746 / 0 | 15832 / 0 / 0 | 15832 / 208 / 0 |
| 1.19.22.01 | 734 | 16582 / 746 / 0 | 15836 / 0 / 0 | 15836 / 208 / 0 |
| 1.19.31.01 | 734 | 16582 / 746 / 0 | 15836 / 0 / 0 | 15836 / 208 / 0 |
| 1.19.41.01 | 734 | 16582 / 746 / 0 | 15836 / 0 / 0 | 15836 / 208 / 0 |
| 1.19.51.01 | 761 | 24936 / 746 / 0 | 24190 / 0 / 0 | 24190 / 208 / 0 |
| 1.19.63.01 | 763 | 25068 / 746 / 0 | 24322 / 0 / 0 | 24322 / 208 / 0 |
| 1.19.73.02 | 782 | 25110 / 746 / 0 | 24364 / 0 / 0 | 24364 / 208 / 0 |
| 1.19.83.01 | 813 | 26272 / 746 / 0 | 25526 / 0 / 0 | 25526 / 214 / 0 |
| 1.20.1.02 | 839 | 26282 / 746 / 0 | 25536 / 0 / 0 | 25536 / 172 / 0 |
| 1.20.15.01 | 869 | 26282 / 746 / 0 | 25536 / 0 / 0 | 25536 / 172 / 0 |
| 1.20.32.03 | 929 | 26258 / 746 / 0 | 25512 / 0 / 0 | 25512 / 172 / 0 |
| 1.20.41.02 | 929 | 26242 / 746 / 0 | 25496 / 0 / 0 | 25496 / 172 / 0 |
| 1.20.51.01 | 997 | 28254 / 746 / 0 | 27508 / 0 / 0 | 27508 / 172 / 0 |
| 1.20.61.01 | 1028 | 28266 / 746 / 0 | 27520 / 0 / 0 | 27520 / 172 / 0 |
| 1.20.72.01 | 1054 | 28298 / 746 / 0 | 27552 / 0 / 0 | 27552 / 172 / 0 |
| 1.20.80.05 | 1078 | 28280 / 746 / 0 | 27534 / 0 / 0 | 27534 / 172 / 0 |
| 1.21.0.03 | 1100 | 28320 / 746 / 0 | 27574 / 0 / 0 | 27574 / 168 / 0 |
| 1.21.20.0 | 1189 | 28302 / 746 / 0 | 27556 / 0 / 0 | 27556 / 164 / 0 |
| 1.21.30.0 | 1212 | 28300 / 746 / 0 | 27554 / 0 / 0 | 27554 / 164 / 0 |
| 1.21.40.0 | 1219 | 28392 / 746 / 0 | 27646 / 0 / 0 | 27646 / 152 / 0 |
| 1.21.50.0 | 1253 | 30324 / 778 / 0 | 29546 / 0 / 0 | 29546 / 152 / 0 |
| 1.21.60.0 | 1253 | 30336 / 778 / 0 | 29558 / 0 / 0 | 29558 / 152 / 0 |
| 1.21.70.0 | 1260 | 30474 / 778 / 0 | 29696 / 0 / 0 | 29696 / 152 / 0 |
| 1.21.80.0 | 1261 | 30506 / 778 / 0 | 29728 / 0 / 0 | 29728 / 152 / 0 |
| 1.21.90.0 | 1261 | 30506 / 778 / 0 | 29728 / 0 / 0 | 29728 / 152 / 0 |
| 1.21.93.0 | 1261 | 30506 / 778 / 0 | 29728 / 0 / 0 | 29728 / 152 / 0 |
| 1.21.100.0 | 1269 | 30570 / 778 / 0 | 29792 / 0 / 0 | 29792 / 152 / 0 |
| 1.21.110.0 | 1321 | 31690 / 778 / 0 | 30912 / 0 / 0 | 30912 / 152 / 0 |
| 1.21.120.0 | 1321 | 31690 / 778 / 0 | 30912 / 0 / 0 | 30912 / 152 / 0 |
| 1.21.130.0 | 1321 | 31690 / 778 / 0 | 30912 / 0 / 0 | 30912 / 152 / 0 |
| 1.26.0.0 | 1321 | 31690 / 778 / 0 | 30912 / 0 / 0 | 30912 / 152 / 0 |
| 1.26.10.0 | 1322 | 31692 / 778 / 0 | 30914 / 0 / 0 | 30914 / 152 / 0 |
| 1.26.20.0 | 1355 | 33798 / 778 / 0 | 33020 / 0 / 0 | 33020 / 152 / 0 |
| 1.26.30.0 | 1356 | 33826 / 778 / 0 | 33048 / 0 / 0 | 33048 / 152 / 0 |
| 1.26.40.0 | 1429 | 35400 / 778 / 0 | 34622 / 0 / 0 | 34622 / 152 / 0 |
| 1.26.50.0 | 1477 | 44182 / 1288 / 0 | 42894 / 0 / 0 | 42894 / 152 / 0 |
| 1.26.60.0 | 1480 | 47920 / 1288 / 0 | 46632 / 0 / 0 | 46632 / 152 / 0 |

### Java blocks (pivot -> Java)

| Version | ids | `from_pivot_maps` | `from_pivot_valid` |
|---|---|---|---|
| 1.13 | 593 | 17590 / 0 / 0 | 17590 / 0 / 0 |
| 1.13.1 | 598 | 17600 / 0 / 0 | 17600 / 0 / 0 |
| 1.13.2 | 598 | 17600 / 0 / 0 | 17600 / 0 / 0 |
| 1.14 | 676 | 23278 / 0 / 0 | 23278 / 0 / 0 |
| 1.14.1 | 676 | 23278 / 0 / 0 | 23278 / 0 / 0 |
| 1.14.2 | 676 | 23278 / 0 / 0 | 23278 / 0 / 0 |
| 1.14.3 | 676 | 23278 / 0 / 0 | 23278 / 0 / 0 |
| 1.14.4 | 676 | 23278 / 0 / 0 | 23278 / 0 / 0 |
| 1.15 | 680 | 23378 / 0 / 0 | 23378 / 0 / 0 |
| 1.15.1 | 680 | 23378 / 0 / 0 | 23378 / 0 / 0 |
| 1.15.2 | 680 | 23378 / 0 / 0 | 23378 / 0 / 0 |
| 1.16 | 763 | 25946 / 0 / 0 | 25946 / 0 / 0 |
| 1.16.1 | 763 | 25946 / 0 / 0 | 25946 / 0 / 0 |
| 1.16.2 | 763 | 25946 / 0 / 0 | 25946 / 0 / 0 |
| 1.16.3 | 763 | 25946 / 0 / 0 | 25946 / 0 / 0 |
| 1.16.4 | 763 | 25946 / 0 / 0 | 25946 / 0 / 0 |
| 1.16.5 | 763 | 25946 / 0 / 0 | 25946 / 0 / 0 |
| 1.17 | 898 | 29480 / 0 / 0 | 29480 / 0 / 0 |
| 1.17.1 | 898 | 29480 / 0 / 0 | 29480 / 0 / 0 |
| 1.18 | 898 | 29480 / 0 / 0 | 29480 / 0 / 0 |
| 1.18.1 | 898 | 29480 / 0 / 0 | 29480 / 0 / 0 |
| 1.18.2 | 898 | 29480 / 0 / 0 | 29480 / 0 / 0 |
| 1.19 | 933 | 30658 / 0 / 0 | 30658 / 0 / 0 |
| 1.19.1 | 933 | 30658 / 0 / 0 | 30658 / 0 / 0 |
| 1.19.2 | 933 | 30658 / 0 / 0 | 30658 / 0 / 0 |
| 1.19.3 | 972 | 32698 / 0 / 0 | 32698 / 0 / 0 |
| 1.19.4 | 998 | 33396 / 0 / 0 | 33396 / 0 / 0 |
| 1.20 | 1003 | 33818 / 0 / 0 | 33818 / 0 / 0 |
| 1.20.1 | 1003 | 33818 / 0 / 0 | 33818 / 0 / 0 |
| 1.20.2 | 1003 | 33818 / 0 / 0 | 33818 / 0 / 0 |
| 1.20.3 | 1058 | 36808 / 0 / 0 | 36808 / 0 / 0 |
| 1.20.4 | 1058 | 36808 / 0 / 0 | 36808 / 0 / 0 |
| 1.20.5 | 1060 | 36874 / 0 / 0 | 36874 / 0 / 0 |
| 1.20.6 | 1060 | 36874 / 0 / 0 | 36874 / 0 / 0 |
| 1.21 | 1060 | 36874 / 0 / 0 | 36874 / 0 / 0 |
| 1.21.1 | 1060 | 36874 / 0 / 0 | 36874 / 0 / 0 |
| 1.21.2 | 1084 | 37874 / 0 / 0 | 37874 / 0 / 0 |
| 1.21.3 | 1084 | 37874 / 0 / 0 | 37874 / 0 / 0 |
| 1.21.4 | 1095 | 38426 / 0 / 0 | 38426 / 0 / 0 |
| 1.21.5 | 1104 | 38510 / 0 / 0 | 38510 / 0 / 0 |
| 1.21.6 | 1105 | 38542 / 0 / 0 | 38542 / 0 / 0 |
| 1.21.7 | 1105 | 38542 / 0 / 0 | 38542 / 0 / 0 |
| 1.21.8 | 1105 | 38542 / 0 / 0 | 38542 / 0 / 0 |
| 1.21.9 | 1166 | 40272 / 0 / 0 | 40272 / 0 / 0 |
| 1.21.10 | 1166 | 40272 / 0 / 0 | 40272 / 0 / 0 |
| 1.21.11 | 1166 | 40272 / 0 / 0 | 40272 / 0 / 0 |
| 26.1 | 1168 | 40276 / 0 / 0 | 40276 / 0 / 0 |
| 26.1.1 | 1168 | 40276 / 0 / 0 | 40276 / 0 / 0 |
| 26.1.2 | 1168 | 40276 / 0 / 0 | 40276 / 0 / 0 |
| 26.2 | 1196 | 42782 / 0 / 0 | 42782 / 0 / 0 |
| 26.3 | 1286 | 46394 / 0 / 0 | 46394 / 0 / 0 |

### Bedrock items (Bedrock -> pivot)

| Version | ids | `to_pivot` | `roundtrip` |
|---|---|---|---|
| 1.12.0.28 | 229 | 229 / 8 / 0 | 221 / 4 / 0 |
| 1.13.3.0 | 230 | 230 / 8 / 0 | 222 / 4 / 0 |
| 1.14.1.4 | 232 | 232 / 8 / 0 | 224 / 4 / 0 |
| 1.14.20.1 | 232 | 232 / 8 / 0 | 224 / 4 / 0 |
| 1.14.32.1 | 232 | 232 / 8 / 0 | 224 / 4 / 0 |
| 1.14.60.5 | 232 | 232 / 8 / 0 | 224 / 4 / 0 |
| 1.16.1.02 | 253 | 253 / 8 / 0 | 245 / 4 / 0 |
| 1.16.20.03 | 253 | 253 / 8 / 0 | 245 / 4 / 0 |
| 1.16.40.02 | 253 | 253 / 8 / 0 | 245 / 4 / 0 |
| 1.16.101.01 | 359 | 359 / 10 / 0 | 349 / 4 / 0 |
| 1.16.201.02 | 359 | 359 / 10 / 0 | 349 / 4 / 0 |
| 1.16.210.06 | 359 | 359 / 10 / 0 | 349 / 4 / 0 |
| 1.16.221.01 | 359 | 359 / 10 / 0 | 349 / 4 / 0 |
| 1.17.1.01 | 374 | 374 / 10 / 0 | 364 / 4 / 0 |
| 1.17.11.01 | 374 | 374 / 10 / 0 | 364 / 4 / 0 |
| 1.17.34.02 | 374 | 374 / 10 / 0 | 364 / 4 / 0 |
| 1.17.41.01 | 374 | 374 / 10 / 0 | 364 / 4 / 0 |
| 1.18.1.02 | 375 | 375 / 10 / 0 | 365 / 4 / 0 |
| 1.18.12.01 | 381 | 381 / 11 / 0 | 370 / 4 / 0 |
| 1.18.31.04 | 388 | 388 / 11 / 0 | 377 / 5 / 0 |
| 1.19.2.02 | 396 | 396 / 10 / 0 | 386 / 5 / 0 |
| 1.19.11.01 | 397 | 397 / 10 / 0 | 387 / 5 / 0 |
| 1.19.22.01 | 397 | 397 / 10 / 0 | 387 / 5 / 0 |
| 1.19.31.01 | 397 | 397 / 10 / 0 | 387 / 5 / 0 |
| 1.19.41.01 | 397 | 397 / 10 / 0 | 387 / 5 / 0 |
| 1.19.51.01 | 412 | 412 / 10 / 0 | 402 / 5 / 0 |
| 1.19.63.01 | 417 | 417 / 10 / 0 | 407 / 5 / 0 |
| 1.19.73.02 | 424 | 424 / 11 / 0 | 413 / 5 / 0 |
| 1.19.83.01 | 465 | 465 / 14 / 0 | 451 / 5 / 0 |
| 1.20.1.02 | 469 | 469 / 16 / 0 | 453 / 5 / 0 |
| 1.20.15.01 | 471 | 471 / 18 / 0 | 453 / 5 / 0 |
| 1.20.32.03 | 475 | 475 / 22 / 0 | 453 / 5 / 0 |
| 1.20.41.02 | 475 | 475 / 22 / 0 | 453 / 5 / 0 |
| 1.20.51.01 | 484 | 484 / 23 / 0 | 461 / 5 / 0 |
| 1.20.61.01 | 491 | 491 / 25 / 0 | 466 / 5 / 0 |
| 1.20.72.01 | 497 | 497 / 29 / 0 | 468 / 5 / 0 |
| 1.20.80.05 | 510 | 510 / 33 / 0 | 477 / 5 / 0 |
| 1.21.0.03 | 519 | 519 / 37 / 0 | 482 / 5 / 0 |
| 1.21.20.0 | 529 | 529 / 47 / 0 | 482 / 5 / 0 |
| 1.21.30.0 | 532 | 532 / 50 / 0 | 482 / 5 / 0 |
| 1.21.40.0 | 549 | 549 / 50 / 0 | 499 / 5 / 0 |
| 1.21.50.0 | 557 | 557 / 51 / 0 | 506 / 5 / 0 |
| 1.21.60.0 | 557 | 557 / 51 / 0 | 506 / 5 / 0 |
| 1.21.70.0 | 559 | 559 / 51 / 0 | 508 / 5 / 0 |
| 1.21.80.0 | 576 | 576 / 51 / 0 | 525 / 5 / 0 |
| 1.21.90.0 | 577 | 577 / 51 / 0 | 526 / 5 / 0 |
| 1.21.93.0 | 578 | 578 / 51 / 0 | 527 / 5 / 0 |
| 1.21.100.0 | 589 | 589 / 51 / 0 | 538 / 5 / 0 |
| 1.21.110.0 | 589 | 589 / 51 / 0 | 538 / 5 / 0 |
| 1.21.120.0 | 589 | 589 / 51 / 0 | 538 / 5 / 0 |
| 1.21.130.0 | 606 | 606 / 51 / 0 | 555 / 5 / 0 |
| 1.26.0.0 | 606 | 606 / 51 / 0 | 555 / 5 / 0 |
| 1.26.10.0 | 606 | 606 / 51 / 0 | 555 / 5 / 0 |
| 1.26.20.0 | 608 | 608 / 51 / 0 | 557 / 5 / 0 |
| 1.26.30.0 | 609 | 609 / 51 / 0 | 558 / 5 / 0 |
| 1.26.40.0 | 633 | 633 / 51 / 0 | 582 / 5 / 0 |
| 1.26.50.0 | 635 | 635 / 53 / 0 | 582 / 5 / 0 |
| 1.26.60.0 | 636 | 636 / 53 / 0 | 583 / 5 / 0 |

### Java items (pivot -> Java)

| Version | ids | `from_pivot_maps` | `from_pivot_valid` |
|---|---|---|---|
| 1.13 | 785 | 242 / 0 / 0 | 242 / 0 / 0 |
| 1.13.1 | 790 | 242 / 0 / 0 | 242 / 0 / 0 |
| 1.13.2 | 790 | 242 / 0 / 0 | 242 / 0 / 0 |
| 1.14 | 877 | 260 / 0 / 0 | 260 / 0 / 0 |
| 1.14.1 | 877 | 260 / 0 / 0 | 260 / 0 / 0 |
| 1.14.2 | 877 | 260 / 0 / 0 | 260 / 0 / 0 |
| 1.14.3 | 877 | 260 / 0 / 0 | 260 / 0 / 0 |
| 1.14.4 | 877 | 260 / 0 / 0 | 260 / 0 / 0 |
| 1.15 | 884 | 262 / 0 / 0 | 262 / 0 / 0 |
| 1.15.1 | 884 | 262 / 0 / 0 | 262 / 0 / 0 |
| 1.15.2 | 884 | 262 / 0 / 0 | 262 / 0 / 0 |
| 1.16 | 975 | 278 / 0 / 0 | 278 / 0 / 0 |
| 1.16.1 | 975 | 278 / 0 / 0 | 278 / 0 / 0 |
| 1.16.2 | 976 | 278 / 0 / 0 | 278 / 0 / 0 |
| 1.16.3 | 976 | 278 / 0 / 0 | 278 / 0 / 0 |
| 1.16.4 | 976 | 278 / 0 / 0 | 278 / 0 / 0 |
| 1.16.5 | 976 | 278 / 0 / 0 | 278 / 0 / 0 |
| 1.17 | 1100 | 290 / 0 / 0 | 290 / 0 / 0 |
| 1.17.1 | 1100 | 290 / 0 / 0 | 290 / 0 / 0 |
| 1.18 | 1101 | 291 / 0 / 0 | 291 / 0 / 0 |
| 1.18.1 | 1101 | 291 / 0 / 0 | 291 / 0 / 0 |
| 1.18.2 | 1101 | 291 / 0 / 0 | 291 / 0 / 0 |
| 1.19 | 1152 | 306 / 0 / 0 | 306 / 0 / 0 |
| 1.19.1 | 1152 | 306 / 0 / 0 | 306 / 0 / 0 |
| 1.19.2 | 1152 | 306 / 0 / 0 | 306 / 0 / 0 |
| 1.19.3 | 1186 | 319 / 0 / 0 | 319 / 0 / 0 |
| 1.19.4 | 1228 | 341 / 0 / 0 | 341 / 0 / 0 |
| 1.20 | 1255 | 364 / 0 / 0 | 364 / 0 / 0 |
| 1.20.1 | 1255 | 364 / 0 / 0 | 364 / 0 / 0 |
| 1.20.2 | 1255 | 364 / 0 / 0 | 364 / 0 / 0 |
| 1.20.3 | 1312 | 365 / 0 / 0 | 365 / 0 / 0 |
| 1.20.4 | 1312 | 365 / 0 / 0 | 365 / 0 / 0 |
| 1.20.5 | 1330 | 379 / 0 / 0 | 379 / 0 / 0 |
| 1.20.6 | 1330 | 379 / 0 / 0 | 379 / 0 / 0 |
| 1.21 | 1333 | 382 / 0 / 0 | 382 / 0 / 0 |
| 1.21.1 | 1333 | 382 / 0 / 0 | 382 / 0 / 0 |
| 1.21.2 | 1375 | 404 / 0 / 0 | 404 / 0 / 0 |
| 1.21.3 | 1375 | 404 / 0 / 0 | 404 / 0 / 0 |
| 1.21.4 | 1385 | 405 / 0 / 0 | 405 / 0 / 0 |
| 1.21.5 | 1396 | 407 / 0 / 0 | 407 / 0 / 0 |
| 1.21.6 | 1415 | 424 / 0 / 0 | 424 / 0 / 0 |
| 1.21.7 | 1416 | 425 / 0 / 0 | 425 / 0 / 0 |
| 1.21.8 | 1416 | 425 / 0 / 0 | 425 / 0 / 0 |
| 1.21.9 | 1488 | 436 / 0 / 0 | 436 / 0 / 0 |
| 1.21.10 | 1488 | 436 / 0 / 0 | 436 / 0 / 0 |
| 1.21.11 | 1505 | 449 / 0 / 0 | 449 / 0 / 0 |
| 26.1 | 1506 | 449 / 0 / 0 | 449 / 0 / 0 |
| 26.1.1 | 1506 | 449 / 0 / 0 | 449 / 0 / 0 |
| 26.1.2 | 1506 | 449 / 0 / 0 | 449 / 0 / 0 |
| 26.2 | 1537 | 451 / 0 / 0 | 451 / 0 / 0 |
| 26.3 | 1658 | 471 / 0 / 0 | 471 / 0 / 0 |

### Cross-edition (Bedrock version -> Java version), 116 pairs

Total `to_java`: 2561116 checked / 35106 allowed / 0 failed. Per-pair numbers are in the test output (`@@V cross` lines).

### Id tables, 116 pairs, summed

| Table pair | checked | allowed | failed |
|---|---|---|---|
| `biome_id>biome_name` | 9568 | 3140 | 0 |
| `biome_id>biome_id` | 9568 | 380 | 0 |
| `entity_type` | 13664 | 544 | 0 |
| `entity_id>entity_type` | 13664 | 544 | 0 |
| `enchantment_id>enchantment` | 4498 | 6 | 0 |
| `effect_id>effect` | 3830 | 123 | 0 |
| `potion_id>potion` | 5260 | 348 | 0 |
| `trim_material` | 1194 | 0 | 0 |
| `trim_pattern` | 1048 | 5 | 0 |
| `banner_pattern` | 4840 | 0 | 0 |
| `painting` | 4344 | 80 | 0 |

## Allow-list and gap inventory

Chunker's own allow-lists (counts over all versions): Bedrock blocks `to_pivot` allowed 42,824 states (education/element/technical
blocks), `roundtrip` allowed 8,936 (lossy blocks and states); Bedrock items 1,522 unsupported and 271 lossy (`dye`, `banner_pattern`,
`boat`, `chest_boat`, `skull`). `removeLossyStates` normalisations (rails, hoppers, mushrooms, quartz/purpur axis, anvils, ...) are applied to both sides.

## Real mapping gaps (Bedrock -> Java), by priority

None of these is a failure of the tables against Chunker (Chunker has the same holes); they are content that has no Java equivalent
and will be dropped or replaced when converting. Listed so the pipeline can decide a fallback.

1. **Legacy biomes have no Java 1.18+ name.** 27 pivot biomes that Bedrock still stores (`DESERT_HILLS`, `JUNGLE_HILLS`,
   `TAIGA_HILLS`, `WOODED_HILLS`, `SNOWY_MOUNTAINS`, `SNOWY_TAIGA_HILLS`, `SNOWY_TAIGA_MOUNTAINS`, `BIRCH_FOREST_HILLS`, `TALL_BIRCH_HILLS`,
   `DARK_FOREST_HILLS`, `GIANT_TREE_TAIGA_HILLS`, `GIANT_SPRUCE_TAIGA_HILLS`, `SWAMP_HILLS`, `MODIFIED_JUNGLE`, `MODIFIED_JUNGLE_EDGE`,
   `MODIFIED_GRAVELLY_MOUNTAINS`, `MODIFIED_BADLANDS_PLATEAU`, `MODIFIED_WOODED_BADLANDS_PLATEAU`, `SHATTERED_SAVANNA_PLATEAU`,
   `BADLANDS_PLATEAU`, `MOUNTAIN_EDGE`, `MUSHROOM_FIELD_SHORE`, `DEEP_WARM_OCEAN`, `DESERT_LAKES`, `BAMBOO_JUNGLE_HILLS`, `TAIGA_MOUNTAINS`,
   `LEGACY_FROZEN_OCEAN`) are absent from the `biome_name` table, so on Java 1.18+ they need a substitute biome (they exist only in the
   numeric Java `biome_id` table, pre-1.18). A worlds converted from old Bedrock saves will hit this.
2. **Bedrock-only blocks with no Java writer**: `minecraft:frame` and `minecraft:glow_frame` (`ITEM_FRAME_BEDROCK`, they become block entities in Java),
   and the newest Bedrock blocks `minecraft:icicle`, `minecraft:ice_crystal`, `minecraft:disc_press` (`ICICLE`, `ICE_CRYSTAL`, `DISC_PRESS`) which
   no Java version in the data has.
3. **Bedrock-only entities, effects, biomes**: `ELDER_GUARDIAN_GHOST`, `MOVING_BLOCK`, `SHIELD`, `WITHER_SKULL_DANGEROUS`, `FROSTBITE` (entity ids),
   `FATAL_POISON` (effect id),. The biomes `CHERRY_GROVE`, `DEEP_DARK`, `MANGROVE_SWAMP`, `PALE_GARDEN`, `DAPPLED_FOREST`, `SULFUR_CAVES`
   have no numeric Java id (pre-1.18 `biome_id` table), which is correct: they only exist as 1.18+ names.
4. **Newer content than the target Java release**: 217 pivot block types (`newer:` in the cross check) are written by the latest Java tables but not by
   the closest-release one (e.g. 1.21.5 blocks when converting a 1.21.2-era world to Java 1.21.1). Expected; the converter should fall back to air or
   a nearby block rather than fail.
5. Education/technical blocks (`UNSUPPORTED_BLOCKS`) are dropped by design, as in Chunker.

Not checked (no ground truth in Chunker's data): block entity ids, enchantment/effect lists per version, map colours, legacy numeric item ids.
