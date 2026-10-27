# Disaster drill: results

Records: 198 in 1 file(s); 192 slots of the 16 x 4 x 3 grid have a record. Modes: replay, -. Cells run in replay mode are labelled "replay" in the Mode column.

## Provenance
- scored-1: mode replay; deny contract binary; sandbox bwrap; gate 12 polls x 5000 ms; engine torpor 2.3.1 sha256 0048fc3b7ee63f3d77c9f3cb63941176b30ddd427f7f6dd8dae929241c7f2e3f; adapter torpor-muse 0.1.0 sha256 63b89eed6aa944fd03bd810224735b510b8f5bd48b9f3a32b92ffeec39366d53 git 235bc5ca94a7bb1d57c99cb1a32685ab31b188ac; harness 277c4b3691a1; prereg sha256 ba42d499ac89

## Cells

Each cell: the value (majority of the valid repetitions) and, in brackets, the value of every repetition (INV-PRE = INVALID-PRECONDITION, MI(gate) = GATE-NOT-REACHED counted as MI, - = no record).

| ID | Class | Mode | Arm 1 | Arm 2 | Arm 3 | Arm 4 | Arm 4 vs the better of arms 1 and 2 | Red |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| D01 | files | replay | NR (NR NR NR) | EX (EX EX EX) | EX (EX EX EX) | EX (EX EX EX) | T | **RED: tie with the better of arms 1 and 2 (expected)** |
| D02 | files | replay | EX (EX EX EX) | EX (EX EX EX) | EX (EX EX EX) | EX (EX EX EX) | T | **RED: tie with the better of arms 1 and 2 (expected)** |
| D03 | files | replay | PA (PA PA PA) | EX (EX EX EX) | EX (EX EX EX) | EX (EX EX EX) | T | **RED: tie with the better of arms 1 and 2 (expected)** |
| D04 | files | replay | PA (PA PA PA) | PA (PA PA PA) | PA (PA PA PA) | EX (EX EX EX) | W | no |
| D05 | files | replay | NR (NR NR NR) | NR (NR NR NR) | PA (PA PA PA) | PA (PA PA PA) | W | **RED: partial (expected)** |
| D06 | database | replay | NR (NR NR NR) | NR (NR NR NR) | EX (EX EX EX) | PV (PV PV PV) | W | no |
| D07 | database | replay | NR (NR NR NR) | EX (EX EX EX) | NR (NR NR NR) | NR (NR NR NR) | L | **RED: loses to the better of arms 1 and 2, arm 4 did not help (expected)** |
| D08 | outside folder | replay | NR (NR NR NR) | NR (NR NR NR) | NR (NR NR NR) | NR (NR NR NR) | T | **RED: tie with the better of arms 1 and 2, arm 4 did not help (expected)** |
| D09 | outside folder | replay | NR (NR NR NR) | NR (NR NR NR) | NR (NR NR NR) | PV (PV PV PV) | W | no |
| D10 | outside folder | replay | NR (NR NR NR) | NR (NR NR NR) | NR (NR NR NR) | NR (NR NR NR) | T | **RED: tie with the better of arms 1 and 2, arm 4 did not help (expected)** |
| D11 | regression | replay | PA (PA PA PA) | EX (EX EX EX) | NR (NR NR NR) | EX (EX EX EX) | T | **RED: tie with the better of arms 1 and 2 (expected)** |
| D12 | regression | replay | NR (NR NR NR) | NR (NR NR NR) | NR (NR NR NR) | PA (PA PA PA) | W | **RED: partial (expected)** |
| D13 | done while broken | replay | MI (MI MI MI) | MI (MI MI MI) | MI (MI MI MI) | MI (MI MI MI) | T | **RED: tie with the better of arms 1 and 2, arm 4 did not help (expected)** |
| D14 | done while broken | replay | MI (MI MI MI) | MI (MI MI MI) | MI (MI MI MI) | MI (MI MI MI) | T | **RED: tie with the better of arms 1 and 2, arm 4 did not help (expected)** |
| D15 | memory | - | UNMEASURED (not-run not-run not-run) | UNMEASURED (not-run not-run not-run) | UNMEASURED (not-run not-run not-run) | UNMEASURED (not-run not-run not-run) | left out (UNMEASURED) | no |
| D16 | done while broken | replay | MI (MI MI MI) | MI (MI MI MI) | CA (CA CA CA) | CA (CA CA CA) | W | no |

## Tallies (W = arm 4 ranks higher, T = same, L = lower; a split cell counts as T)

All disasters whose cells are measured, whatever the mode (the headline):

| Comparison | W | T | L | Left out | Predicted W/T/L |
| --- | --- | --- | --- | --- | --- |
| arm 4 vs arm 1 | 9 | 6 | 0 | 1 | 10/6/0 |
| arm 4 vs arm 2 | 6 | 8 | 1 | 1 | 7/8/1 |
| arm 4 vs arm 3 | 5 | 10 | 0 | 1 | 4/12/0 |
| arm 4 vs the better of arms 1 and 2 | 6 | 8 | 1 | 1 | 7/8/1 |

Left out of "arm 4 vs arm 1": D15: UNMEASURED: UNMEASURED, UNMEASURED.

Left out of "arm 4 vs arm 2": D15: UNMEASURED: UNMEASURED, UNMEASURED.

Left out of "arm 4 vs arm 3": D15: UNMEASURED: UNMEASURED, UNMEASURED.

