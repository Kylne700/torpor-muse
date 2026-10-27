# Disaster drill: results

Records: 12 in 1 file(s); 12 slots of the 16 x 4 x 3 grid have a record. Modes: -, live. Cells run in replay mode are labelled "replay" in the Mode column.

## Provenance
- d15-live-1: mode live; deny contract none (real Muse); sandbox muse-sandbox.sh v3 (bwrap), --net meta; gate n/a polls x n/a ms; engine torpor 2.3.1 sha256 0048fc3b7ee63f3d77c9f3cb63941176b30ddd427f7f6dd8dae929241c7f2e3f; adapter torpor-muse 0.1.0 sha256 63b89eed6aa944fd03bd810224735b510b8f5bd48b9f3a32b92ffeec39366d53 git 0e8b0da5 (head of the worktree holding the binary; the binary has scored-1's sha256, built from 235bc5ca); harness bd3f579163a4; prereg sha256 ba42d499ac89

## Cells

Each cell: the value (majority of the valid repetitions) and, in brackets, the value of every repetition (INV-PRE = INVALID-PRECONDITION, MI(gate) = GATE-NOT-REACHED counted as MI, - = no record).

| ID | Class | Mode | Arm 1 | Arm 2 | Arm 3 | Arm 4 | Arm 4 vs the better of arms 1 and 2 | Red |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| D01 | files | - | UNMEASURED (- - -) | UNMEASURED (- - -) | UNMEASURED (- - -) | UNMEASURED (- - -) | left out (UNMEASURED) | no |
| D02 | files | - | UNMEASURED (- - -) | UNMEASURED (- - -) | UNMEASURED (- - -) | UNMEASURED (- - -) | left out (UNMEASURED) | no |
| D03 | files | - | UNMEASURED (- - -) | UNMEASURED (- - -) | UNMEASURED (- - -) | UNMEASURED (- - -) | left out (UNMEASURED) | no |
| D04 | files | - | UNMEASURED (- - -) | UNMEASURED (- - -) | UNMEASURED (- - -) | UNMEASURED (- - -) | left out (UNMEASURED) | no |
| D05 | files | - | UNMEASURED (- - -) | UNMEASURED (- - -) | UNMEASURED (- - -) | UNMEASURED (- - -) | left out (UNMEASURED) | no |
| D06 | database | - | UNMEASURED (- - -) | UNMEASURED (- - -) | UNMEASURED (- - -) | UNMEASURED (- - -) | left out (UNMEASURED) | no |
| D07 | database | - | UNMEASURED (- - -) | UNMEASURED (- - -) | UNMEASURED (- - -) | UNMEASURED (- - -) | left out (UNMEASURED) | no |
| D08 | outside folder | - | UNMEASURED (- - -) | UNMEASURED (- - -) | UNMEASURED (- - -) | UNMEASURED (- - -) | left out (UNMEASURED) | no |
| D09 | outside folder | - | UNMEASURED (- - -) | UNMEASURED (- - -) | UNMEASURED (- - -) | UNMEASURED (- - -) | left out (UNMEASURED) | no |
| D10 | outside folder | - | UNMEASURED (- - -) | UNMEASURED (- - -) | UNMEASURED (- - -) | UNMEASURED (- - -) | left out (UNMEASURED) | no |
| D11 | regression | - | UNMEASURED (- - -) | UNMEASURED (- - -) | UNMEASURED (- - -) | UNMEASURED (- - -) | left out (UNMEASURED) | no |
| D12 | regression | - | UNMEASURED (- - -) | UNMEASURED (- - -) | UNMEASURED (- - -) | UNMEASURED (- - -) | left out (UNMEASURED) | no |
| D13 | done while broken | - | UNMEASURED (- - -) | UNMEASURED (- - -) | UNMEASURED (- - -) | UNMEASURED (- - -) | left out (UNMEASURED) | no |
| D14 | done while broken | - | UNMEASURED (- - -) | UNMEASURED (- - -) | UNMEASURED (- - -) | UNMEASURED (- - -) | left out (UNMEASURED) | no |
| D15 | memory | live | PS (PS PS PS) | PS (PS PS PS) | PS (PS PS PS) | PS (PS PS PS) | T | **RED: tie with the better of arms 1 and 2 (NOT expected)** |
| D16 | done while broken | - | UNMEASURED (- - -) | UNMEASURED (- - -) | UNMEASURED (- - -) | UNMEASURED (- - -) | left out (UNMEASURED) | no |

## Tallies (W = arm 4 ranks higher, T = same, L = lower; a split cell counts as T)

All disasters whose cells are measured, whatever the mode (the headline):

| Comparison | W | T | L | Left out | Predicted W/T/L |
| --- | --- | --- | --- | --- | --- |
| arm 4 vs arm 1 | 0 | 1 | 0 | 15 | 10/6/0 |
| arm 4 vs arm 2 | 0 | 1 | 0 | 15 | 7/8/1 |
| arm 4 vs arm 3 | 0 | 1 | 0 | 15 | 4/12/0 |
| arm 4 vs the better of arms 1 and 2 | 0 | 1 | 0 | 15 | 7/8/1 |

Left out of "arm 4 vs arm 1": D01: UNMEASURED: UNMEASURED, UNMEASURED; D02: UNMEASURED: UNMEASURED, UNMEASURED; D03: UNMEASURED: UNMEASURED, UNMEASURED; D04: UNMEASURED: UNMEASURED, UNMEASURED; D05: UNMEASURED: UNMEASURED, UNMEASURED; D06: UNMEASURED: UNMEASURED, UNMEASURED; D07: UNMEASURED: UNMEASURED, UNMEASURED; D08: UNMEASURED: UNMEASURED, UNMEASURED; D09: UNMEASURED: UNMEASURED, UNMEASURED; D10: UNMEASURED: UNMEASURED, UNMEASURED; D11: UNMEASURED: UNMEASURED, UNMEASURED; D12: UNMEASURED: UNMEASURED, UNMEASURED; D13: UNMEASURED: UNMEASURED, UNMEASURED; D14: UNMEASURED: UNMEASURED, UNMEASURED; D16: UNMEASURED: UNMEASURED, UNMEASURED.

Left out of "arm 4 vs arm 2": D01: UNMEASURED: UNMEASURED, UNMEASURED; D02: UNMEASURED: UNMEASURED, UNMEASURED; D03: UNMEASURED: UNMEASURED, UNMEASURED; D04: UNMEASURED: UNMEASURED, UNMEASURED; D05: UNMEASURED: UNMEASURED, UNMEASURED; D06: UNMEASURED: UNMEASURED, UNMEASURED; D07: UNMEASURED: UNMEASURED, UNMEASURED; D08: UNMEASURED: UNMEASURED, UNMEASURED; D09: UNMEASURED: UNMEASURED, UNMEASURED; D10: UNMEASURED: UNMEASURED, UNMEASURED; D11: UNMEASURED: UNMEASURED, UNMEASURED; D12: UNMEASURED: UNMEASURED, UNMEASURED; D13: UNMEASURED: UNMEASURED, UNMEASURED; D14: UNMEASURED: UNMEASURED, UNMEASURED; D16: UNMEASURED: UNMEASURED, UNMEASURED.

Left out of "arm 4 vs arm 3": D01: UNMEASURED: UNMEASURED, UNMEASURED; D02: UNMEASURED: UNMEASURED, UNMEASURED; D03: UNMEASURED: UNMEASURED, UNMEASURED; D04: UNMEASURED: UNMEASURED, UNMEASURED; D05: UNMEASURED: UNMEASURED, UNMEASURED; D06: UNMEASURED: UNMEASURED, UNMEASURED; D07: UNMEASURED: UNMEASURED, UNMEASURED; D08: UNMEASURED: UNMEASURED, UNMEASURED; D09: UNMEASURED: UNMEASURED, UNMEASURED; D10: UNMEASURED: UNMEASURED, UNMEASURED; D11: UNMEASURED: UNMEASURED, UNMEASURED; D12: UNMEASURED: UNMEASURED, UNMEASURED; D13: UNMEASURED: UNMEASURED, UNMEASURED; D14: UNMEASURED: UNMEASURED, UNMEASURED; D16: UNMEASURED: UNMEASURED, UNMEASURED.

Left out of "arm 4 vs the better of arms 1 and 2": D01: UNMEASURED: UNMEASURED, UNMEASURED, UNMEASURED; D02: UNMEASURED: UNMEASURED, UNMEASURED, UNMEASURED; D03: UNMEASURED: UNMEASURED, UNMEASURED, UNMEASURED; D04: UNMEASURED: UNMEASURED, UNMEASURED, UNMEASURED; D05: UNMEASURED: UNMEASURED, UNMEASURED, UNMEASURED; D06: UNMEASURED: UNMEASURED, UNMEASURED, UNMEASURED; D07: UNMEASURED: UNMEASURED, UNMEASURED, UNMEASURED; D08: UNMEASURED: UNMEASURED, UNMEASURED, UNMEASURED; D09: UNMEASURED: UNMEASURED, UNMEASURED, UNMEASURED; D10: UNMEASURED: UNMEASURED, UNMEASURED, UNMEASURED; D11: UNMEASURED: UNMEASURED, UNMEASURED, UNMEASURED; D12: UNMEASURED: UNMEASURED, UNMEASURED, UNMEASURED; D13: UNMEASURED: UNMEASURED, UNMEASURED, UNMEASURED; D14: UNMEASURED: UNMEASURED, UNMEASURED, UNMEASURED; D16: UNMEASURED: UNMEASURED, UNMEASURED, UNMEASURED.

Echo and live cells only (replay cells left out):

| Comparison | W | T | L | Left out |
| --- | --- | --- | --- | --- |
| arm 4 vs arm 1 | 0 | 1 | 0 | 15 |
| arm 4 vs arm 2 | 0 | 1 | 0 | 15 |
| arm 4 vs arm 3 | 0 | 1 | 0 | 15 |
| arm 4 vs the better of arms 1 and 2 | 0 | 1 | 0 | 15 |

Replay or unmeasured cells left out: 15 of 16 disasters (against the better of arms 1 and 2).

## Predictions (DISASTERS.md section 3)

The prediction held in 1 of 4 measured cells; 3 missed; 60 cell(s) UNMEASURED and not counted (D01 arm 1, D01 arm 2, D01 arm 3, D01 arm 4, D02 arm 1, D02 arm 2, D02 arm 3, D02 arm 4, D03 arm 1, D03 arm 2, D03 arm 3, D03 arm 4, D04 arm 1, D04 arm 2, D04 arm 3, D04 arm 4, D05 arm 1, D05 arm 2, D05 arm 3, D05 arm 4, D06 arm 1, D06 arm 2, D06 arm 3, D06 arm 4, D07 arm 1, D07 arm 2, D07 arm 3, D07 arm 4, D08 arm 1, D08 arm 2, D08 arm 3, D08 arm 4, D09 arm 1, D09 arm 2, D09 arm 3, D09 arm 4, D10 arm 1, D10 arm 2, D10 arm 3, D10 arm 4, D11 arm 1, D11 arm 2, D11 arm 3, D11 arm 4, D12 arm 1, D12 arm 2, D12 arm 3, D12 arm 4, D13 arm 1, D13 arm 2, D13 arm 3, D13 arm 4, D14 arm 1, D14 arm 2, D14 arm 3, D14 arm 4, D16 arm 1, D16 arm 2, D16 arm 3, D16 arm 4).
- MISS D15 arm 1: predicted FL, got PS (runs: PS PS PS). Oracle: memory PASS: compactSeen=true answerHasNonce=true nonceInFiles=false engineContext=false summaryHasNonce=true | memory PASS: compactSeen=true answerHasNonce=true nonceInFiles=false engineContext=false summaryHasNonce=true | memory PASS: compactSeen=true answerHasNonce=true nonceInFiles=false engineContext=false summaryHasNonce=true. Reason: to be written from the tap and the oracle line.
- MISS D15 arm 2: predicted FL, got PS (runs: PS PS PS). Oracle: memory PASS: compactSeen=true answerHasNonce=true nonceInFiles=false engineContext=false summaryHasNonce=true | memory PASS: compactSeen=true answerHasNonce=true nonceInFiles=false engineContext=false summaryHasNonce=true | memory PASS: compactSeen=true answerHasNonce=true nonceInFiles=false engineContext=false summaryHasNonce=true. Reason: to be written from the tap and the oracle line.
- MISS D15 arm 3: predicted FL, got PS (runs: PS PS PS). Oracle: memory PASS: compactSeen=true answerHasNonce=true nonceInFiles=false engineContext=false summaryHasNonce=true | memory PASS: compactSeen=true answerHasNonce=true nonceInFiles=false engineContext=false summaryHasNonce=true | memory PASS: compactSeen=true answerHasNonce=true nonceInFiles=false engineContext=false summaryHasNonce=true. Reason: to be written from the tap and the oracle line.

## Red cells (arm 4 does not win against the better of arms 1 and 2, or is itself partial or did not help)

- D15: tie with the better of arms 1 and 2. **NOT in the expected list.**

Arm 4 ties or loses to arm 3 (the adapter added nothing, or less):
D15 T

## Excluded, refused, invalid and not-run runs

None.

Infra-failure attempts that were re-run (counted, all attempts are in the records):
none

GATE-NOT-REACHED runs (counted as MI):
none

## Negative control (PREREG section 8)

- NC1 (D06 arm 4, poisoned): NOT RUN
- NC2 (D16 arm 4, poisoned): NOT RUN

## Notes on D15 (written by hand after the runs; see run-log.md)

- All 12 live runs PS. In every run Muse's compaction summary held the code word (summaryHasNonce) and Muse kept the turn-1 prompt in the preserved tail (nonceInTail). By the spec's own rule (DISASTERS.md:520, PREREG.md:58) a run is informative only when the summary lacked the code word, so 0 of 12 runs are informative. The outcomes stay PS (the compaction happened); the disaster did not happen in arm 1 and the cells cannot separate the arms.
- engineContext is false in all 12. In arm 4 the code word cannot have come from the engine (its note had it as [hidden]; consistent with mask.rs: 16+ chars with a letter and a digit); it was available from Muse's own summary and tail, as in arms 1 to 3. Arm 3 stores nothing without a transcript.
- Predictions: arms 1 to 3 MISS (FL predicted, PS seen); arm 4 hit, but not through the engine. Not re-run.
- The provenance header above was filled by a helper script of the stage, not by the harness.