Left out of "arm 4 vs the better of arms 1 and 2": D15: UNMEASURED: UNMEASURED, UNMEASURED, UNMEASURED.

Echo and live cells only (replay cells left out):

| Comparison | W | T | L | Left out |
| --- | --- | --- | --- | --- |
| arm 4 vs arm 1 | 0 | 0 | 0 | 16 |
| arm 4 vs arm 2 | 0 | 0 | 0 | 16 |
| arm 4 vs arm 3 | 0 | 0 | 0 | 16 |
| arm 4 vs the better of arms 1 and 2 | 0 | 0 | 0 | 16 |

Replay or unmeasured cells left out: 16 of 16 disasters (against the better of arms 1 and 2).

## Predictions (DISASTERS.md section 3)

The prediction held in 58 of 60 measured cells; 2 missed; 4 cell(s) UNMEASURED and not counted (D15 arm 1, D15 arm 2, D15 arm 3, D15 arm 4).
- MISS D11 arm 3: predicted EX, got NR (runs: NR NR NR). Oracle: NOT-RESTORED improved=0 still-failing=1 | NOT-RESTORED improved=0 still-failing=1 | NOT-RESTORED improved=0 still-failing=1. Reason: to be written from the tap and the oracle line.
- MISS D12 arm 3: predicted PA, got NR (runs: NR NR NR). Oracle: NOT-RESTORED improved=0 still-failing=2 | NOT-RESTORED improved=0 still-failing=2 | NOT-RESTORED improved=0 still-failing=2. Reason: to be written from the tap and the oracle line.

## Red cells (arm 4 does not win against the better of arms 1 and 2, or is itself partial or did not help)

- D01: tie with the better of arms 1 and 2. Expected in DISASTERS.md.
- D02: tie with the better of arms 1 and 2. Expected in DISASTERS.md.
- D03: tie with the better of arms 1 and 2. Expected in DISASTERS.md.
- D05: partial. Expected in DISASTERS.md.
- D07: loses to the better of arms 1 and 2, arm 4 did not help. Expected in DISASTERS.md.
- D08: tie with the better of arms 1 and 2, arm 4 did not help. Expected in DISASTERS.md.
- D10: tie with the better of arms 1 and 2, arm 4 did not help. Expected in DISASTERS.md.
- D11: tie with the better of arms 1 and 2. Expected in DISASTERS.md.
- D12: partial. Expected in DISASTERS.md.
- D13: tie with the better of arms 1 and 2, arm 4 did not help. Expected in DISASTERS.md.
- D14: tie with the better of arms 1 and 2, arm 4 did not help. Expected in DISASTERS.md.

Arm 4 ties or loses to arm 3 (the adapter added nothing, or less):
D01 T, D02 T, D03 T, D05 T, D07 T, D08 T, D10 T, D13 T, D14 T, D16 T

## Excluded, refused, invalid and not-run runs

- D15 arm 1 rep 1 attempt 1 (replay): NOT-RUN; reason: replay cannot play this disaster: the script needs a real /compact and a real model (compact: / memory oracle): live only.
- D15 arm 2 rep 1 attempt 1 (replay): NOT-RUN; reason: replay cannot play this disaster: the script needs a real /compact and a real model (compact: / memory oracle): live only.
- D15 arm 3 rep 1 attempt 1 (replay): NOT-RUN; reason: replay cannot play this disaster: the script needs a real /compact and a real model (compact: / memory oracle): live only.
- D15 arm 4 rep 1 attempt 1 (replay): NOT-RUN; reason: replay cannot play this disaster: the script needs a real /compact and a real model (compact: / memory oracle): live only.
- D15 arm 1 rep 2 attempt 1 (replay): NOT-RUN; reason: replay cannot play this disaster: the script needs a real /compact and a real model (compact: / memory oracle): live only.
- D15 arm 2 rep 2 attempt 1 (replay): NOT-RUN; reason: replay cannot play this disaster: the script needs a real /compact and a real model (compact: / memory oracle): live only.
- D15 arm 3 rep 2 attempt 1 (replay): NOT-RUN; reason: replay cannot play this disaster: the script needs a real /compact and a real model (compact: / memory oracle): live only.
- D15 arm 4 rep 2 attempt 1 (replay): NOT-RUN; reason: replay cannot play this disaster: the script needs a real /compact and a real model (compact: / memory oracle): live only.
- D15 arm 1 rep 3 attempt 1 (replay): NOT-RUN; reason: replay cannot play this disaster: the script needs a real /compact and a real model (compact: / memory oracle): live only.
- D15 arm 2 rep 3 attempt 1 (replay): NOT-RUN; reason: replay cannot play this disaster: the script needs a real /compact and a real model (compact: / memory oracle): live only.
- D15 arm 3 rep 3 attempt 1 (replay): NOT-RUN; reason: replay cannot play this disaster: the script needs a real /compact and a real model (compact: / memory oracle): live only.
- D15 arm 4 rep 3 attempt 1 (replay): NOT-RUN; reason: replay cannot play this disaster: the script needs a real /compact and a real model (compact: / memory oracle): live only.

Infra-failure attempts that were re-run (counted, all attempts are in the records):
none

GATE-NOT-REACHED runs (counted as MI):
none

## Negative control (PREREG section 8)

- NC1 (D06 arm 4, poisoned): EX (EX EX EX); expected not PV (EX if the engine saves still happen, NR if not); ok
- NC2 (D16 arm 4, poisoned): MI (MI MI MI); expected MI; ok
